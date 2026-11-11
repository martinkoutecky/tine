// Ctrl+K typed the moment the window is up, before the graph is bound. Opening
// the graph publishes its config-derived state: the new meta makes the switcher
// ask again, and then the display epoch moves, which retires every read begun
// before it. The switcher kept the retired, empty answer, so an existing page
// was offered only as "Create page: X" for as long as the switcher stayed open.
// (Where the typing falls relative to the bind depends on the machine; the
// journey records which side it landed on and asserts the same outcome either
// way: the existing page is offered, never a Create row standing in for it.)
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { remote } from "webdriverio";
import { currentPageTitle, switcherPageRows } from "./lib/e2e-navigation.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";

const ROOT = process.cwd();
const APP = process.env.TINE_APP || path.join(ROOT, "target/debug/tine");
const OUT = path.resolve(process.env.E2E_ARTIFACT_DIR || "artifacts/switcher-warm");
fs.mkdirSync(OUT, { recursive: true });
const TMP = fs.mkdtempSync(path.join(OUT, "journey-"));
const GRAPH = path.join(TMP, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(TMP, dir));
const files = {
  "page_A": "alias:: my_important_page\n\n- Canonical body\n",
  "page_B": "- Another page\n",
};
for (const [name, text] of Object.entries(files)) fs.writeFileSync(path.join(GRAPH, `pages/${name}.md`), text);
const env = { ...process.env, TINE_GRAPH: GRAPH, XDG_DATA_HOME: path.join(TMP, "data"), XDG_CONFIG_HOME: path.join(TMP, "config"), XDG_CACHE_HOME: path.join(TMP, "cache"), WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4799);
const log = fs.openSync(path.join(TMP, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || path.join(process.env.CARGO_HOME, "bin/tauri-driver"), ["--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log], detached: true });
const proof = { commit: process.env.E2E_TESTED_COMMIT, binarySha256: createHash("sha256").update(fs.readFileSync(APP)).digest("hex"), steps: [], timeline: [] };
const state = (browser) => browser.execute(() => ({
  searching: Boolean(document.querySelector('#switcher-results [role="status"]')),
  rows: [...document.querySelectorAll(".switcher-row:not(.block-result) .switcher-name")].map((node) => (node.textContent ?? "").trim().normalize("NFC")),
}));
const sourceFiles = () => fs.readdirSync(path.join(GRAPH, "pages")).sort();
const sourceNames = sourceFiles();
let browser;
try {
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 2, connectionRetryTimeout: 30_000, capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } } });
  await ensureMainWindow(browser);
  await browser.$(".app-container").waitForExist({ timeout: 30_000 });

  await browser.keys(["Control", "k"]);
  const input = await browser.$(".switcher-input");
  await input.waitForExist({ timeout: 10_000 });
  // The whole name at once, as a paste: one request for "page_A", not one per
  // keystroke, so a later keystroke cannot re-ask after the bind by itself.
  // `warm_done` is refused until the graph is bound to this window, so a refusal
  // right after the input proves the query was typed before the bind.
  proof.boundWhenTyped = await browser.executeAsync((done) => {
    const field = document.querySelector(".switcher-input");
    field.value = "page_A";
    field.dispatchEvent(new InputEvent("input", { bubbles: true }));
    window.__TAURI_INTERNALS__.invoke("warm_done").then(() => done(true), () => done(false));
  });
  proof.steps.push(`typed page_A at once (graph bound by then: ${proof.boundWhenTyped})`);
  let last;

  // The page is offered once the switcher has answered, and it stays so: no
  // Create row stands in for a page that exists.
  let stable = 0;
  try {
    await browser.waitUntil(async () => {
      last = await state(browser);
      proof.timeline.push({ at: "answered", ...last });
      stable = !last.searching && last.rows.includes("page_A") && !last.rows.includes("Create page: page_A") ? stable + 1 : 0;
      return stable >= 6;
    }, { timeout: 20_000, interval: 250, timeoutMsg: "timed out" });
  } catch {
    throw new Error(`the existing page was not offered; offering ${JSON.stringify(last?.rows)} (searching: ${last?.searching})`);
  }
  proof.steps.push("page_A is offered, no Create row");

  // Activating it opens the existing page: no second file appears.
  await browser.keys(["Enter"]);
  await browser.waitUntil(async () => (await currentPageTitle(browser)) === "page_A", { timeout: 15_000, timeoutMsg: "page_A did not open" });
  if (JSON.stringify(sourceFiles()) !== JSON.stringify(sourceNames)) throw new Error("the pages directory changed (a second file was created?)");
  for (const [name, text] of Object.entries(files)) {
    if (fs.readFileSync(path.join(GRAPH, `pages/${name}.md`), "utf8") !== text) throw new Error(`${name}: browsing changed the source file`);
  }
  proof.steps.push("page_A opened; the pages directory and its bytes are unchanged");
  console.log(`PASS ${JSON.stringify(proof)}`);
} catch (error) {
  proof.error = String(error);
  proof.classification = "ambiguous; inspect captured DOM and driver log";
  if (browser) {
    fs.writeFileSync(path.join(TMP, "failure-dom.html"), await browser.execute(() => document.body.innerHTML).catch(() => ""));
    proof.finalRows = await switcherPageRows(browser).catch(() => undefined);
  }
  throw error;
} finally {
  fs.writeFileSync(path.join(TMP, "proof.json"), JSON.stringify(proof, null, 2));
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  fs.closeSync(log);
}
