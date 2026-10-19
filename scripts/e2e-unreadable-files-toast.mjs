// GH #727 (discussion): when a page file cannot be read, the red startup toast
// names the file (graph-relative path) instead of pointing at the opt-in debug
// log. Literal user path: a graph with one page file holding invalid UTF-8 (the
// same fixture the backend's failed_reads tests use), launch, read the toast.
import fs from "node:fs";
import path from "node:path";
import { execFileSync, spawn } from "node:child_process";
import { remote } from "webdriverio";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";

const root = path.resolve(import.meta.dirname, "..");
const testedCommit = execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim();
const artifacts = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(root, "artifacts/unreadable-files-toast/native"));
fs.mkdirSync(artifacts, { recursive: true });
const run = fs.mkdtempSync(path.join(artifacts, "run-"));
const graph = path.join(run, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(run, dir));

fs.writeFileSync(path.join(graph, "pages/Good.md"), "- a readable page\n");
fs.writeFileSync(path.join(graph, "pages/Bad.md"), Buffer.from("title:: \xff\n- bad\n", "latin1"));

const app = process.env.TINE_APP || path.join(root, "target/debug/tine");
const driverPath = process.env.TAURI_DRIVER || "tauri-driver";
const port = Number(process.env.E2E_DRIVER_PORT || 4798);
const env = { ...process.env, TINE_GRAPH: graph, XDG_DATA_HOME: path.join(run, "data"), XDG_CONFIG_HOME: path.join(run, "config"), XDG_CACHE_HOME: path.join(run, "cache"), WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const log = fs.openSync(path.join(run, "driver.log"), "w");
const driver = spawn(driverPath, ["--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log] });
const passed = [];
let browser;
let stage = "startup";
const assert = (ok, message) => { if (!ok) throw new Error(message); };
const read = (file) => fs.readFileSync(file, "utf8");

/** Real (WebDriver) right click on the LIVE query result row whose own text
 *  contains `text`, at `part` (bullet or text). Rows re-mount when their page
 *  loads, so locate and click by coordinates in one step, then wait for a menu. */
async function rightClickResult(text, part) {
  let point = null;
  await browser.waitUntil(async () => (point = await browser.execute((text, part) => {
    const row = [...document.querySelectorAll(".query-block .ls-block:not(.ref-block)")]
      .find((el) => el.querySelector(":scope > .block-main .block-content")?.textContent?.includes(text));
    const target = row?.querySelector(`:scope > .block-main ${part}`);
    if (!target) return null;
    target.scrollIntoView({ block: "center" });
    const box = target.getBoundingClientRect();
    return { x: Math.round(box.left + Math.min(box.width / 2, 12)), y: Math.round(box.top + box.height / 2) };
  }, text, part)) !== null, { timeout: 20_000, interval: 200, timeoutMsg: `live query result ${JSON.stringify(text)} not rendered` });
  await rightClickAt(point);
}

async function rightClickAt(point) {
  await browser.performActions([{ type: "pointer", id: "mouse", parameters: { pointerType: "mouse" }, actions: [
    { type: "pointerMove", duration: 0, origin: "viewport", x: point.x, y: point.y },
    { type: "pointerDown", button: 2 }, { type: "pointerUp", button: 2 },
  ] }]);
  await browser.releaseActions();
  await browser.$(".ctx-menu").waitForExist({ timeout: 10_000 });
}

async function menuItems() {
  return browser.execute(() => [...document.querySelectorAll(".ctx-menu .ctx-item")].map((el) => el.textContent.trim()));
}

async function chooseFromMenu(label) {
  const item = await browser.$(`//div[contains(@class,"ctx-item") and normalize-space(.)="${label}"]`);
  await item.waitForDisplayed({ timeout: 10_000 });
  await item.click();
}

async function closeMenu() {
  await browser.keys(["Escape"]);
  await browser.waitUntil(() => browser.execute(() => !document.querySelector(".ctx-menu")), { timeout: 5_000 }).catch(() => {});
}

try {
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 2, connectionRetryTimeout: 30_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: app } } });
  await ensureMainWindow(browser);
  await browser.setWindowSize(1100, 900);
  await browser.$(".app-container").waitForExist({ timeout: 30_000 });

  stage = "the unreadable-files toast names pages/Bad.md and does not mention the debug log";
  let toast = null;
  await browser.waitUntil(async () => (toast = await browser.execute(() =>
    [...document.querySelectorAll(".toast.toast-error")].map((node) => (node.textContent ?? "").replace(/×$/, "").trim())
      .find((text) => text.includes("couldn't be read")) ?? null)) !== null,
  { timeout: 30_000, interval: 200, timeoutMsg: "no 'couldn't be read' error toast appeared" });
  assert(toast.includes("pages/Bad.md"), `toast does not name the file: ${JSON.stringify(toast)}`);
  assert(!/debug log/i.test(toast), `toast still points at the debug log: ${JSON.stringify(toast)}`);
  assert(!toast.includes(graph), `toast shows an absolute host path: ${JSON.stringify(toast)}`);
  passed.push(stage); console.log(`PASS ${stage}: ${toast}`);

  await browser.saveScreenshot(path.join(run, "pass.png"));
  fs.writeFileSync(path.join(run, "result.json"), JSON.stringify({ outcome: "pass", testedCommit, passed, graph, toast }, null, 2));
} catch (error) {
  if (browser) {
    try { fs.writeFileSync(path.join(run, "failure-dom.html"), await browser.execute(() => document.documentElement.outerHTML)); } catch {}
    try { await browser.saveScreenshot(path.join(run, "failure.png")); } catch {}
  }
  fs.writeFileSync(path.join(run, "result.json"), JSON.stringify({ outcome: "fail", testedCommit, classification: "ambiguous", expected: stage, observed: String(error), passed, graph }, null, 2));
  console.error(`FAIL ${stage}: ${error}`);
  process.exitCode = 1;
} finally {
  if (browser) await browser.deleteSession().catch(() => {});
  driver.kill();
  fs.closeSync(log);
}
