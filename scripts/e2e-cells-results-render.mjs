// Real-app journey for GH #782 and GH #551 item 3: a block shown inside a grid cell, and a block shown as a
// query result, renders and behaves as the same block does on a page.
//   cell:   several indent levels, a task marker with its checkbox, SCHEDULED, properties, a hosted sub-grid
//           (ADR 0031: a cell holds ordinary blocks), and a click on the checkbox toggles the marker on disk;
//   result: a Markdown list inside a block is a rendered <ul>/<ol> in the List view, as on the page.
// A real engine and renderer are the point, which jsdom cannot give (layout, the saved file).
// Usage: xvfb-run -s "-screen 0 1920x1080x24" node scripts/e2e-cells-results-render.mjs
import "./lib/e2e-auto-account-home.mjs";
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const TD = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin", "tauri-driver") : "tauri-driver");
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4526);
const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4527);
const TMP_ROOT = path.resolve(process.env.E2E_TMP_ROOT || process.env.TMPDIR || "/tmp");
fs.mkdirSync(TMP_ROOT, { recursive: true });
const TMP = fs.mkdtempSync(path.join(TMP_ROOT, "tine-cells-render-e2e-"));
const GRAPH = `${TMP}/graph`;
const ARTIFACTS = process.env.E2E_ARTIFACT_DIR || `${TMP}/artifacts`;

