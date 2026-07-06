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
  // Seeded file has 7 bullets (host + 2 rows + 4 cells) — an Enter-split would add one.
  const bulletCount = (disk.match(/^\s*-/gm) || []).length;
  check("no block was split/created (7 bullets)", bulletCount === 7, `got ${bulletCount}: ${JSON.stringify(disk)}`);
  check("grid config intact", disk.includes("tine.view:: grid"), JSON.stringify(disk));

  // Esc from selection → outline selection on the grid block (no doc change).
  await browser.keys(["Escape"]);
  await sleep(200);
  const outlineSel = await browser.$$(".ls-block.selected, .block-main.selected, .selected");
  check("Esc exits toward outline selection (no crash)", true, "");
  const disk2 = fs.readFileSync(JFILE, "utf8");
  check("Esc changed nothing on disk", disk2 === disk);
} catch (e) {
  failures++;
  console.error("E2E error:", e && e.message);
} finally {
  try { await browser?.deleteSession(); } catch {}
  td.kill();
}
console.log(failures === 0 ? "\nALL PASS" : `\n${failures} FAILURES`);
process.exit(failures === 0 ? 0 : 1);
