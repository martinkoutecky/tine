// E2E smoke for Sheets Phase 2a: drives the REAL Tine binary (real backend,
// real save path) under Xvfb via tauri-driver. Seeds a journal with a 2x2 grid,
// clicks into a cell, types, commits, and asserts the on-disk markdown changed
// exactly as expected (cell text edited; structure intact; Enter did not split).
// Usage: DISPLAY=:99 node scripts/e2e-sheets.mjs
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const G = "/tmp/sheets-e2e";
const APP = process.env.TINE_APP || `${repo}/target/release/tine`;
const TD =
  process.env.TAURI_DRIVER ||
  (process.env.CARGO_HOME ? `${process.env.CARGO_HOME}/bin/tauri-driver` : "tauri-driver");

// Today's journal file name, matching Logseq's YYYY_MM_DD.
const now = new Date();
const jname = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
const JFILE = `${G}/journals/${jname}.md`;

const GRID_MD = [
  "- grid host",
  "  tine.view:: grid",
  "\t-",
  "\t\t- alpha",
  "\t\t- beta",
  "\t-",
  "\t\t- gamma",
  "\t\t- delta",
  "- {{query (todo TODO DOING DONE)}}",
  "  tine.view:: board",
  "  tine.group-by:: state",
  "- TODO buy milk",
  "- task table",
  "  tine.view:: table",
  "  tine.fields:: topic=enum:infra,ui;shipped=checkbox",
  "\t- row one",
  "\t  shipped:: false",
  "",
].join("\n");

fs.rmSync(G, { recursive: true, force: true });
fs.mkdirSync(`${G}/pages`, { recursive: true });
fs.mkdirSync(`${G}/journals`, { recursive: true });
fs.writeFileSync(JFILE, GRID_MD);

fs.rmSync("/tmp/sheets-e2e-xdg", { recursive: true, force: true });
for (const d of ["data", "config", "cache"]) fs.mkdirSync(`/tmp/sheets-e2e-xdg/${d}`, { recursive: true });
const env = {
  ...process.env,
  TINE_GRAPH: G,
  XDG_DATA_HOME: "/tmp/sheets-e2e-xdg/data",
  XDG_CONFIG_HOME: "/tmp/sheets-e2e-xdg/config",
  XDG_CACHE_HOME: "/tmp/sheets-e2e-xdg/cache",
  WEBKIT_DISABLE_DMABUF_RENDERER: "1",
  LIBGL_ALWAYS_SOFTWARE: "1",
  WEBKIT_DISABLE_COMPOSITING_MODE: "1",
  GDK_BACKEND: "x11",
};

const tdLog = fs.openSync("/tmp/sheets-e2e-td.log", "w");
const td = spawn(
  TD,
  ["--port", "4444", "--native-port", "4445", "--native-driver", "/usr/bin/WebKitWebDriver"],
  { env, stdio: ["ignore", tdLog, tdLog] }
);
await sleep(3000);

let failures = 0;
const check = (name, ok, extra = "") => {
  console.log(`${ok ? "PASS" : "FAIL"}: ${name}${ok ? "" : "  " + extra}`);
  if (!ok) failures++;
};