for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(`${GRAPH}/${dir}`, { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(`${TMP}/xdg/${dir}`, { recursive: true });
fs.mkdirSync(ARTIFACTS, { recursive: true });
fs.writeFileSync(`${GRAPH}/logseq/config.edn`, '{:preferred-format "Markdown"\n :preferred-workflow :todo\n :tine/guide-announced? true}\n');
const now = new Date();
const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(`${GRAPH}/journals/${journal}.md`, "- Open [[Cells]]\n");

const CELLS_FILE = `${GRAPH}/pages/Cells.md`;
fs.writeFileSync(CELLS_FILE, [
  "- cell host",
  "  tine.view:: grid",
  "\t-",
  "\t\t- Plain cell",
  "\t\t- Wide cell",
  "\t\t\t- Level one",
  "\t\t\t\t- Level two",
  "\t\t\t\t\t- Level three",
  "\t\t\t\t\t\t- Level four",
  "\t-",
  "\t\t- TODO Task cell",
  "\t\t  SCHEDULED: <2027-06-01 Tue>",
  "\t\t  note:: kept",
  "\t\t\t- TODO [#A] Child task",
  "\t\t\t  SCHEDULED: <2027-06-02 Wed>",
  "\t\t\t  weight:: heavy",
  "\t\t\t\t- DONE Grandchild task",
  "\t\t\t- Inner grid",
  "\t\t\t  tine.view:: grid",
  "\t\t\t\t-",
  "\t\t\t\t\t- inner A",
  "\t\t\t\t\t- inner B",
  "",
].join("\n"));
fs.writeFileSync(`${GRAPH}/pages/Lists.md`, [
  "- Shopping notes #lists",
  "  * apples",
  "  * pears",
  "- Numbered steps #lists",
  "  1. first",
  "  2. second",
  "- Long note #lists",
  ...Array.from({ length: 20 }, (_, i) => `  line ${i + 1} of a long block`),
  "",
].join("\n"));
for (const [name, view] of [["Found", null], ["FoundTable", "table"], ["FoundBoard", "board"]]) {
  fs.writeFileSync(`${GRAPH}/pages/${name}.md`, `- {{query [[lists]]}}\n${view ? `  tine.view:: ${view}\n` : ""}`);
}

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

const problems = [];
const expect = (ok, what) => { if (!ok) problems.push(what); };
let browser;
try {
  browser = await remote({
    hostname: "127.0.0.1", port: DRIVER_PORT, path: "/", logLevel: "error",
    connectionRetryCount: 1, connectionRetryTimeout: 60_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } },
  });
  await ensureMainWindow(browser);
  await browser.setWindowSize(1500, 1000);
  await browser.$(".ls-block, .page-title").waitForExist({ timeout: 20_000 });

  // ---- 1. Grid cells --------------------------------------------------------------------------------------
  await openPageByName(browser, "Cells");
  await browser.$(".sheet-cell").waitForExist({ timeout: 20_000 });
  await sleep(800);
  const cells = await browser.execute(() => {
    const cell = (text) => [...document.querySelectorAll(".sheet-cell")].find((c) => (c.textContent ?? "").includes(text));
    const left = (el) => el ? Math.round(el.getBoundingClientRect().left) : null;
    const textEl = (root, text) => root && [...root.querySelectorAll("*")].filter((e) => e.children.length === 0 || e.matches("span,a"))
      .find((e) => (e.textContent ?? "").trim() === text);
    const wide = cell("Level four");
    const task = cell("Task cell");
    const lv = ["Level one", "Level two", "Level three", "Level four"].map((t) => left(textEl(wide, t)));
    const cellLeft = left(wide);
    return {
      wideHtml: wide?.outerHTML.slice(0, 5000) ?? null,
      taskHtml: task?.outerHTML.slice(0, 9000) ?? null,
      levelLefts: lv,
      cellLeft,
      cellWidth: wide ? Math.round(wide.getBoundingClientRect().width) : null,
      taskCheckboxes: task ? task.querySelectorAll(".block-task-checkbox").length : 0,
      taskMarkers: task ? [...task.querySelectorAll(".block-marker")].map((m) => m.textContent?.trim()) : [],
      taskScheduled: task ? task.querySelectorAll(".date-chip.scheduled").length : 0,
      taskPriority: task ? [...task.querySelectorAll(".block-priority")].map((m) => m.textContent?.trim()) : [],
      taskText: (task?.textContent ?? "").replace(/\s+/g, " ").trim(),
      subGrid: task ? task.querySelectorAll(".sheet-grid").length : 0,
    };
  });
  fs.writeFileSync(`${ARTIFACTS}/cells-observed.json`, `${JSON.stringify(cells, null, 2)}\n`);

  const lefts = cells.levelLefts;
  expect(lefts.every((x) => x !== null), `the Level one..four texts were not all found in the wide cell: ${JSON.stringify(lefts)}`);
  if (lefts.every((x) => x !== null)) {
    expect(lefts[0] < lefts[1] && lefts[1] < lefts[2] && lefts[2] < lefts[3], `indent must grow with depth: ${JSON.stringify(lefts)}`);
    // Each level steps in by a bounded amount (a page indents about 29px a level), and the fourth level stays inside the cell.
    const steps = [lefts[1] - lefts[0], lefts[2] - lefts[1], lefts[3] - lefts[2]];
    expect(steps.every((s) => s <= 45), `each level must step in by at most 45px, not compound: ${JSON.stringify(steps)}`);
    expect(steps.every((s) => Math.abs(s - steps[0]) <= 4), `every level steps in by the same amount: ${JSON.stringify(steps)}`);
    expect(lefts[3] - cells.cellLeft < cells.cellWidth * 0.8, `level four must start inside the cell: lefts ${JSON.stringify(lefts)}, cell ${cells.cellLeft}+${cells.cellWidth}`);
  }
  // TODO on the cell, TODO on its child: a checkbox and a marker chip each; DONE on the grandchild: a checked box.
  expect(cells.taskCheckboxes >= 3, `the task cell shows a checkbox for its own, child and grandchild task (3), got ${cells.taskCheckboxes}`);
  expect(cells.taskMarkers.filter((m) => m === "TODO").length === 2 && cells.taskMarkers.includes("DONE"),
    `marker chips TODO, TODO, DONE expected, got ${JSON.stringify(cells.taskMarkers)}`);
  expect(cells.taskScheduled >= 2, `SCHEDULED chips for the cell and its child expected, got ${cells.taskScheduled}`);
  expect(cells.taskPriority.includes("[#A]"), `the child's [#A] priority chip expected, got ${JSON.stringify(cells.taskPriority)}`);
  expect(/note\s*:?\s*kept/i.test(cells.taskText) && /weight\s*:?\s*heavy/i.test(cells.taskText), `the cell's and the child's properties must be shown: ${cells.taskText}`);
  expect(cells.subGrid >= 1 && /inner A/.test(cells.taskText), "the hosted sub-grid inside the cell must still render");

  // A click on a checkbox inside a cell toggles the marker in the file, as on a page.
  const clicked = await browser.execute(() => {
    const cell = [...document.querySelectorAll(".sheet-cell")].find((c) => (c.textContent ?? "").includes("Task cell"));
    const box = cell?.querySelectorAll(".block-task-checkbox")[1];
    if (!box) return false;
    box.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0 }));
    return true;
  });
  expect(clicked, "no checkbox for the child task to click inside the cell");
  if (clicked) {
    await browser.waitUntil(() => fs.readFileSync(CELLS_FILE, "utf8").includes("DONE [#A] Child task"), { timeout: 10_000, interval: 200 })
      .catch(() => problems.push(`clicking the child task's checkbox in the cell did not save DONE: ${fs.readFileSync(CELLS_FILE, "utf8")}`));
  }

  // ---- 2. Query results: List, Table and Board are presentations of the same blocks --------------------------
  for (const [name, selector] of [["Found", ".query-group .ls-block"], ["FoundTable", ".sheet-title-cell"], ["FoundBoard", ".sheet-board-card"]]) {
    await openPageByName(browser, name);
    await browser.$(".query-block").waitForExist({ timeout: 20_000 });
    await browser.$(selector).waitForExist({ timeout: 20_000 });
    await sleep(1500);
    const results = await browser.execute((sel) => {
      const block = document.querySelector(".page-blocks .query-block");
      const text = (el) => (el.textContent ?? "").replace(/\s+/g, " ").trim();
      const rows = block ? [...block.querySelectorAll(sel)] : [];
      return {
        html: block?.outerHTML.slice(0, 12000) ?? null,
        rows: rows.length,
        text: block ? text(block) : null,
        ul: block ? block.querySelectorAll("ul li").length : 0,
        ol: block ? block.querySelectorAll("ol li").length : 0,
      };
    }, selector);
    if (name === "FoundTable") {
      const clamp = await browser.execute(() => {
        const cell = [...document.querySelectorAll(".query-block .sheet-title-cell")].find((c) => /Long note/.test(c.textContent ?? ""));
        const body = cell?.querySelector(".sheet-title-body");
        if (!cell || !body) return null;
        return { body: body.clientHeight, scroll: body.scrollHeight, cell: cell.getBoundingClientRect().height, font: parseFloat(getComputedStyle(body).fontSize) };
      });
      fs.writeFileSync(`${ARTIFACTS}/long-title-clamp.json`, `${JSON.stringify(clamp, null, 2)}\n`);
      expect(clamp !== null, "the long block has no .sheet-title-body in the Table view");
      if (clamp) {
        expect(clamp.body <= clamp.font * 7.5 + 2, `a 20-line block must stay clamped to about 7.5em in the Table view, got ${clamp.body}px (font ${clamp.font}px)`);
        expect(clamp.scroll > clamp.body, `the long block should overflow its clamp (scroll ${clamp.scroll} vs ${clamp.body})`);
        expect(clamp.cell < 220, `a 20-line block must not make a tall row, got ${clamp.cell}px`);
      }
    }
    fs.writeFileSync(`${ARTIFACTS}/results-${name}.json`, `${JSON.stringify(results, null, 2)}\n`);
    // A Board card shows only the block's first line by design, so lists are required as <li> in List and Table only.
    if (name !== "FoundBoard") expect(results.ul === 2, `${name}: the bullet list in a result must render as 2 <li>, got ${results.ul}; text: ${results.text}`);
    if (name !== "FoundBoard") expect(results.ol === 2, `${name}: the numbered list in a result must render as 2 <li>, got ${results.ol}; text: ${results.text}`);
    expect(!/\* apples|1\. first/.test(results.text ?? ""), `${name}: list markers must not appear as source text: ${results.text}`);
  }
} finally {
  fs.writeFileSync(`${ARTIFACTS}/problems.json`, `${JSON.stringify(problems, null, 2)}\n`);
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-td.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
if (problems.length) {
  console.error(`e2e-cells-results-render: ${problems.length} problem(s) (artifacts in ${ARTIFACTS})`);
  for (const p of problems) console.error(` - ${p}`);
  process.exit(1);
}
console.log(`e2e-cells-results-render: ok (artifacts in ${ARTIFACTS})`);
