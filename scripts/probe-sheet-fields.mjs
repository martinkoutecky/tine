// Real-app probe for the small field-cell nits (Martin, Jul 9):
//   (a) table checkbox must NOT be a white native box in dark theme
//   (b) an EMPTY date cell must open the date picker on click (add a date)
//   (c) a cell's right-click menu must offer "Delete row"
// Spawns the driver in its OWN process group and kills the group on teardown so
// WebKitWebDriver/webview grandchildren don't orphan into zombies.
// Usage: DISPLAY=:99 node scripts/probe-sheet-fields.mjs
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const G = "/tmp/sheet-fields";
const APP = process.env.TINE_APP || `${repo}/target/release/tine`;
const TD = process.env.CARGO_HOME ? `${process.env.CARGO_HOME}/bin/tauri-driver` : "tauri-driver";
const today = new Date();
const JNAME = `${today.getFullYear()}_${String(today.getMonth() + 1).padStart(2, "0")}_${String(today.getDate()).padStart(2, "0")}`;
const JFILE = `${G}/journals/${JNAME}.md`;

const MD = [
  "- Tasks",
  "  tine.view:: table",
  "  tine.fields:: done=checkbox;due=date",
  "\t- Alpha",
  "\t  done:: true",
  "\t  due:: 2026-07-20",
  "\t- Beta",
  "\t  done:: false",
  "",
].join("\n");

fs.rmSync(G, { recursive: true, force: true });
fs.mkdirSync(`${G}/journals`, { recursive: true });
fs.writeFileSync(JFILE, MD);
fs.rmSync("/tmp/sheet-fields-xdg", { recursive: true, force: true });
for (const d of ["data", "config", "cache"]) fs.mkdirSync(`/tmp/sheet-fields-xdg/${d}`, { recursive: true });
const env = {
  ...process.env,
  TINE_GRAPH: G,
  XDG_DATA_HOME: "/tmp/sheet-fields-xdg/data",
  XDG_CONFIG_HOME: "/tmp/sheet-fields-xdg/config",
  XDG_CACHE_HOME: "/tmp/sheet-fields-xdg/cache",
  WEBKIT_DISABLE_DMABUF_RENDERER: "1",
  LIBGL_ALWAYS_SOFTWARE: "1",
  WEBKIT_DISABLE_COMPOSITING_MODE: "1",
  GDK_BACKEND: "x11",
};

const tdLog = fs.openSync("/tmp/sheet-fields-td.log", "w");
const td = spawn(TD, ["--port", "4444", "--native-port", "4445", "--native-driver", "/usr/bin/WebKitWebDriver"], {
  env,
  stdio: ["ignore", tdLog, tdLog],
  detached: true, // own process group → kill the whole tree on teardown (no orphan zombies)
});
const killTree = () => {
  try {
    process.kill(-td.pid, "SIGKILL");
  } catch {
    /* already gone */
  }
};
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
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } },
    logLevel: "error",
    connectionRetryCount: 1,
    connectionRetryTimeout: 60000,
  });
  await browser.$(".ls-block").waitForExist({ timeout: 20000 });
  await browser.$(".sheet-table").waitForExist({ timeout: 10000 });
  // Force dark theme — the nit is specifically dark-theme checkboxes.
  await browser.execute(() => document.documentElement.setAttribute("data-theme", "dark"));
  await sleep(400);

  // (a) checkbox is custom-drawn (appearance:none), not a native white box.
  const cb = await browser.execute(() => {
    const el = document.querySelector(".sheet-checkbox");
    if (!el) return null;
    const s = getComputedStyle(el);
    return {
      appearance: s.appearance || s.webkitAppearance,
      borderStyle: s.borderStyle,
      // native checkboxes report a non-transparent UA background; ours is transparent when unchecked
      checkedBg: s.backgroundColor,
    };
  });
  check("table checkbox renders (found .sheet-checkbox)", !!cb, JSON.stringify(cb));
  check("checkbox is custom-drawn (appearance:none), not a native white box", !!cb && cb.appearance === "none", JSON.stringify(cb));

  // (b) the EMPTY due cell (Beta row) opens the picker on click.
  const emptyDue = await browser.execute(() => {
    // find the "due" column index from the header, then Beta's cell in it
    const headers = [...document.querySelectorAll(".sheet-header-cell")].map((h) => (h.textContent || "").trim());
    const dueCol = headers.findIndex((h) => h.toLowerCase() === "due");
    const betaTitle = [...document.querySelectorAll(".sheet-cell")].find((c) => (c.textContent || "").trim().endsWith("Beta"));
    const row = betaTitle?.getAttribute("data-row");
    const cell = document.querySelector(`.sheet-cell[data-row="${row}"][data-col="${dueCol}"]`);
    if (!cell) return { ok: false };
    const hadChip = !!cell.querySelector(".date-chip");
    cell.setAttribute("data-probe", "emptydue");
    return { ok: true, hadChip, dueCol, row };
  });
  check("empty due cell has no chip (nothing to click before the fix)", emptyDue.ok && !emptyDue.hadChip, JSON.stringify(emptyDue));
  if (emptyDue.ok) {
    await browser.$('.sheet-cell[data-probe="emptydue"]').click();
    await sleep(400);
    const pickerOpen = await browser.execute(() => !!document.querySelector(".date-picker"));
    check("clicking the empty date cell opens the date picker", pickerOpen);
    await browser.keys(["Escape"]);
    await sleep(200);
  }

  // (c) a cell's right-click menu offers "Delete row".
  const tagged = await browser.execute(() => {
    // a FIELD cell (not the title cell) — right-click there opens the cell menu
    const c = document.querySelector('.sheet-field-cell[data-row="0"][data-col="1"]');
    if (!c) return false;
    c.setAttribute("data-probe", "delrow");
    return true;
  });
  check("row-0 field cell present for menu", tagged);
  if (tagged) {
    const el = await browser.$('.sheet-cell[data-probe="delrow"]');
    await browser.execute((node) => {
      node.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 40, clientY: 40 }));
    }, el);
    await sleep(300);
    const hasDelete = await browser.execute(() =>
      [...document.querySelectorAll(".ctx-item")].some((i) => (i.textContent || "").trim() === "Delete row")
    );
    check('cell menu offers "Delete row"', hasDelete);
    if (hasDelete) {
      await browser.execute(() => {
        const item = [...document.querySelectorAll(".ctx-item")].find((i) => (i.textContent || "").trim() === "Delete row");
        item?.click();
      });
      await sleep(500);
      const rowsLeft = await browser.execute(
        () => new Set([...document.querySelectorAll(".sheet-cell[data-row]")].map((c) => c.getAttribute("data-row"))).size
      );
      check("Delete row removed a row (1 data row left)", rowsLeft === 1, `rows=${rowsLeft}`);
    }
  }

  console.log(failures === 0 ? `\nALL PASS (${checks} checks)` : `\n${failures} FAILED of ${checks}`);
} catch (e) {
  console.error("PROBE ERROR:", e);
  failures++;
} finally {
  if (browser) await browser.deleteSession().catch(() => {});
  killTree();
}
process.exit(failures === 0 ? 0 : 1);
