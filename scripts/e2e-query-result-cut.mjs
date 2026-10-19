// GH #726: Cut / Delete from a query result's block menu act on the RESULT,
// on its own page, and never remove the query host block. Reporter's literal
// page (Queries/TasksUnscheduled, a {{tine-query ...}} host), plus the legacy
// {{query}} macro, the table view and the OG legacy query table.
// OG og-1.0.0 frontend/components/content.cljs hiccup-content resolves the menu
// from the nearest .bullet-container[blockid] and cut-block!s that block.
import fs from "node:fs";
import path from "node:path";
import { execFileSync, spawn } from "node:child_process";
import { remote } from "webdriverio";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";

const root = path.resolve(import.meta.dirname, "..");
const testedCommit = execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim();
const artifacts = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(root, "artifacts/query-result-cut/native"));
fs.mkdirSync(artifacts, { recursive: true });
const run = fs.mkdtempSync(path.join(artifacts, "run-"));
const graph = path.join(run, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(run, dir));

const QUERY_LINE = "{{tine-query @block and task in ('TODO', 'DOING', 'NOW', 'LATER', 'WAITING', 'WAIT', 'STARTED', 'IN-PROGRESS') and not scheduled is not null}}";
const reporterFile = path.join(graph, "pages/Queries___TasksUnscheduled.md");
const tasksFile = path.join(graph, "pages/Tasks.md");
const legacyFile = path.join(graph, "pages/Legacy Query.md");
const tableFile = path.join(graph, "pages/Table Query.md");
const oldTableFile = path.join(graph, "pages/Old Table.md");
// The reporter's file name maps to "Queries/TasksUnscheduled" under the default new-graph name format.
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{:file/name-format :triple-lowbar}\n");
fs.writeFileSync(reporterFile, `- ${QUERY_LINE}\n- TODO process this \n  SCHEDULED: <2026-10-13 Tue .+1m>\n- \n`);
fs.writeFileSync(tasksFile, "- WAITING cut me from the query\n- WAITING delete me from the query\n- LATER legacy macro result\n- NOW table row result\n");
fs.writeFileSync(legacyFile, "- {{query (task LATER)}}\n- after legacy\n");
fs.writeFileSync(tableFile, "- {{tine-query @block and task in ('NOW')}}\n  tine.view:: table\n- after table\n");
fs.writeFileSync(oldTableFile, "- {{query (task NOW)}}\n  query-table:: true\n- after old table\n");

const app = process.env.TINE_APP || path.join(root, "target/debug/tine");
const driverPath = process.env.TAURI_DRIVER || "tauri-driver";
const port = Number(process.env.E2E_DRIVER_PORT || 4796);
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

  stage = "reporter page: right-click a {{tine-query}} result's bullet, Cut block";
  await openPageByName(browser, "Queries/TasksUnscheduled");
  await rightClickResult("cut me from the query", ".bullet-container");
  await chooseFromMenu("Cut block");
  await waitForFileText(tasksFile, (text) => !text.includes("cut me from the query") && text.includes("delete me from the query"));
  assert(read(reporterFile).includes(QUERY_LINE), `query host removed or changed: ${JSON.stringify(read(reporterFile))}`);
  const clipboard = await browser.execute(async () => window.__TAURI_INTERNALS__.invoke("plugin:clipboard-manager|read_text"));
  assert(String(clipboard).includes("cut me from the query"), `clipboard lacks the cut result: ${JSON.stringify(clipboard)}`);
  passed.push(stage); console.log(`PASS ${stage}`);

  stage = "reporter page: right-click a result's text, Delete block";
  await rightClickResult("delete me from the query", ".block-content");
  await chooseFromMenu("Delete block");
  await waitForFileText(tasksFile, (text) => !text.includes("delete me from the query"));
  assert(read(reporterFile).includes(QUERY_LINE), "query host removed by Delete");
  passed.push(stage); console.log(`PASS ${stage}`);

  stage = "{{query}} macro list result: Cut block";
  await openPageByName(browser, "Legacy Query");
  await rightClickResult("legacy macro result", ".bullet-container");
  await chooseFromMenu("Cut block");
  await waitForFileText(tasksFile, (text) => !text.includes("legacy macro result"));
  assert(read(legacyFile) === "- {{query (task LATER)}}\n- after legacy\n", `legacy query page changed: ${JSON.stringify(read(legacyFile))}`);
  passed.push(stage); console.log(`PASS ${stage}`);

  stage = "table view: right-clicking a result row never offers the host's Cut/Delete";
  await openPageByName(browser, "Table Query");
  let cellPoint = null;
  await browser.waitUntil(async () => (cellPoint = await browser.execute(() => {
    const cell = [...document.querySelectorAll(".query-block .sheet-title-cell")].find((el) => el.textContent.includes("table row result"));
    if (!cell) return null;
    const box = cell.getBoundingClientRect();
    return { x: Math.round(box.left + box.width / 2), y: Math.round(box.top + box.height / 2) };
  })) !== null, { timeout: 20_000, interval: 200, timeoutMsg: "table row not rendered" });
  // The row's own cell menu (or the table's sheet menu) answers; never the host's block menu.
  await rightClickAt(cellPoint);
  const tableItems = await menuItems();
  assert(!tableItems.includes("Cut block") && !tableItems.includes("Delete block"), `host block menu offered in table view: ${JSON.stringify(tableItems)}`);
  await closeMenu();
  assert(read(tableFile).includes("{{tine-query @block and task in ('NOW')}}"), "table query host changed");
  passed.push(stage); console.log(`PASS ${stage}`);

  stage = "OG legacy query table: right-clicking a block cell never opens the host's menu";
  await openPageByName(browser, "Old Table");
  let oldTableItems = null;
  await browser.waitUntil(async () => (oldTableItems = await browser.execute(() => {
    const cell = [...document.querySelectorAll(".query-block table tbody td")].find((el) => el.textContent.includes("table row result"));
    if (!cell) return null;
    // Synthetic event: no native WebKit menu can open over the session. Tine's
    // menu renders synchronously from the handler, so read it in the same tick.
    cell.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, button: 2, clientX: 300, clientY: 300 }));
    return [...document.querySelectorAll(".ctx-menu .ctx-item")].map((el) => el.textContent.trim());
  })) !== null, { timeout: 20_000, interval: 200, timeoutMsg: "legacy table cell not rendered" });
  assert(!oldTableItems.includes("Cut block") && !oldTableItems.includes("Delete block"), `host block menu offered in legacy table: ${JSON.stringify(oldTableItems)}`);
  await closeMenu();
  passed.push(stage); console.log(`PASS ${stage}`);

  await browser.saveScreenshot(path.join(run, "pass.png"));
  fs.writeFileSync(path.join(run, "result.json"), JSON.stringify({ outcome: "pass", testedCommit, passed, graph }, null, 2));
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
