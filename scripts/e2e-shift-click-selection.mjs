// GH #924: a real Shift+click from an editor selects complete blocks.
import { spawn, execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { remote } from "webdriverio";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { currentPageTitle, openPageByLink, openPageByName, openJournals } from "./lib/e2e-navigation.mjs";
import { accountHomeFixture, assertAppLease } from "./lib/e2e-account-home.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";

const ROOT = path.resolve(import.meta.dirname, "..");
const OUT = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(ROOT, "target/artifacts/fix-924"));
fs.mkdirSync(OUT, { recursive: true });
const RUN = fs.mkdtempSync(path.join(OUT, "native-"));
const GRAPH = path.join(RUN, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
for (const dir of ["home", "data", "config", "cache", "state"]) fs.mkdirSync(path.join(RUN, dir));
fs.writeFileSync(path.join(GRAPH, "logseq/config.edn"), "{:tine/guide-announced? true}\n");
const outline = "- Block A caret starts here\n- Block B middle\n- Block C middle\n- Block D text after the click\n  collapsed:: true\n  - Last block child\n- [[Shift click]]\n- [[Selection destination]]\n";
const pageFile = path.join(GRAPH, "pages/Shift click.md");
fs.writeFileSync(pageFile, outline);
fs.writeFileSync(path.join(GRAPH, "pages/Selection destination.md"), "- Destination stays visible\n");
const today = new Date();
const date = [today.getFullYear(), String(today.getMonth() + 1).padStart(2, "0"), String(today.getDate()).padStart(2, "0")].join("_");
fs.writeFileSync(path.join(GRAPH, `journals/${date}.md`), outline);
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const revision = execFileSync("git", ["-C", ROOT, "rev-parse", "HEAD"], { encoding: "utf8" }).trim();
const fixture = accountHomeFixture(RUN, RUN);
const env = { ...fixture.env, HOME: path.join(RUN, "home"), TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: path.join(RUN, "data"), XDG_CONFIG_HOME: path.join(RUN, "config"),
  XDG_CACHE_HOME: path.join(RUN, "cache"), XDG_STATE_HOME: path.join(RUN, "state"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4924);
const log = fs.openSync(path.join(RUN, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || path.join(process.env.CARGO_HOME || "", "bin/tauri-driver"),
  ["--port", String(port), "--native-port", String(port + 1), "--native-driver", "/usr/bin/WebKitWebDriver"],
  { env, stdio: ["ignore", log, log], detached: true });
const screenshot = name => execFileSync("import", ["-window", "root", path.join(RUN, `${name}.png`)], { timeout: 10000 });
let browser, stage = "startup", classification = "infrastructure";
const evidence = [];
async function shiftedClick(element, options) {
  // keys(["Shift"]) presses AND releases Shift before returning.
  await browser.performActions([{ type: "key", id: "range-shift", actions: [{ type: "keyDown", value: "\uE008" }] }]);
  try { await element.click(options); }
  finally { await browser.releaseActions(); }
}
try {
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 2, connectionRetryTimeout: 30000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } } });
  await ensureMainWindow(browser);
  await browser.$(".app-container").waitForExist({ timeout: 30000 });
  await assertAppLease(fixture);
  await openPageByName(browser, "Shift click");
  await browser.execute(() => {
    window.__gh924Presses = [];
    document.addEventListener("mousedown", event => window.__gh924Presses.push({
      shift: event.shiftKey, id: event.target.closest?.("[data-block-id]")?.dataset.blockId ?? null,
    }), true);
  });
  for (const surface of ["page", "sidebar", "journal"]) {
    stage = `${surface}: caret in A, Shift+click text in D`;
    classification = "harness";
    const root = surface === "sidebar" ? ".right-sidebar" : ".main-content";
    if (surface === "sidebar") {
      const link = await browser.$(".main-content a.page-ref");
      await shiftedClick(link);
    }
    if (surface === "journal") await openJournals(browser);
    await browser.waitUntil(() => browser.execute(root => [...document.querySelectorAll(`${root} .block-content`)]
      .some(el => el.textContent.trim() === "Block D text after the click"), root), { timeout: 15000 });
    const ids = await browser.execute(root => ["A", "D"].map(letter => [...document.querySelectorAll(`${root} .block-content`)]
      .find(el => el.textContent.startsWith(`Block ${letter} `)).closest("[data-block-id]").dataset.blockId), root);
    const row = id => `${root} [data-block-id="${id}"] > .block-main`;
    await browser.$(`${row(ids[0])} .block-content`).click();
    await browser.$(`${row(ids[0])} textarea.block-editor`).waitForDisplayed({ timeout: 10000 });
    // Interior caret, then a literal shifted pointer click on another block.
    await browser.keys(["Home", "ArrowRight", "ArrowRight"]);
    const destination = await browser.$(`${row(ids[1])} .block-content`);
    const size = await destination.getSize();
    await shiftedClick(destination, { x: Math.round(20 - size.width / 2), y: 0 });
    const observed = await browser.execute(root => ({
      selected: [...document.querySelectorAll(`${root} .block-main.selected`)].map(el => el.querySelector(".block-content")?.textContent.trim()),
      editors: document.querySelectorAll(`${root} textarea.block-editor`).length,
      nativeSelection: document.getSelection()?.toString(),
      presses: window.__gh924Presses,
    }), root);
    evidence.push({ surface, ...observed });
    screenshot(surface);
    assert.deepEqual(observed.presses.at(-1), { shift: true, id: ids[1] }, "the native press must reach D with Shift held");
    classification = "product";
    assert.deepEqual(observed.selected, ["Block A caret starts here", "Block B middle", "Block C middle", "Block D text after the click"]);
    assert.equal(observed.editors, 0, "block selection must leave editing");
    assert.equal(observed.nativeSelection, "", "no partial native text selection remains");
    if (surface === "page") {
      stage = "cut range removes D and its collapsed child, Undo restores both";
      await browser.keys(["Control", "x", "NULL"]);
      await waitForFileText(pageFile, text => text.includes("[[Shift click]]") && !text.includes("Block A") && !text.includes("Block D") && !text.includes("Last block child"));
      await browser.keys(["Control", "z", "NULL"]);
      await waitForFileText(pageFile, text => text.includes("Block D text after the click") && text.includes("Last block child"));
      await browser.waitUntil(() => browser.execute(() => [...document.querySelectorAll(".main-content .block-content")]
        .some(el => el.textContent.trim() === "Block D text after the click")), { timeout: 10000 });
    }
  }
  const routeFailures = [];
  for (const entry of ["link", "palette", "back", "forward", "keyboard"]) {
    stage = `Ifods: select a routed block, navigate by ${entry}, Delete, reopen the source`;
    classification = "harness";
    if (entry === "back") await openPageByName(browser, "Selection destination");
    await openPageByName(browser, "Shift click");
    if (entry === "forward") {
      await openPageByLink(browser, "Selection destination");
      await browser.keys(["Alt", "ArrowLeft", "NULL"]);
      await browser.waitUntil(async () => (await currentPageTitle(browser)) === "Shift click");
    }
    const sourceBlock = await browser.$(".main-content .block-content");
    await sourceBlock.click();
    await browser.$(".main-content textarea.block-editor").waitForDisplayed();
    await browser.keys(["Escape"]);
    await browser.$(".main-content .block-main.selected").waitForExist();
    const beforeDeparture = fs.readFileSync(pageFile, "utf8");
    if (entry === "link") await openPageByLink(browser, "Selection destination");
    if (entry === "palette") await openPageByName(browser, "Selection destination");
    if (entry === "back" || entry === "forward") {
      await browser.keys(["Alt", entry === "back" ? "ArrowLeft" : "ArrowRight", "NULL"]);
      await browser.waitUntil(async () => (await currentPageTitle(browser)) === "Selection destination");
    }
    if (entry === "keyboard") {
      await browser.keys(["g", "j"]);
      await browser.$(".main-content .journal-title").waitForExist();
    }
    await browser.keys(["Delete"]);
    await openPageByName(browser, "Shift click");
    await browser.waitUntil(() => browser.execute(() => !document.querySelector(".main-content .page-loading")));
    const sourceStillVisible = await browser.execute(() => [...document.querySelectorAll(".main-content .block-content")]
      .some(el => el.textContent.trim() === "Block A caret starts here"));
    classification = "product";
    let undoRestored = null;
    if (!sourceStillVisible) {
      await waitForFileText(pageFile, text => !text.includes("Block A caret starts here"));
      await browser.keys(["Control", "z", "NULL"]);
      await waitForFileText(pageFile, text => text === beforeDeparture);
      undoRestored = true;
    }
    const fileUnchanged = sourceStillVisible && fs.readFileSync(pageFile, "utf8") === beforeDeparture;
    evidence.push({ entry, sourceStillVisible, fileUnchanged, undoRestored });
    screenshot(`route-selection-${entry}`);
    if (!fileUnchanged) routeFailures.push(entry);
  }
  assert.deepEqual(routeFailures, [], "Delete after every navigation method must leave the source file byte-identical");
  fixture.verify();
  fs.writeFileSync(path.join(RUN, "receipt.json"), JSON.stringify({ outcome: "PASS", revision, workingDiff: true, app: APP, evidence }, null, 2));
  console.log(`PASS ${RUN}`);
} catch (error) {
  try { screenshot("failure"); } catch {}
  fs.writeFileSync(path.join(RUN, "failure.json"), JSON.stringify({ revision, app: APP, journey: stage,
    expected: "A through D selected as whole blocks, cut/Undo preserves children; Delete after route departure leaves the source file unchanged",
    observed: error.stack, classification, evidence }, null, 2));
  console.error(`FAIL ${RUN}`);
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  fs.closeSync(log);
  fixture.close?.();
}
