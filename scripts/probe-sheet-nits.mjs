// Real-app probe for two Sheets nits (Martin, Jul 8):
//   Nit 3 — a cell shown in OUTLINE mode must be clickable into edit (the parent
//           grid's pointerdown used to swallow the click).
//   Nit 1 — arrowing to a seam must NOT spawn a scrollbar on a subgrid that
//           otherwise fits (the seam bar rounded 1px past the content box).
// Drives the REAL Tine binary under Xvfb via tauri-driver.
// Usage: DISPLAY=:99 node scripts/probe-sheet-nits.mjs
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const G = "/tmp/sheet-nits";
const APP = process.env.TINE_APP || `${repo}/target/release/tine`;
const TD =
  process.env.TAURI_DRIVER ||
  (process.env.CARGO_HOME ? `${process.env.CARGO_HOME}/bin/tauri-driver` : "tauri-driver");
const today = new Date();
const JNAME = `${today.getFullYear()}_${String(today.getMonth() + 1).padStart(2, "0")}_${String(today.getDate()).padStart(2, "0")}`;
const JFILE = `${G}/journals/${JNAME}.md`;

// Grid 1 "Outer": a NESTED grid (sub) for the scrollbar check — Aug | Sep.
// Grid 2 "Outliney": a cell "months" in OUTLINE mode (children, no tine.view).
const MD = [
  "- Outer",
  "  tine.view:: grid",
  "\t-",
  "\t\t- label",
  "\t\t- sub",
  "\t\t  tine.view:: grid",
  "\t\t\t-",
  "\t\t\t\t- Aug",
  "\t\t\t\t- Sep",
  "- Outliney",
  "  tine.view:: grid",
  "\t-",
  "\t\t- refs",
  "\t\t- months",
  "\t\t\t- Aug2",
  "\t\t\t- Sep2",
  "",
].join("\n");

fs.rmSync(G, { recursive: true, force: true });
fs.mkdirSync(`${G}/journals`, { recursive: true });
fs.writeFileSync(JFILE, MD);
fs.rmSync("/tmp/sheet-nits-xdg", { recursive: true, force: true });
for (const d of ["data", "config", "cache"]) fs.mkdirSync(`/tmp/sheet-nits-xdg/${d}`, { recursive: true });
const env = {
  ...process.env,
  TINE_GRAPH: G,
  XDG_DATA_HOME: "/tmp/sheet-nits-xdg/data",
  XDG_CONFIG_HOME: "/tmp/sheet-nits-xdg/config",
  XDG_CACHE_HOME: "/tmp/sheet-nits-xdg/cache",
  WEBKIT_DISABLE_DMABUF_RENDERER: "1",
  LIBGL_ALWAYS_SOFTWARE: "1",
  WEBKIT_DISABLE_COMPOSITING_MODE: "1",
  GDK_BACKEND: "x11",
};

const tdLog = fs.openSync("/tmp/sheet-nits-td.log", "w");
const td = spawn(TD, ["--port", "4444", "--native-port", "4445", "--native-driver", "/usr/bin/WebKitWebDriver"], {
  env,
  stdio: ["ignore", tdLog, tdLog],
});
await sleep(3000);

let failures = 0;
let checks = 0;
const check = (name, ok, extra = "") => {
  checks++;
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
  await browser.$(".sheet-cell").waitForExist({ timeout: 10000 });
  await sleep(500);

  // ---- Nit 3: clicking a cell in OUTLINE mode enters edit ------------------
  const taggedSep = await browser.execute(() => {
    const el = Array.from(document.querySelectorAll(".sheet-nested-lines .sheet-cell-body")).find(
      (b) => (b.textContent || "").trim() === "Sep2"
    );
    if (!el) return false;
    el.setAttribute("data-probe", "sep2");
    return true;
  });
  check("outline-mode 'Sep2' block is present", taggedSep);
  if (taggedSep) {
    await browser.$('[data-probe="sep2"]').click();
    await sleep(400);
    const edit = await browser.execute(() => {
      const a = document.activeElement;
      return a && a.tagName === "TEXTAREA" ? a.value : null;
    });
    check("clicking an outline-mode cell enters edit on it", edit === "Sep2", JSON.stringify(edit));
    await browser.keys(["Escape"]);
    await sleep(200);
  }

  // ---- Nit 1: selecting a seam in a fitted subgrid spawns no scrollbar -----
  // Select the nested-grid cell "Aug" (a real grid cell, not outline), then
  // arrow to a seam, and confirm the subgrid did not overflow.
  const taggedAug = await browser.execute(() => {
    const body = Array.from(document.querySelectorAll(".sheet-cell-body")).find(
      (b) => (b.textContent || "").trim() === "Aug" && b.closest(".sheet-cell") && !b.closest(".sheet-nested-lines")
    );
    const cell = body ? body.closest(".sheet-cell") : null;
    if (!cell) return false;
    cell.setAttribute("data-probe", "aug");
    return true;
  });
  check("nested-grid cell 'Aug' is present", taggedAug);
  const measure = () =>
    browser.execute(() => {
      const cell = document.querySelector('.sheet-cell[data-probe="aug"]');
      const grid = cell ? cell.closest(".sheet-grid") : null;
      if (!grid) return null;
      return {
        ox: grid.scrollWidth - grid.clientWidth,
        oy: grid.scrollHeight - grid.clientHeight,
        seam: !!document.querySelector(".sheet-seam-selected"),
      };
    });
  if (taggedAug) {
    await browser.$('.sheet-cell[data-probe="aug"]').click();
    await sleep(300);
    const before = await measure();
    await browser.keys(["ArrowRight"]); // step to the column seam
    await sleep(300);
    const after = await measure();
    check("column-seam is selected in the subgrid", !!after && after.seam, JSON.stringify(after));
    // Allow a 1px sub-pixel rounding tolerance; the bug added a full scrollbar width.
    check(
      "seam selection adds no scrollbar to a fitted subgrid",
      !!after && after.ox <= 1 && after.oy <= 1,
      `before=${JSON.stringify(before)} after=${JSON.stringify(after)}`
    );
    await browser.keys(["Escape"]);
    await sleep(150);
    await browser.$('.sheet-cell[data-probe="aug"]').click();
    await sleep(250);
    await browser.keys(["ArrowDown"]); // row seam (horizontal bar)
    await sleep(300);
    const rowSeam = await measure();
    check(
      "row-seam selection adds no scrollbar to a fitted subgrid",
      !!rowSeam && rowSeam.ox <= 1 && rowSeam.oy <= 1,
      JSON.stringify(rowSeam)
    );
  }

  console.log(failures === 0 ? `\nALL PASS (${checks} checks)` : `\n${failures} FAILED of ${checks}`);
} catch (e) {
  console.error("PROBE ERROR:", e);
  failures++;
} finally {
  if (browser) await browser.deleteSession().catch(() => {});
  td.kill("SIGKILL");
}
process.exit(failures === 0 ? 0 : 1);
