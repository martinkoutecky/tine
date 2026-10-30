// GH #810: scrolling a long windowed page that has expanded block embeds must not jump. When the outline
// window holding an embed scrolled back into view it remounted with the embed's rows replaced by a one-line
// placeholder (the embed group's lazy-mount spacer), so the window came back about one embed shorter than
// the spacer it replaced and every block below it jumped; the rows then mounted when the embed itself
// reached the viewport and the page jumped back. The page below is synthetic (an outline of plain rows with
// three expanded block embeds of a nine-row source outline), shaped like the report's page.
// Ready for native WebKitGTK and hosted WebView2. Sampling: every animation frame, the page offset of each
// visible block present in the previous frame; a shift of the same block by more than SHIFT_LIMIT px is a jump.
// Run under xvfb-run -s "-screen 0 1920x1080x24" with TINE_APP set; GH810_EMBED=page embeds the source page.
import { spawn, spawnSync } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { startWebdriverApplication, stopWebdriverApplication, tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, process.platform === "win32" ? "target/release/tine.exe" : "target/release/tine");
const TD = process.env.TAURI_DRIVER || "tauri-driver";
const DRIVER = Number(process.env.E2E_DRIVER_PORT || 4606);
const NATIVE = Number(process.env.E2E_NATIVE_PORT || 4607);
const OUT = process.env.E2E_ARTIFACT_DIR || path.join(ROOT, "artifacts/gh810-embed-scroll");
const TMP = path.join(OUT, `native-${process.pid}`);
const GRAPH = path.join(TMP, "graph");
const SHIFT_LIMIT = Number(process.env.GH810_SHIFT_LIMIT || 40);
const STEP = Number(process.env.GH810_STEP || 120);
const PAUSE = Number(process.env.GH810_PAUSE || 45);
fs.mkdirSync(OUT, { recursive: true });

// Fixture: Source holds one block with eight descendants (a nine-row embed). The long page has 180 plain rows
// and expands that block three times (rows 6, 70, 130), so several outline windows hold an embed.
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(`${GRAPH}/${dir}`, { recursive: true });
for (const dir of ["data", "config", "cache", "home"]) fs.mkdirSync(`${TMP}/xdg/${dir}`, { recursive: true });
fs.writeFileSync(`${GRAPH}/logseq/config.edn`, "{:tine/guide-announced? true}\n");
const SOURCE_ID = "81000000-0000-4000-8000-000000000810";
const sourceRows = Array.from({ length: 8 }, (_, i) => `\t- Source child ${i} of the embedded outline`);
fs.writeFileSync(path.join(GRAPH, "pages/Embed source.md"),
  `- Embedded outline root\n  id:: ${SOURCE_ID}\n${sourceRows.join("\n")}\n`);
const EMBED_ROWS = new Set([6, 70, 130]);
// GH810_EMBED=page embeds the whole source page instead of the block (the same group component).
const EMBED = process.env.GH810_EMBED === "page" ? "{{embed [[Embed source]]}}" : `{{embed ((${SOURCE_ID}))}}`;
const rows = Array.from({ length: 180 }, (_, i) => EMBED_ROWS.has(i)
  ? `- ${EMBED}\n`
  : `- Row ${i} of the long page with a plain single line of text\n`);
fs.writeFileSync(path.join(GRAPH, "pages/Long embed page.md"), rows.join(""));

