import os from "node:os";
// GH #731: dragging a block selection or a block by its bullet to the bottom edge of a long page
// scrolls the pane, so blocks beyond the first screen are selected / receive the move.
import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { remote } from "webdriverio";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";

const ROOT = path.resolve(import.meta.dirname, "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const DRIVER = process.env.TAURI_DRIVER || path.join(process.env.CARGO_HOME || "", "bin/tauri-driver");
const artifacts = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(os.tmpdir(), "tine-e2e-gh731"));
fs.mkdirSync(artifacts, { recursive: true });
const TMP = fs.mkdtempSync(path.join(artifacts, "run-"));
const GRAPH = path.join(TMP, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(TMP, dir));
const ROWS = 160;
const rows = Array.from({ length: ROWS }, (_, i) => `row ${i}`);
const original = rows.map((r) => `- ${r}\n`).join("");
const pageFile = path.join(GRAPH, "pages/Long.md");
fs.writeFileSync(pageFile, original);
const env = { ...process.env, TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: path.join(TMP, "data"), XDG_CONFIG_HOME: path.join(TMP, "config"), XDG_CACHE_HOME: path.join(TMP, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4740);
const log = fs.openSync(path.join(TMP, "driver.log"), "w");
const driver = spawn(DRIVER, ["--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log], detached: true });
let browser;
let stage = "startup";
const testedBase = process.env.E2E_TESTED_BASE || "og + gh731 working diff";
const connect = () => remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 2, connectionRetryTimeout: 30_000,
  capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } } });

const mouse = (actions) => browser.performActions([{ type: "pointer", id: "mouse", parameters: { pointerType: "mouse" }, actions }]);
const moveTo = (x, y, duration = 40) => ({ type: "pointerMove", duration, x: Math.round(x), y: Math.round(y) });
const rowNumber = (text) => Number(/^row (\d+)$/.exec((text ?? "").trim())?.[1] ?? NaN);

/** Viewport point of a row's rendered text, or its bullet. */
const rowPoint = (n, what) => browser.execute((n, what) => {
  const block = [...document.querySelectorAll(".ls-block")].find((b) => b.querySelector(":scope > .block-main .block-content")?.textContent?.trim() === `row ${n}`);
  if (!block) return null;
  const node = what === "bullet" ? block.querySelector(":scope > .block-main .bullet-container") : block.querySelector(":scope > .block-main .block-content");
  const r = node.getBoundingClientRect();
  return { x: what === "bullet" ? r.left + r.width / 2 : r.left + 12, y: r.top + r.height / 2 };
}, n, what);

const state = () => browser.execute(() => {
  const scroller = document.querySelector(".main-content");
  const visibleRows = [...document.querySelectorAll(".ls-block")].map((b) => {
    const text = b.querySelector(":scope > .block-main .block-content")?.textContent?.trim() ?? "";
    return { text, bottom: b.getBoundingClientRect().bottom };
  });
  return {
    scrollTop: scroller?.scrollTop ?? -1,
    innerHeight: window.innerHeight,
    selected: [...document.querySelectorAll(".block-main.selected")].map((m) => m.querySelector(".block-content")?.textContent?.trim() ?? ""),
    visibleRows,
  };
});
const lastVisibleRow = (s) => Math.max(...s.visibleRows.filter((r) => r.bottom <= s.innerHeight).map((r) => rowNumber(r.text)).filter(Number.isFinite));