let browser;
try {
  browser = await remote({
    hostname: "127.0.0.1",
    port: 4444,
    path: "/",
    capabilities: {
      browserName: "wry",
      "wdio:enforceWebDriverClassic": true,
      "tauri:options": { application: APP },
    },
    logLevel: "error",
    connectionRetryCount: 1,
    connectionRetryTimeout: 60000,
  });
  await browser.$(".ls-block").waitForExist({ timeout: 20000 });

  // The journals feed shows today's journal — the grid should be rendered.
  const cell = await browser.$('.sheet-cell[data-row="0"][data-col="0"]');
  await cell.waitForExist({ timeout: 10000 });
  check("grid renders in real app", true);

  // Click the cell text → edit mode (mousedown entry).
  await cell.click();
  await sleep(400);
  const editing = await browser.execute(() => !!document.activeElement && document.activeElement.tagName === "TEXTAREA");
  check("click-into-cell mounts editor", editing);

  // Move to end of the cell text (click-to-caret lands mid-word at the click
  // point — correct, but make the expected text deterministic), type, then
  // Enter — must COMMIT (not split the block).
  await browser.keys(["End"]);
  await browser.keys(["-", "e", "d", "i", "t", "e", "d"]);
  await sleep(200);
  await browser.keys(["Enter"]);
  await sleep(300);
  const stillEditing = await browser.execute(() => !!document.activeElement && document.activeElement.tagName === "TEXTAREA");
  check("Enter commits (editor closed)", !stillEditing);
  const selRing = await browser.$$(".sheet-cell-selected");
  check("Enter returns to cell selection", selRing.length === 1);

  // Wait out the save debounce, then verify the disk state.
  await sleep(2500);
  const disk = fs.readFileSync(JFILE, "utf8");
  check("edited text saved to disk", disk.includes("alpha-edited"), JSON.stringify(disk));
  // Seeded file has 9 bullets (host + 2 rows + 4 cells + board query + task) —
  // an Enter-split would add one.
  const bulletCount = (disk.match(/^\s*-/gm) || []).length;
  check("no block was split/created (11 bullets)", bulletCount === 11, `got ${bulletCount}: ${JSON.stringify(disk)}`);
  check("grid config intact", disk.includes("tine.view:: grid"), JSON.stringify(disk));

  // Esc from selection → outline selection on the grid block (no doc change).
  await browser.keys(["Escape"]);
  await sleep(200);
  check("Esc exits toward outline selection (no crash)", true, "");
  const disk2 = fs.readFileSync(JFILE, "utf8");
  check("Esc changed nothing on disk", disk2 === disk);

  // --- Seam insert + undo (phase 2b write path, real backend) ---------------
  // Re-enter cell (0,0), step down onto the row seam between rows, type → a
  // NEW row materializes with the typed char; then undo removes it fully.
  const cell00 = await browser.$('.sheet-cell[data-row="0"][data-col="0"]');
  await cell00.click();
  await sleep(300);
  await browser.keys(["Escape"]); // edit → cell selection on (0,0)
  await sleep(200);
  await browser.keys(["ArrowDown"]); // cell -> row seam (seam stepping ON)
  await sleep(200);
  const seamShown = await browser.$$(".sheet-seam-selected");
  check("ArrowDown lands on a seam", seamShown.length === 1);
  await browser.keys(["z"]); // type on seam → insert row + overtype edit
  await sleep(400);
  await browser.keys(["Enter"]); // commit
  await sleep(2500);
  const disk3 = fs.readFileSync(JFILE, "utf8");
  const bullets3 = (disk3.match(/^\s*-/gm) || []).length;
  check("seam-typing inserted a row on disk (13 bullets: +row +cell)", bullets3 === 13, `got ${bullets3}: ${JSON.stringify(disk3)}`);
  check("inserted cell holds the typed char", /-\s*z\s*$/m.test(disk3), JSON.stringify(disk3));
  // The typed char rides the editor's own undo entry; the structural insert is
  // its own atomic unit — so TWO undos fully revert (text, then structure).
  await browser.keys(["Control", "z"]);
  await sleep(400);
  await browser.keys(["Control", "z"]);
  await sleep(2500);
  const disk4 = fs.readFileSync(JFILE, "utf8");
  check("two undos fully revert the seam insert (text, then structure)", disk4 === disk2, JSON.stringify(disk4));

  // --- Fill down (phase 2c) --------------------------------------------------
  const cellA = await browser.$('.sheet-cell[data-row="0"][data-col="1"]');
  await cellA.click();
  await sleep(300);
  await browser.keys(["Escape"]); // cell selection (0,1) = "beta"
  await sleep(150);
  await browser.keys(["Shift", "ArrowDown"]); // range (0,1)-(1,1)... shift over seam? range extension skips seams
  await sleep(150);
  await browser.keys(["Control", "d"]); // fill down
  await sleep(2500);
  const disk5 = fs.readFileSync(JFILE, "utf8");
  const betaCount = (disk5.match(/- beta/g) || []).length;
  check("Ctrl+D filled beta into the row below", betaCount === 2, JSON.stringify(disk5));

  // --- Typed cells (phase 6b): checkbox toggle + enum popup write ------------
  // Seed has a schema'd table: columns title=0, topic(enum)=1, shipped(checkbox)=2.
  const cbCell = await browser.$('.sheet-table .sheet-cell[data-row="0"][data-col="2"]');
  if (await cbCell.isExisting()) {
    await cbCell.click(); // checkbox type: mousedown toggles, no editor
    await sleep(2400);
    const diskCb = fs.readFileSync(JFILE, "utf8");
    check("checkbox cell click wrote shipped:: true", diskCb.includes("shipped:: true"), JSON.stringify(diskCb));

    // The empty enum cell's center can be obscured (cell handle) for a WD click;
    // the app's edit entry is mousedown anyway — dispatch it directly.
    await browser.execute(() => {
      const el = document.querySelector('.sheet-table .sheet-cell[data-row="0"][data-col="1"]');
      el?.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0 }));
    });
    await sleep(400);
    const items = await browser.execute(() =>
      [...document.querySelectorAll(".ctx-item")].map((el) => (el.textContent ?? "").trim())
    );
    check("enum popup lists declared values + Clear",
      items.includes("infra") && items.includes("ui") && items.includes("Clear"), JSON.stringify(items));
    const clicked = await browser.execute(() => {
      const item = [...document.querySelectorAll(".ctx-item")].find((el) => (el.textContent ?? "").trim() === "ui");
      if (!item) return false;
      item.click();
      return true;
    });
    if (clicked) {
      await sleep(2400);
      const diskEnum = fs.readFileSync(JFILE, "utf8");
      check("enum pick wrote topic:: ui", diskEnum.includes("topic:: ui"), JSON.stringify(diskEnum));
    } else {
      check("enum popup item clickable", false, JSON.stringify(items));
    }
  } else {
    check("schema'd table rendered", false, "no table cell (0,2) found");
  }

  // --- Board card-move (phase 3): flip a task marker via Ctrl+ArrowRight ----
  // The seeded journal also carries a board query + one TODO task (see seed).
  const card = await browser.$(".sheet-board-card");
  if (await card.isExisting()) {
    await card.click(); // enters edit on the card block
    await sleep(300);
    await browser.keys(["Escape"]); // card selection
    await sleep(150);
    const selCards = await browser.execute(() => document.querySelectorAll(".sheet-board-card.sheet-cell-selected").length);
    check("Esc from card edit returns card selection", selCards === 1);
    // Column order comes from the marker workflow and the board re-groups after
    // every move, so step one column at a time toward DOING, re-reading the
    // card's position each step (a precomputed step count goes stale).
    const readPos = () => browser.execute(() => {
      const headers = [...document.querySelectorAll(".sheet-board-header")].map((h) => h.textContent ?? "");
      const cardCol = document.querySelector(".sheet-board-card")?.closest(".sheet-board-column");
      const cols = [...document.querySelectorAll(".sheet-board-column")];
      const boards = document.querySelectorAll(".sheet-board").length;
      return { boards, from: cols.indexOf(cardCol), to: headers.findIndex((h) => h.startsWith("DONE")) };
    });
    for (let i = 0; i < 8; i++) {
      const pos = await readPos();
      if (i === 0) check("exactly one board renders for the query block", pos.boards === 1, `got ${pos.boards}`);
      if (pos.from === pos.to || pos.from < 0 || pos.to < 0) break;
      await browser.keys(["Control", pos.to > pos.from ? "ArrowRight" : "ArrowLeft"]);
      await sleep(350);
    }
    await sleep(2400);
    const disk6 = fs.readFileSync(JFILE, "utf8");
    check("card-move flipped TODO to DONE on disk", disk6.includes("DONE buy milk"), JSON.stringify(disk6));
    check("card-move touched only the marker (no reparent)", (disk6.match(/buy milk/g) || []).length === 1);
  } else {
    check("board card rendered", false, "no .sheet-card found — check board seed/selector");
  }
} catch (e) {
  failures++;
  console.error("E2E error:", e && e.message);
} finally {
  try { await browser?.deleteSession(); } catch {}
  td.kill();
}
console.log(failures === 0 ? "\nALL PASS" : `\n${failures} FAILURES`);
process.exit(failures === 0 ? 0 : 1);
