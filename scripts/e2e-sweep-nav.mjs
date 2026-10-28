// GH #780/#785: real routed page, native mouse history, palette/shortcut folding
// and durable page bytes. All graph/account data belongs to this artifact run.
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { execFileSync, spawn } from "node:child_process";
import { remote } from "webdriverio";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName, currentPageTitle } from "./lib/e2e-navigation.mjs";
import { accountHomeFixture } from "./lib/e2e-account-home.mjs";
import { x11Tools } from "./lib/e2e-x11.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";

const root = path.resolve(import.meta.dirname, "..");
const out = path.join(root, "artifacts/sweep-nav/native");
fs.mkdirSync(out, { recursive: true });
const run = fs.mkdtempSync(path.join(out, "run-"));
const graph = path.join(run, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(run, dir));
const page = path.join(graph, "pages/Folding.md");
const before = Array.from({ length: 30 }, (_, i) => `- Parent ${i}\n  - Child ${i}\n`).join("");
fs.writeFileSync(page, before);
fs.writeFileSync(path.join(graph, "pages/Start.md"), "- Follow [[Folding]]\n");
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{}\n");
const account = accountHomeFixture(run, run);
const env = { ...account.env, TINE_GRAPH: graph,
  XDG_DATA_HOME: path.join(run, "data"), XDG_CONFIG_HOME: path.join(run, "config"), XDG_CACHE_HOME: path.join(run, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4897);
const log = fs.openSync(path.join(run, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", ["--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log] });
const testedCommit = execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim();
let browser;
let stage = "startup";
const passed = [];
const pass = () => { passed.push(stage); console.log(`PASS: ${stage}`); };
async function palette(label) {
  await browser.keys(["Control", "Shift", "p"]);
  const input = await browser.$(".switcher-input");
  await input.waitForExist({ timeout: 10000 });
  await input.setValue(label);
  await browser.waitUntil(() => browser.execute(wanted => {
    const row = [...document.querySelectorAll(".switcher-row")].find(el => el.textContent.includes(wanted));
    if (!row) return false;
    row.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0 }));
    return true;
  }, label), { timeout: 10000, timeoutMsg: `Missing command ${label}` });
}
try {
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 2, connectionRetryTimeout: 30000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: process.env.TINE_APP || path.join(root, "target/debug/tine") } } });
  await ensureMainWindow(browser);
  await browser.setWindowSize(1200, 1000);
  await browser.$(".app-container").waitForExist({ timeout: 30000 });
  await openPageByName(browser, "Start");
  stage = "#780 follow page link, native mouse back and forward";
  await browser.$(".main-content .page-ref").click();
  await browser.waitUntil(async () => await currentPageTitle(browser) === "Folding", { timeout: 10000 });
  if (process.platform === "linux") {
    const x11 = x11Tools(env);
    const id = x11.windowIds()[0];
    assert(id, "native graph window exists");
    const geometry = x11.geometry(id);
    x11.xdo("mousemove", String(geometry.X + 600), String(geometry.Y + 300));
    x11.xdo("click", "8");
    await browser.waitUntil(async () => await currentPageTitle(browser) === "Start", { timeout: 10000 });
    x11.xdo("click", "9");
    await browser.waitUntil(async () => await currentPageTitle(browser) === "Folding", { timeout: 10000 });
    pass();
    stage = "#780 native side-button drag preserves the page";
    x11.xdo("mousedown", "8");
    x11.xdo("mousemove_relative", "30", "0");
    x11.xdo("mouseup", "8");
    // A subsequent successful UI operation below is the processing barrier.
  }
  stage = "#785 palette collapses 60 blocks and saves only fold properties";
  await palette("Collapse all blocks on current page");
  await waitForFileText(page, text => (text.match(/collapsed:: true/g) ?? []).length === 30);
  const collapsed = fs.readFileSync(page, "utf8");
  assert.equal(collapsed.replace(/^  collapsed:: true\n/gm, ""), before);
  assert.equal(await currentPageTitle(browser), "Folding", "side-button drag did not navigate");
  const bytesAdded = Buffer.byteLength(collapsed) - Buffer.byteLength(before);
  pass();
  stage = "#785 t o expands a fully folded routed page and persists";
  await browser.keys(["t", "o"]);
  await waitForFileText(page, text => !text.includes("collapsed::"));
  assert.equal(fs.readFileSync(page, "utf8"), before);
  await browser.$(".main-content .ls-block .block-content").waitForExist();
  pass();
  stage = "#785 bullet context menu folds and expands one subtree";
  const content = await browser.$(".main-content .ls-block .block-content");
  await content.click({ button: "right" });
  await browser.$('.ctx-item=Collapse all').click();
  await waitForFileText(page, text => (text.match(/collapsed:: true/g) ?? []).length === 1);
  await content.click({ button: "right" });
  await browser.$('.ctx-item=Expand all').click();
  await waitForFileText(page, text => !text.includes("collapsed::"));
  await browser.waitUntil(async () => (await browser.$(".main-content").getText()).includes("Child 0"), {
    timeout: 10000, timeoutMsg: "expanded subtree did not become visible",
  });
  pass();
  await browser.$("h1.page-title").click();
  await browser.executeAsync(done => requestAnimationFrame(() => requestAnimationFrame(() => done())));
  execFileSync("import", ["-window", "root", path.join(run, "folding.png")], { env });
  account.verify();
  fs.writeFileSync(path.join(run, "result.json"), JSON.stringify({ testedCommit, dirtyWorktree: true, passed, bytesAdded, graph }, null, 2));
  console.log(`Evidence: ${run}`);
} catch (error) {
  fs.writeFileSync(path.join(run, "failure.json"), JSON.stringify({ testedCommit, journey: stage,
    expected: "Routed page history and bulk folds preserve the user's intended state and persist fold properties",
    observed: String(error), classification: "ambiguous", passed }, null, 2));
  throw error;
} finally {
  if (browser) await browser.deleteSession().catch(() => {});
  driver.kill();
  fs.closeSync(log);
}