try {
  browser = await connect();
  await browser.$(".app-container").waitForExist({ timeout: 30_000 });
  await openPageByName(browser, "Long");
  await browser.waitUntil(async () => (await rowPoint(3, "text")) !== null, { timeout: 15_000, timeoutMsg: "row 3 never rendered" });
  const first = await state();
  const firstScreen = lastVisibleRow(first);
  if (!(firstScreen >= 8 && firstScreen < ROWS - 40)) throw new Error(`the fixture does not overflow the window: first screen ends at row ${firstScreen} of ${ROWS}`);

  stage = "drag-select to the bottom edge";
  const from = await rowPoint(3, "text");
  const to = await rowPoint(5, "text");
  const edgeY = first.innerHeight - 3;
  await mouse([moveTo(from.x, from.y, 0), { type: "pointerDown", button: 0 }, moveTo(to.x, to.y), moveTo(to.x, (to.y + edgeY) / 2), moveTo(to.x, edgeY), { type: "pause", duration: 2500 }]);
  const held = await state();
  await mouse([{ type: "pointerUp", button: 0 }]);
  await browser.releaseActions();
  const selectedRows = held.selected.map(rowNumber).filter(Number.isFinite);
  if (held.scrollTop <= 200) throw new Error(`holding the pointer at the bottom edge did not scroll the page (scrollTop ${held.scrollTop})`);
  if (!(Math.max(...selectedRows) > firstScreen + 3)) throw new Error(`the selection did not extend beyond the first screen (row ${firstScreen}): ${JSON.stringify(selectedRows)}`);
  const contiguous = selectedRows.sort((a, b) => a - b).every((n, i, all) => i === 0 || n === all[i - 1] + 1);
  // Rows scrolled far out of view may be unmounted: the rendered part must be one run.
  if (!contiguous) throw new Error(`the selected rows are not one contiguous run: ${JSON.stringify(selectedRows)}`);
  const settled = await state();
  await browser.pause(600);
  const later = await state();
  if (later.scrollTop !== settled.scrollTop) throw new Error(`the page kept scrolling after the button was released (${settled.scrollTop} -> ${later.scrollTop})`);
  const laterRows = later.selected.map(rowNumber).filter(Number.isFinite);
  // The pane keeps scrolling (and extending) until the button is actually released.
  if (!laterRows.length || Math.max(...laterRows) < Math.max(...selectedRows)) throw new Error("the block selection did not stand after the drop");
  const finalRows = (await state()).selected.map(rowNumber).filter(Number.isFinite);
  if (JSON.stringify(finalRows) !== JSON.stringify(laterRows)) throw new Error("the block selection changed after the drop");
  if (fs.readFileSync(pageFile, "utf8") !== original) throw new Error("selecting blocks changed the page file");

  stage = "drag-move by the bullet to the bottom edge";
  await browser.keys(["Escape"]);
  await browser.execute(() => { document.querySelector(".main-content").scrollTop = 0; });
  await browser.waitUntil(async () => (await rowPoint(1, "bullet")) !== null, { timeout: 10_000, timeoutMsg: "row 1 bullet not visible after scrolling back" });
  const bullet = await rowPoint(1, "bullet");
  await mouse([moveTo(bullet.x, bullet.y, 0), { type: "pointerDown", button: 0 }, moveTo(bullet.x + 40, bullet.y + 80), moveTo(bullet.x + 40, edgeY - 120), moveTo(bullet.x + 40, edgeY), { type: "pause", duration: 2500 }]);
  const moving = await state();
  await mouse([{ type: "pointerUp", button: 0 }]);
  await browser.releaseActions();
  if (moving.scrollTop <= 200) throw new Error(`dragging a block to the bottom edge did not scroll the page (scrollTop ${moving.scrollTop})`);
  const saved = await waitForFileText(pageFile, (text) => text !== original, { timeoutMs: 15_000 });
  const lines = saved.split("\n").filter((l) => l !== "");
  const at = lines.findIndex((l) => l.trim() === "- row 1");
  if (at < 0 || lines.filter((l) => l.trim() === "- row 1").length !== 1) throw new Error(`row 1 is not present exactly once after the move:\n${saved.slice(0, 400)}`);
  if (at <= firstScreen) throw new Error(`row 1 landed at line ${at}, not beyond the first screen (${firstScreen})`);
  const rest = lines.filter((_, i) => i !== at).map((l) => l.trim());
  const expectedRest = rows.filter((r) => r !== "row 1").map((r) => `- ${r}`);
  if (JSON.stringify(rest) !== JSON.stringify(expectedRest)) throw new Error("the move disturbed other blocks' text or order");
  const after = await state();
  await browser.pause(600);
  if ((await state()).scrollTop !== after.scrollTop) throw new Error("the page kept scrolling after the drop");
  fs.writeFileSync(path.join(TMP, "proof.json"), JSON.stringify({ testedBase, app: APP, firstScreen, selectedUpToRow: Math.max(...selectedRows), scrollTopSelect: held.scrollTop, scrollTopMove: moving.scrollTop, rowOneLine: at,
    outcome: "PASS: holding the pointer at the bottom edge scrolled the pane for drag-select and bullet drag-move; blocks beyond the first screen were selected / received the move; file bytes were exactly the expected move" }));
  console.log(`PASS GH 731 drag autoscroll: ${TMP}`);
} catch (error) {
  try { await browser?.saveScreenshot(path.join(TMP, "failure.png")); } catch {}
  try { fs.writeFileSync(path.join(TMP, "failure-dom.html"), await browser.getPageSource()); } catch {}
  fs.writeFileSync(path.join(TMP, "failure.txt"), `Tested: ${testedBase}\nJourney: long routed page, real mouse drag to the bottom edge. Stage: ${stage}. Observed: ${error.stack}\n`);
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
