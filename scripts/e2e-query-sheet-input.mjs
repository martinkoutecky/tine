// Linux real-WebKit journey for GH #619 ("query builder is unusable in Beta"): with the query sheet open,
// REAL pointer presses and REAL key presses stay inside the sheet. Before the fix a press on the sheet was
// taken by the block behind it (it entered edit mode, tore the sheet down and the control never acted), and
// the sheet opened at the viewport's top-left.
//
// Every pointer interaction here goes through the W3C Actions API (pointerdown/mousedown/mouseup/click as a
// pointing device produces them), never `element.click()` from script: a synthetic click skips the
// press events the bug lived in.
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { openPageByName } from "./lib/e2e-navigation.mjs";

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
fs.writeFileSync(`${GRAPH}/pages/Tasks.md`, "- TODO alpha task\n- TODO beta task\n- DONE finished task\n");
for (let i = 0; i < 12; i += 1) {
  fs.writeFileSync(`${GRAPH}/pages/Book ${i}.md`, `kind-${i}:: v${i}\ntype:: book\n\n- A book page ${i}\n`);
}
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
  const state = () => browser.execute(() => ({
    sheet: !!document.querySelector(".qs-sheet"),
    chooser: document.querySelector(".qs-sheet .qs-add")?.getAttribute("aria-expanded") === "true",
    editing: !!document.querySelector("textarea.block-editor"),
    rows: document.querySelectorAll(".qs-sheet .qs-row").length,
  }));
  const expectState = async (what, want) => {
    const got = await state();
    for (const [key, value] of Object.entries(want)) {
      if (got[key] !== value) throw new Error(`${what}: expected ${key}=${value}, got ${JSON.stringify(got)}`);
    }
  };

  // The query rests as a sentence; wait for it to answer so the page has settled.
  await browser.waitUntil(() => browser.execute(() => /alpha task/.test(document.querySelector(".page-blocks .query-block")?.textContent ?? "")),
    { timeout: 20_000, interval: 150, timeoutMsg: "the query never answered" });

  // 1. The gear opens the sheet next to the sentence (not at the viewport's top-left), and no block editor opens.
  await press(".page-blocks .query-block .qs-gear");
  await browser.$(".qs-sheet").waitForExist({ timeout: 10_000 });
  await expectState("after opening the sheet", { sheet: true, editing: false, chooser: false });
  const place = await browser.execute(() => {
    const sheet = document.querySelector(".qs-sheet").getBoundingClientRect();
    const sentence = document.querySelector(".page-blocks .query-block .qs-sentence").getBoundingClientRect();
    return { sheet: [sheet.left, sheet.top, sheet.right, sheet.bottom], sentence: [sentence.left, sentence.top, sentence.right, sentence.bottom] };
  });
  const [sl, st, , sb] = place.sheet;
  const [ql, qt, , qb] = place.sentence;
  if (!(Math.abs(sl - ql) < 80 && (st >= qb - 8 || sb <= qt + 8))) {
    throw new Error(`the sheet is not next to its sentence: ${JSON.stringify(place)}`);
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
  await sleep(1_000);
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
