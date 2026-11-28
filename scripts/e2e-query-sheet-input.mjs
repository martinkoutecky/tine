// Linux real-WebKit journey for GH #619 ("query builder is unusable in Beta"): with the query sheet open,
// REAL pointer presses and REAL key presses stay inside the sheet. Before the fix a press on the sheet was
// taken by the block behind it (it entered edit mode, tore the sheet down and the control never acted), and
// the sheet opened at the viewport's top-left. It then edits a query table's title cell: Delete at its end
// must not merge the next block of the result's file, which the table does not draw, into the result.
// Last, typing in a table's `Ready` text column on a row holding `ready:: false` (another spelling of the
// same property to Logseq) is refused visibly and leaves that line on disk, and a refused `Done` checkbox
// beside `done:: true` keeps showing the saved value; the file is compared after the refused gestures and the next save.
//
// Every pointer interaction here goes through the W3C Actions API (pointerdown/mousedown/mouseup/click as a
// pointing device produces them), never `element.click()` from script: a synthetic click skips the
// press events the bug lived in.
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { BLOCK_EDITOR } from "./lib/e2e-block-editor.mjs";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const TD = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin", "tauri-driver") : "tauri-driver");
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4520);
const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4521);
const TMP_ROOT = path.resolve(process.env.E2E_TMP_ROOT || process.env.TMPDIR || "/tmp");
fs.mkdirSync(TMP_ROOT, { recursive: true });
const TMP = fs.mkdtempSync(path.join(TMP_ROOT, "tine-og-query-sheet-input-e2e-"));
const GRAPH = `${TMP}/graph`;
const ARTIFACTS = process.env.E2E_ARTIFACT_DIR || `${TMP}/artifacts`;
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(`${GRAPH}/${dir}`, { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(`${TMP}/xdg/${dir}`, { recursive: true });
fs.mkdirSync(ARTIFACTS, { recursive: true });
fs.writeFileSync(`${GRAPH}/logseq/config.edn`, "{}\n");
const now = new Date();
const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(`${GRAPH}/journals/${journal}.md`, "- Open [[Queries]]\n");
const TASKS_FILE = `${GRAPH}/pages/Tasks.md`;
const TASKS = "- TODO alpha task\n- TODO beta task\n- DONE finished task\n";
fs.writeFileSync(TASKS_FILE, TASKS);
// A query drawn as a table: its rows are the TODO tasks; "DONE finished task" follows "beta task" in its file but is not drawn.
fs.writeFileSync(`${GRAPH}/pages/Task Table.md`, "- {{query (task TODO)}}\n  tine.view:: table\n");
for (let i = 0; i < 12; i += 1) {
  fs.writeFileSync(`${GRAPH}/pages/Book ${i}.md`, `kind-${i}:: v${i}\ntype:: book\n\n- A book page ${i}\n`);
}
// A children table whose `ready` checkbox and `Ready` text columns are one property to Logseq, as are its
// `Done` checkbox column and row two's `done::` line.
const READY_FILE = `${GRAPH}/pages/Ready Table.md`;
const READY = "- Rows\n  tine.view:: table\n  tine.fields:: ready=checkbox;Ready=text;Done=checkbox\n"
  + "\t- Row one\n\t  ready:: false\n\t- Row two\n\t  done:: true\n\t- Row three\n";
fs.writeFileSync(READY_FILE, READY);
const QUERIES_FILE = `${GRAPH}/pages/Queries.md`;
const BELOW = "- plain block below the query";
fs.writeFileSync(QUERIES_FILE, `- {{query (task TODO)}}\n${BELOW}\n`);
const disk = () => fs.readFileSync(QUERIES_FILE, "utf8");

const env = {
  ...process.env,
  TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: `${TMP}/xdg/data`,
  XDG_CONFIG_HOME: `${TMP}/xdg/config`,
  XDG_CACHE_HOME: `${TMP}/xdg/cache`,
  WEBKIT_DISABLE_DMABUF_RENDERER: "1",
  WEBKIT_DISABLE_COMPOSITING_MODE: "1",
  LIBGL_ALWAYS_SOFTWARE: "1",
  GDK_BACKEND: "x11",
};

const log = fs.openSync(`${TMP}/tauri-driver.log`, "w");
const td = spawn(TD, ["--port", String(DRIVER_PORT), "--native-port", String(NATIVE_PORT), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], {
  env, stdio: ["ignore", log, log], detached: true,
});
await sleep(2500);
let browser;
let failed = null;
try {
  browser = await remote({
    hostname: "127.0.0.1", port: DRIVER_PORT, path: "/", logLevel: "error",
    connectionRetryCount: 1, connectionRetryTimeout: 60_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } },
  });
  await browser.setWindowRect(0, 0, 1280, 900).catch(() => {});
  await browser.$(".ls-block, .page-title").waitForExist({ timeout: 20_000 });
  await openPageByName(browser, "Queries");
  await browser.execute(() => {
    for (const close of document.querySelectorAll(".toast-sticky .toast-close")) if (close instanceof HTMLElement) close.click();
  });

  /** A real pointer press-and-release on the element, as a mouse produces it. */
  const press = async (selector) => {
    const el = await browser.$(selector);
    await el.waitForExist({ timeout: 10_000 });
    await el.scrollIntoView({ block: "nearest", inline: "nearest" });
    await browser.action("pointer", { parameters: { pointerType: "mouse" } })
      .move({ origin: el, x: 0, y: 0 }).down({ button: 0 }).up({ button: 0 }).perform();
    await sleep(350);
  };
  const state = () => browser.execute(editor => ({
    sheet: !!document.querySelector(".qs-sheet"),
    chooser: document.querySelector(".qs-sheet .qs-add")?.getAttribute("aria-expanded") === "true",
    editing: !!document.querySelector(editor),
    rows: document.querySelectorAll(".qs-sheet .qs-row").length,
  }), BLOCK_EDITOR);
  const expectState = async (what, want) => {
    const got = await state();
    for (const [key, value] of Object.entries(want)) {
      if (got[key] !== value) throw new Error(`${what}: expected ${key}=${value}, got ${JSON.stringify(got)}`);
    }
  };

  // Wait for the resting query to answer so the page has settled.
  await browser.waitUntil(() => browser.execute(() => /alpha task/.test(document.querySelector(".page-blocks .query-block")?.textContent ?? "")),
    { timeout: 20_000, interval: 150, timeoutMsg: "the query never answered" });

  // 1. The query menu's Edit action opens its inline sheet, and no block editor opens.
  await press(".page-blocks .query-block .qc-actions > summary");
  await press("//*[@class='qc-menu']//button[normalize-space()='Edit']");
  await browser.$(".qs-sheet").waitForExist({ timeout: 10_000 });
  await expectState("after opening the sheet", { sheet: true, editing: false, chooser: false });
  const place = await browser.execute(() => {
    const sheet = document.querySelector(".qs-sheet").getBoundingClientRect();
    const query = document.querySelector(".page-blocks .query-block");
    const header = query.querySelector(".query-header").getBoundingClientRect();
    const bounds = query.getBoundingClientRect();
    return { owned: query.contains(document.querySelector(".qc-panel .qs-sheet")),
      sheet: [sheet.left, sheet.top, sheet.right, sheet.bottom],
      headerBottom: header.bottom, query: [bounds.left, bounds.top, bounds.right, bounds.bottom] };
  });
  const [sl, st, sr, sb] = place.sheet;
  const [ql, , qr, qb] = place.query;
  if (!(place.owned && sl >= ql && sr <= qr && st >= place.headerBottom && sb <= qb)) {
    throw new Error(`the sheet is not inline below its query header: ${JSON.stringify(place)}`);
  }

  // 2. A real press on "+ Add condition" opens the field chooser; the block behind does not take the press.
  await press(".qs-sheet .qs-add");
  await expectState("after pressing + Add condition", { sheet: true, chooser: true, editing: false });
  // The chooser's rows are readable and fully on screen (they were squeezed to a few px).
  const layout = await browser.execute(() => {
    const rows = [...document.querySelectorAll(".qs-vocab-row")].map((row) => row.getBoundingClientRect().height);
    const menu = document.querySelector(".qs-menu")?.getBoundingClientRect();
    return { rows, menu: menu && [menu.left, menu.top, menu.right, menu.bottom], view: [innerWidth, innerHeight] };
  });
  if (layout.rows.length === 0) throw new Error(`the field chooser listed no rows: ${JSON.stringify(layout)}`);
  if (layout.rows.some((height) => height < 30)) throw new Error(`chooser rows were squeezed: ${JSON.stringify(layout.rows)}`);
  const [ml, mt, mr, mb] = layout.menu;
  if (ml < 0 || mt < 0 || mr > layout.view[0] || mb > layout.view[1]) throw new Error(`the chooser runs off screen: ${JSON.stringify(layout)}`);

  // 3. Keys typed with the chooser open reach its filter, not the block behind.
  await press(".qs-sheet .qs-menu-filter");
  await browser.keys(["t", "y", "p", "e"]);
  await sleep(300);
  const typed = await browser.execute(() => document.querySelector(".qs-sheet .qs-menu-filter")?.value ?? null);
  if (typed !== "type") throw new Error(`keystrokes did not reach the chooser filter (it holds ${JSON.stringify(typed)})`);
  await expectState("after typing in the filter", { sheet: true, chooser: true, editing: false });

  // 4. A real press on a chooser row adds that condition; the sheet stays open and the block stays unedited.
  await press(".qs-vocab-option");
  // The typed field is a property: its operator list opens next, and "is set" (no value to type) adds the condition.
  await browser.$(".qs-value-editor").waitForExist({ timeout: 10_000, timeoutMsg: "pressing a chooser row did not open its operator list" });
  await press("//*[@role='option'][normalize-space()='is set']");
  await browser.waitUntil(async () => (await state()).rows >= 2, { timeout: 10_000, interval: 150, timeoutMsg: "pressing a chooser row added no condition" });
  await expectState("after choosing a field", { sheet: true, editing: false });
  const afterRow = disk();

  // 5. Escape peels the chooser, then the sheet; the block behind is never edited and the other block's bytes are intact.
  await browser.keys(["Escape"]);
  await sleep(300);
  await expectState("after the first Escape", { editing: false });
  if ((await state()).chooser) {
    await browser.keys(["Escape"]);
    await sleep(300);
  }
  await browser.keys(["Escape"]);
  await browser.waitUntil(() => browser.execute(() => !document.querySelector(".qs-sheet")), { timeout: 10_000, interval: 100, timeoutMsg: "Escape did not close the sheet" });
  await expectState("after closing the sheet", { editing: false });
  if (!disk().includes(BELOW)) throw new Error(`the block below the query changed:\n${disk()}`);
  if (!afterRow.includes(BELOW)) throw new Error(`the block below changed while the sheet was open:\n${afterRow}`);

  // 6. Delete at the end of a query table's title cell edits only that result: the block after it in
  //    its file, which the table does not draw, is not merged into it. Typing afterwards proves the
  //    editor kept the caret and that the save after the key has reached the disk.
  await openPageByName(browser, "Task Table");
  const title = await browser.$("//*[contains(@class,'sheet-table')]//*[contains(@class,'sheet-cell') and contains(., 'beta task')]");
  await title.waitForExist({ timeout: 20_000, timeoutMsg: "the query table never drew its beta task row" });
  await title.scrollIntoView({ block: "nearest", inline: "nearest" });
  // A press on the title text opens the result's page; select the cell by its padding, then Enter edits it.
  const { width } = await title.getSize();
  await browser.action("pointer", { parameters: { pointerType: "mouse" } })
    .move({ origin: title, x: Math.floor(width / 2) - 3, y: 0 }).down({ button: 0 }).up({ button: 0 }).perform();
  await browser.$(".sheet-table .sheet-cell-selected").waitForExist({ timeout: 10_000, timeoutMsg: "a press on the title cell's padding selected no cell" });
  await browser.keys(["Enter"]);
  const cellEditor = await browser.$(`.sheet-table ${BLOCK_EDITOR}`);
  await cellEditor.waitForExist({ timeout: 10_000, timeoutMsg: "Enter on the selected title cell opened no editor" });
  await browser.keys(["End"]);
  await browser.keys(["Delete"]);
  await browser.keys(["x"]);
  const tasks = await waitForFileText(TASKS_FILE, (body) => body.includes("beta taskx"));
  const want = TASKS.replace("beta task", "beta taskx");
  if (tasks !== want) throw new Error(`Delete at the end of the title cell changed another block:\n${tasks}`);
  if (!(await browser.$(`.sheet-table ${BLOCK_EDITOR}`).isExisting())) throw new Error("the title cell's editor closed on Delete");
  await browser.keys(["Escape"]);
  await sleep(1_000);

  // 7. Typing in the `Ready` column of a row holding `ready:: false` is refused with a notice naming
  //    `ready`, and a press on row two's `Done` checkbox beside `done:: true` is refused with a notice
  //    naming `done` while the box stays unchecked. A refused gesture saves nothing, so the page's next save
  //    is the first moment the disk could show it: row three's `ready` checkbox makes that save without
  //    touching rows one or two, and the file must then hold exactly that one line more. Row one's own
  //    `ready` checkbox is compared the same way.
  await openPageByName(browser, "Ready Table");
  const column = (field) => browser.execute((wanted) => {
    const headers = [...document.querySelectorAll(".sheet-table [data-sheet-field-header]")];
    return headers.findIndex((h) => h.getAttribute("data-sheet-field") === wanted) + 1;
  }, field);
  await browser.$(".sheet-table [data-sheet-field-header]").waitForExist({ timeout: 20_000, timeoutMsg: "the Ready Table never drew its columns" });
  const readyCol = await column("prop:Ready");
  const checkCol = await column("prop:ready");
  const doneCol = await column("prop:Done");
  if (!readyCol || !checkCol || !doneCol) throw new Error("the Ready Table lacks its ready, Ready or Done column");
  const checkbox = (row, col) => `.sheet-table .sheet-cell[data-row="${row}"][data-col="${col}"] input[type="checkbox"]`;
  const notice = (other, what) => browser.waitUntil(() => browser.execute((name) => [...document.querySelectorAll(".toast-msg")]
    .some((el) => (el.textContent ?? "").includes(`already has “${name}”, another spelling of this property`)), other),
  { timeout: 10_000, interval: 150, timeoutMsg: `${what} showed no refusal` });
  const readyCell = await browser.$(`.sheet-table .sheet-cell[data-row="0"][data-col="${readyCol}"]`);
  await readyCell.scrollIntoView({ block: "nearest", inline: "nearest" });
  await browser.action("pointer", { parameters: { pointerType: "mouse" } })
    .move({ origin: readyCell, x: 0, y: 0 }).down({ button: 0 }).up({ button: 0 }).pause(40).down({ button: 0 }).up({ button: 0 }).perform();
  const propInput = await browser.$(".sheet-table input.sheet-prop-input");
  await propInput.waitForExist({ timeout: 10_000, timeoutMsg: "a double press on the Ready cell opened no input" });
  await browser.keys(["y", "e", "s", "Enter"]);
  await notice("ready", "typing in Ready beside ready:: false");
  await browser.keys(["Escape"]);
  await press(checkbox(1, doneCol));
  await notice("done", "a press on Done beside done:: true");
  if (await browser.$(checkbox(1, doneCol)).isSelected()) throw new Error("a refused Done checkbox shows checked; the row holds no Done line");
  await press(checkbox(2, checkCol));
  const afterRefusals = READY.replace("\t- Row three\n", "\t- Row three\n\t  ready:: true\n");
  const barrier = await waitForFileText(READY_FILE, (body) => body.includes("Row three\n\t  ready:: true"));
  if (barrier !== afterRefusals) throw new Error(`a refused Ready cell or Done checkbox changed the file:\n${barrier}`);
  await press(checkbox(0, checkCol));
  const ready = await waitForFileText(READY_FILE, (body) => body.includes("Row one\n\t  ready:: true"));
  if (ready !== afterRefusals.replace("ready:: false", "ready:: true")) throw new Error(`row one's ready checkbox changed more than its line:\n${ready}`);
} catch (error) {
  failed = error;
  if (browser) {
    const snapshot = await browser.execute(() => ({ text: document.body.innerText })).catch(() => null);
    fs.writeFileSync(`${ARTIFACTS}/failure-state.json`, `${JSON.stringify(snapshot, null, 2)}\n`);
    try { await browser.saveScreenshot(`${ARTIFACTS}/failure.png`); } catch {}
  }
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-td.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
if (failed) {
  console.error(`FAIL query sheet input: ${failed.message}\nartifacts: ${ARTIFACTS}`);
  process.exit(1);
}
console.log("PASS query sheet input journey");
fs.rmSync(TMP, { recursive: true, force: true });