const env = { ...process.env, HOME: `${TMP}/xdg/home`, TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: `${TMP}/xdg/data`, XDG_CONFIG_HOME: `${TMP}/xdg/config`, XDG_CACHE_HOME: `${TMP}/xdg/cache`,
  APPDATA: path.join(TMP, "appdata"), LOCALAPPDATA: path.join(TMP, "localappdata"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
if (process.platform === "win32" && process.env.CI === "true") spawnSync("taskkill", ["/IM", path.basename(APP), "/T", "/F"], { stdio: "ignore" });
const webviewTarget = await startWebdriverApplication(APP, env, NATIVE);
const log = fs.openSync(path.join(OUT, "gh810-tauri-driver.log"), "w");
const driver = spawn(TD, webdriverServerArgs(DRIVER, NATIVE, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"),
  { env: webviewTarget.env, stdio: ["ignore", log, log], detached: process.platform !== "win32" });
await sleep(2500);
let browser;
let record;
try {
  browser = await remote({ hostname: "127.0.0.1", port: DRIVER, path: "/", logLevel: "error", connectionRetryCount: 1, connectionRetryTimeout: 60_000,
    capabilities: tauriCapabilities(APP, "default", process.platform, webviewTarget.debuggerAddress) });
  await ensureMainWindow(browser);
  try { await browser.setWindowRect(0, 0, 1280, 1000); } catch (e) { console.log(`setWindowRect: ${e.message}`); }
  await sleep(1000);
  await openPageByName(browser, "Long embed page");
  await browser.waitUntil(() => browser.execute(() => document.querySelectorAll(".main-content .embed-block .ls-block").length >= 9), {
    timeout: 20_000, timeoutMsg: "the first embed never showed its nine source rows" });
  await sleep(1500);
  const info = await browser.execute(() => {
    const sc = document.querySelector(".main-content");
    return { ua: navigator.userAgent, scrollHeight: sc.scrollHeight, client: sc.clientHeight, windows: document.querySelectorAll("[data-outline-window]").length };
  });
  console.log(JSON.stringify({ env: info }));
  if (info.windows < 4) throw new Error(`fixture did not window (${info.windows} outline windows); the journey would not exercise remounts`);

  await browser.execute(() => {
    const sc = document.querySelector(".main-content");
    const ids = new WeakMap(); let n = 0;
    const idOf = (el) => { let v = ids.get(el); if (v === undefined) { v = ++n; ids.set(el, v); } return v; };
    const rec = window.__gh810 = { shifts: [], frames: 0, running: true, phase: "down", embedRows: 0 };
    let prev = null;
    const tick = () => {
      const sr = sc.getBoundingClientRect();
      const st = sc.scrollTop;
      const seen = new Map();
      for (const el of document.querySelectorAll(".ls-block > .block-main")) {
        const r = el.getBoundingClientRect();
        if (r.bottom < sr.top || r.top > sr.bottom) continue;
        seen.set(idOf(el), { y: r.top - sr.top + st, text: (el.textContent || "").slice(0, 28) });
      }
      if (prev) {
        let worst = 0, who = "";
        for (const [k, v] of seen) {
          const p = prev.get(k);
          if (!p) continue;
          const d = v.y - p.y;
          if (Math.abs(d) > Math.abs(worst)) { worst = d; who = v.text; }
        }
        if (Math.abs(worst) >= 2) rec.shifts.push({ phase: rec.phase, st: Math.round(st), shift: Math.round(worst), block: who });
      }
      rec.frames++;
      prev = seen;
      if (rec.running) requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  });
  const wheelX = await browser.execute(() => { const r = document.querySelector(".main-content").getBoundingClientRect(); return Math.round(r.left + r.width / 2); });
  const position = () => browser.execute(() => { const sc = document.querySelector(".main-content"); return { st: sc.scrollTop, sh: sc.scrollHeight, ch: sc.clientHeight }; });
  const passes = [];
  for (const dir of [1, -1]) {
    await browser.execute((phase) => { window.__gh810.phase = phase; }, dir > 0 ? "down" : "up");
    let steps = 0;
    for (; steps < 1200; steps++) {
      const s = await position();
      if (dir > 0 ? s.st + s.ch >= s.sh - 2 : s.st <= 0) break;
      await browser.action("wheel").scroll({ x: wheelX, y: 300, deltaX: 0, deltaY: dir * STEP, duration: 0 }).perform();
      await sleep(PAUSE);
    }
    await sleep(1000);
    passes.push({ dir: dir > 0 ? "down" : "up", steps });
  }
  record = await browser.execute(() => { window.__gh810.running = false; return window.__gh810; });
  const embedRows = await browser.execute(() => document.querySelectorAll(".main-content .embed-block .ls-block").length);
  const jumps = record.shifts.filter((s) => Math.abs(s.shift) >= SHIFT_LIMIT);
  const result = { passes, frames: record.frames, shiftFrames: record.shifts.length, jumps, worst: Math.max(0, ...record.shifts.map((s) => Math.abs(s.shift))), embedRowsMounted: embedRows };
  console.log(JSON.stringify(result));
  fs.writeFileSync(path.join(OUT, "gh810-result.json"), JSON.stringify({ env: info, result, shifts: record.shifts }, null, 1));
  await browser.saveScreenshot(path.join(OUT, "gh810-end.png")).catch(() => {});
  if (!passes.every((p) => p.steps > 5)) throw new Error(`the page did not scroll: ${JSON.stringify(passes)}`);
  if (jumps.length) {
    throw new Error(`the page jumped while scrolling with expanded embeds: ${jumps.length} frame(s) moved a visible block by ${SHIFT_LIMIT} px or more (worst ${result.worst} px); first: ${JSON.stringify(jumps[0])}`);
  }
  console.log("PASS: scrolling down and back up past three expanded embeds moved no visible block by " + SHIFT_LIMIT + " px or more");
} catch (error) {
  try { await browser?.saveScreenshot(path.join(OUT, "failure.png")); } catch {}
  fs.writeFileSync(path.join(OUT, "failure.json"), JSON.stringify({
    commit: process.env.TINE_SOURCE_REVISION || process.env.TINE_BUILD_COMMIT || "working-tree", journey: "GH810 scroll past expanded embeds",
    expected: "no visible block jumps by SHIFT_LIMIT px or more while scrolling down and back up", observed: String(error),
    classification: "product", shifts: record?.shifts?.slice(0, 40) }, null, 2));
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  if (process.platform === "win32") {
    spawnSync("taskkill", ["/PID", String(driver.pid), "/T", "/F"], { stdio: "ignore" });
    if (process.env.CI === "true") spawnSync("taskkill", ["/IM", path.basename(APP), "/T", "/F"], { stdio: "ignore" });
  } else try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  stopWebdriverApplication(webviewTarget);
  fs.closeSync(log);
}
