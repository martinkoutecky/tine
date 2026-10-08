// GH #623 comment 14: literal native Shift+click on rendered references in
// the journals feed, a named page, the right sidebar and a split pane.
import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { remote } from "webdriverio";
import { openPageByName, openJournals, switcherPageRows } from "./lib/e2e-navigation.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/debug/tine");
const DRIVER = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin/tauri-driver") : "tauri-driver");
const TMP = fs.mkdtempSync("/tmp/tine-gh623-links-");
const GRAPH = path.join(TMP, "graph");
const ARTIFACT = process.env.E2E_ARTIFACT_DIR || path.join(TMP, "artifacts");
const port = Number(process.env.E2E_DRIVER_PORT || 4692);
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(TMP, dir));
fs.mkdirSync(ARTIFACT, { recursive: true });
const pages = {
  "Source": "- Open [[Main target]] and [[Split source]]\n",
  "Journal target": "- From the sidebar open [[Sidebar target]]\n",
  "Main target": "- Main target body\n",
  "Sidebar target": "- Sidebar target body\n",
  "Split source": "- Open [[Split target]]\n",
  "Split target": "- Split target body\n",
  "Canonical page owning these much shorter aliases": "alias:: Short alpha，Short beta, Short gamma\n- Owner body\n",
  "ASCII alias owner with a long canonical name": "alias:: ASCII one, ASCII two\r\n- ASCII owner body\r\n",
  "Fullwidth alias owner with a long canonical name": "alias:: Wide one，Wide two\n- Fullwidth owner body\n",
  "Alias mentions": "- [[Short alpha]] [[Short beta]] [[Short gamma]]\n",
};
for (const [name, text] of Object.entries(pages)) fs.writeFileSync(path.join(GRAPH, "pages", `${name}.md`), text);
const now = new Date();
const date = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(path.join(GRAPH, "journals", `${date}.md`), "- Open [[Journal target]]\n");
const env = {
  ...process.env, TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: path.join(TMP, "data"), XDG_CONFIG_HOME: path.join(TMP, "config"), XDG_CACHE_HOME: path.join(TMP, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11",
};
const log = fs.openSync(path.join(ARTIFACT, "driver.log"), "w");
const driver = spawn(DRIVER, ["--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log], detached: true });
let browser;
const proof = { app: APP, graph: GRAPH, steps: [] };

async function modifiedClick(selector, modifier) {
  await browser.$(selector).waitForDisplayed({ timeout: 15_000 });
  const point = await browser.execute((query) => {
    const element = document.querySelector(query);
    element.scrollIntoView({ block: "center" });
    // A wrapped inline's bounding-box center may be blank space between its
    // fragments; click within one actual fragment and verify the hit target.
    const rect = element.getClientRects()[0];
    const x = Math.round(rect.x + rect.width / 2), y = Math.round(rect.y + rect.height / 2);
    if (!element.contains(document.elementFromPoint(x, y))) throw new Error(`Link is occluded: ${query}`);
    return { x, y };
  }, selector);
  await browser.performActions([
    { type: "key", id: "keyboard", actions: [{ type: "keyDown", value: modifier }, { type: "pause", duration: 0 }, { type: "pause", duration: 0 }, { type: "keyUp", value: modifier }] },
    { type: "pointer", id: "mouse", parameters: { pointerType: "mouse" }, actions: [{ type: "pointerMove", origin: "viewport", ...point }, { type: "pointerDown", button: 0 }, { type: "pointerUp", button: 0 }, { type: "pause", duration: 0 }] },
  ]);
  await browser.releaseActions();
}

// GH #623 comment 16: the reporter saw the alias listed as its OWN result
// beside the owner (the alias text is referenced on another page) and that
// row opened a page named after the alias. The page rows offered for an alias
// query must be the owner, never a row called by the alias.
async function expectAliasRowsAreTheOwner(owner, alias) {
  await openJournals(browser);
  await browser.keys(["Control", "k"]);
  const input = await browser.$(".switcher-input");
  await input.waitForExist({ timeout: 8_000 });
  await input.setValue(alias);
  let rows = [];
  await browser.waitUntil(async () => {
    const searching = await browser.execute(() => Boolean(document.querySelector('#switcher-results [role="status"]')));
    rows = await switcherPageRows(browser);
    return !searching && rows.length > 0;
  }, { timeout: 8_000, interval: 150, timeoutMsg: `no page row offered for ${alias}` });
  const own = rows.filter((name) => name.toLowerCase() === alias.normalize("NFC").toLowerCase());
  if (own.length || rows[0] !== owner.normalize("NFC")) {
    throw new Error(`alias ${JSON.stringify(alias)} offered ${JSON.stringify(rows)}; expected ${JSON.stringify(owner)} first and no row named by the alias`);
  }
  await browser.keys(["Escape"]);
  await browser.waitUntil(() => browser.execute(() => !document.querySelector(".switcher-input")), { timeout: 5_000 });
}

async function expectSidebar(name, body) {
  await browser.waitUntil(() => browser.execute((wanted, text) => {
    const item = [...document.querySelectorAll(".rs-item")].find((node) => node.querySelector(".rs-item-title")?.textContent?.trim() === wanted);
    return !!item && item.textContent.includes(text);
  }, name, body), { timeout: 15_000, timeoutMsg: `Shift+click did not render ${name} in the sidebar` });
}

try {
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 2, connectionRetryTimeout: 30_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } } });
  await browser.$(".app-container").waitForExist({ timeout: 30_000 });
  for (const [owner, aliases, body] of [
    ["Canonical page owning these much shorter aliases", ["Short alpha", "Short beta", "Short gamma"], "Owner body"],
    ["ASCII alias owner with a long canonical name", ["ASCII one", "ASCII two"], "ASCII owner body"],
    ["Fullwidth alias owner with a long canonical name", ["Wide one", "Wide two"], "Fullwidth owner body"],
  ]) {
    for (const alias of aliases) {
      await expectAliasRowsAreTheOwner(owner, alias);
      await openJournals(browser);
      await openPageByName(browser, owner, { query: alias, timeout: 8_000 });
      const visible = await browser.$(".main-content .page-blocks").getText();
      if (!visible.includes(body)) throw new Error(`${alias} opened a title without its owner's body`);
      if (fs.existsSync(path.join(GRAPH, "pages", `${alias}.md`))) throw new Error(`${alias} navigation created a file`);
      proof.steps.push(`${alias} Ctrl+K activation opens canonical owner`);
    }
  }
  await openPageByName(browser, "Alias mentions");
  await modifiedClick('.main-content .page-blocks a.page-ref', "\uE008");
  await expectSidebar("Canonical page owning these much shorter aliases", "Owner body");
  proof.steps.push("rendered alias Shift+click resolves the owner in the sidebar");
  await openJournals(browser);
  await modifiedClick('.main-content a.page-ref', "\uE008");
  await expectSidebar("Journal target", "From the sidebar");
  proof.steps.push("journals Shift+click renders target in sidebar");
  await modifiedClick('.right-sidebar a.page-ref', "\uE008");
  await expectSidebar("Sidebar target", "Sidebar target body");
  proof.steps.push("sidebar Shift+click renders another target in sidebar");
  await openPageByName(browser, "Source");
  await modifiedClick('.main-content a.page-ref', "\uE008");
  await expectSidebar("Main target", "Main target body");
  const title = await browser.$(".main-content h1.page-title").getText();
  if (title.trim() !== "Source") throw new Error(`Shift+click changed source page to ${title}`);
  proof.steps.push("named page Shift+click preserves source and opens sidebar target");
  await modifiedClick('.main-content a.page-ref:last-child', "\uE00A");
  await browser.waitUntil(() => browser.execute(() => [...document.querySelectorAll('[data-pane-id] h1.page-title')].some((node) => node.textContent.trim() === "Split source")), { timeout: 15_000, timeoutMsg: "Alt+click did not open the split source" });
  const splitPane = await browser.execute(() => [...document.querySelectorAll('[data-pane-id] h1.page-title')].find((node) => node.textContent.trim() === "Split source")?.closest('[data-pane-id]')?.getAttribute('data-pane-id'));
  await modifiedClick(`[data-pane-id="${splitPane}"] a.page-ref`, "\uE008");
  await expectSidebar("Split target", "Split target body");
  proof.steps.push("split pane Shift+click renders target in sidebar");
  await browser.saveScreenshot(path.join(ARTIFACT, "shift-click.png"));
  for (const [name, text] of Object.entries(pages)) {
    if (fs.readFileSync(path.join(GRAPH, "pages", `${name}.md`), "utf8") !== text) throw new Error(`Navigation changed ${name}'s file`);
  }
  proof.steps.push("navigation leaves fixture page bytes unchanged");
  console.log(JSON.stringify(proof));
} catch (error) {
  proof.error = String(error);
  if (browser) {
    await browser.saveScreenshot(path.join(ARTIFACT, "failure.png")).catch(() => {});
    fs.writeFileSync(path.join(ARTIFACT, "failure-dom.html"), await browser.execute(() => document.body.innerHTML).catch(() => ""));
  }
  throw error;
} finally {
  fs.writeFileSync(path.join(ARTIFACT, "proof.json"), JSON.stringify(proof, null, 2));
  await browser?.deleteSession().catch(() => {});
  try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
