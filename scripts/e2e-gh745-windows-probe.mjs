// GH #745: ready for native WebKitGTK and hosted WebView2; shared with Chromium.
import { gh745Markdown, gh745Png } from "./lib/gh745-fixture.mjs";
import { spawn, spawnSync } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import zlib from "node:zlib";
import { fileURLToPath } from "node:url";
import { startWebdriverApplication, stopWebdriverApplication, tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, process.platform === "win32" ? "target/release/tine.exe" : "target/release/tine");
const TD = process.env.TAURI_DRIVER || "tauri-driver";
const DRIVER = Number(process.env.E2E_DRIVER_PORT || 4602);
const NATIVE = Number(process.env.E2E_NATIVE_PORT || 4603);
const TMP = path.join(ROOT, `artifacts/gh745-outdent/native-${process.pid}`);
const GRAPH = path.join(TMP, "graph");
const OUT = process.env.GH745_OUT || process.env.E2E_ARTIFACT_DIR || path.join(ROOT, "artifacts/gh745-outdent/native");
fs.mkdirSync(OUT, { recursive: true });

for (const dir of ["pages", "journals", "logseq", "assets"]) fs.mkdirSync(`${GRAPH}/${dir}`, { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(`${TMP}/xdg/${dir}`, { recursive: true });
fs.writeFileSync(`${GRAPH}/logseq/config.edn`, "{}\n");
fs.writeFileSync(path.join(GRAPH, "pages/GH745 outline.md"), gh745Markdown());
fs.writeFileSync(path.join(GRAPH, "assets/gh745.png"), gh745Png(zlib));

const env = { ...process.env, TINE_GRAPH: GRAPH, XDG_DATA_HOME: `${TMP}/xdg/data`, XDG_CONFIG_HOME: `${TMP}/xdg/config`, XDG_CACHE_HOME: `${TMP}/xdg/cache`,
  APPDATA: path.join(TMP, "appdata"), LOCALAPPDATA: path.join(TMP, "localappdata"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
if (process.platform === "win32" && process.env.CI === "true") spawnSync("taskkill", ["/IM", path.basename(APP), "/T", "/F"], { stdio: "ignore" });
const webviewTarget = await startWebdriverApplication(APP, env, NATIVE);
const log = fs.openSync(path.join(OUT, "gh745-tauri-driver.log"), "w");
const driver = spawn(TD, webdriverServerArgs(DRIVER, NATIVE, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"),
  { env: webviewTarget.env, stdio: ["ignore", log, log], detached: process.platform !== "win32" });
await sleep(2500);
let browser;
const results = [];
try {
  browser = await remote({ hostname: "127.0.0.1", port: DRIVER, path: "/", logLevel: "error", connectionRetryCount: 1, connectionRetryTimeout: 60_000,
    capabilities: tauriCapabilities(APP, "default", process.platform, webviewTarget.debuggerAddress)
  });
  await ensureMainWindow(browser);
  try { await browser.setWindowRect(0, 0, 1920, 1040); } catch (e) { console.log(`setWindowRect: ${e.message}`); }
  await sleep(500);
  const env0 = await browser.execute(() => ({ ua: navigator.userAgent, dpr: devicePixelRatio, w: innerWidth, h: innerHeight }));
  console.log(JSON.stringify({ env: env0 }));

  const variants = [
    { name: "outdent-2-to-1", caret: "mid", key: ["Shift", "Tab"] },
    { name: "indent-1-to-2", caret: "mid", key: "Tab", reuseEditor: true },
  ].map((v) => ({ ...v, page: "GH745 outline", needle: "Section 5 row 80 text", wheel: true }))
    .filter((v) => !process.env.ONLY || process.env.ONLY.split(",").includes(v.name));
  let opened = "";
  for (const v of variants) {
    if (opened !== v.page) { await openPageByName(browser, v.page); opened = v.page; await sleep(1500); }
    const top = () => browser.execute((n) => {
      const sc = document.querySelector(".main-content");
      const el = [...document.querySelectorAll(".ls-block > .block-main")].reverse().find((e) => e.textContent.includes(n));
      return { st: sc.scrollTop, h: sc.clientHeight, top: el ? Math.round(el.getBoundingClientRect().top - sc.getBoundingClientRect().top) : null };
    }, v.needle);
    let start, before;
    if (!v.reuseEditor || !results.length) {
    // Setup: bring the block to the bottom part of the view (or `pos`).
    await browser.execute(() => { document.querySelector(".main-content").scrollTop = 0; });
    await sleep(300);
    for (let i = 0; i < 300; i++) {
      const t = await top();
      const want = v.pos !== undefined ? t.h * v.pos : t.h - 160;
      if (t.top !== null && Math.abs(t.top - want) < 60) break;
      await browser.execute((d) => { document.querySelector(".main-content").scrollTop += d; }, t.top === null ? 300 : Math.max(-300, Math.min(300, t.top - want)));
      await sleep(80);
    }
    await sleep(1000);
    start = await top();
    const wheelX = await browser.execute(() => { const r = document.querySelector(".main-content").getBoundingClientRect(); return Math.round(r.left + r.width / 2); });
    if (v.wheel) {
      for (let i = 0; i < 40; i++) {
        const t = await top();
        if (t.top !== null && t.top < 140) break;
        await browser.action("wheel").scroll({ x: wheelX, y: 300, deltaX: 0, deltaY: 100, duration: 0 }).perform();
        await sleep(60);
      }
      await sleep(1000);
    }
    before = await top();
    // Caret into the block.
    await browser.execute((n) => {
      const el = [...document.querySelectorAll(".ls-block > .block-main .block-content")].reverse().find((e) => e.textContent.includes(n));
      el.dispatchEvent(new MouseEvent("mousedown", { bubbles: true })); el.dispatchEvent(new MouseEvent("mouseup", { bubbles: true })); el.click();
    }, v.needle);
    await browser.waitUntil(() => browser.execute(() => !!document.querySelector("textarea.block-editor")), { timeout: 5000 });
    await sleep(300);
    await browser.execute((caret) => {
      const t = document.querySelector("textarea.block-editor");
      const at = caret === "start" ? 0 : Math.floor(t.value.length / 2);
      t.setSelectionRange(at, at);
    }, v.caret);
    await sleep(300);
    } else {
      // Literal reporter round trip: keep the same caret and viewport after
      // Shift+Tab, then press Tab without navigation or setup scrolling.
      start = before = await top();
    }
    const pre = await browser.execute(() => {
      const sc = document.querySelector(".main-content"); const t = document.querySelector("textarea.block-editor");
      return { st: sc.scrollTop, edTop: Math.round(t.getBoundingClientRect().top - sc.getBoundingClientRect().top), h: sc.clientHeight };
    });
    await browser.execute(() => {
      const sc = document.querySelector(".main-content");
      window.__frames = []; window.__focus = []; const t0 = performance.now();
      const foc = HTMLElement.prototype.focus;
      if (!window.__wrapped) {
        window.__wrapped = true;
        HTMLElement.prototype.focus = function (o) {
          const vt = sc.getBoundingClientRect().top; const b = sc.scrollTop; const at = Math.round(this.getBoundingClientRect().top - vt);
          const missing = document.querySelectorAll(".inline-image-missing").length;
          const r = foc.call(this, o);
          (window.__focus ??= []).push({ t: Math.round(performance.now() - (window.__t0 ?? 0)), o: JSON.stringify(o ?? null), before: b, after: sc.scrollTop, elTopAtFocus: at, elTopAfter: Math.round(this.getBoundingClientRect().top - vt), missingImages: missing });
          return r;
        };
      }
      window.__t0 = t0;
      const tick = () => {
        const ed = document.querySelector("textarea.block-editor");
        window.__frames.push([Math.round(performance.now() - t0), Math.round(sc.scrollTop), sc.scrollHeight, ed ? Math.round(ed.getBoundingClientRect().top - sc.getBoundingClientRect().top) : null, document.querySelectorAll(".inline-image-missing").length]);
        if (performance.now() - t0 < 2500) requestAnimationFrame(tick);
      };
      requestAnimationFrame(tick);
    });
    await browser.keys(v.key);
    await sleep(2800);
    const rec = await browser.execute(() => {
      const sc = document.querySelector(".main-content"); const t = document.querySelector("textarea.block-editor");
      return { frames: window.__frames, focus: window.__focus, st: sc.scrollTop, h: sc.clientHeight, edTop: t ? Math.round(t.getBoundingClientRect().top - sc.getBoundingClientRect().top) : null, edBottom: t ? Math.round(t.getBoundingClientRect().bottom - sc.getBoundingClientRect().top) : null };
    });
    const frames = []; let last = "";
    for (const f of rec.frames) { const k = f.slice(1).join("|"); if (k !== last) { frames.push(f.join("/")); last = k; } }
    const jumped = Math.abs(rec.st - pre.st) > 40 && Math.abs((rec.edTop ?? 1e6) - pre.edTop) > 120;
    const lost = rec.edTop === null || rec.edTop > rec.h || (rec.edBottom ?? 0) < 0;
    const collapsedImage = rec.focus.some((f) => f.missingImages > 0);
    const transientScroll = Math.max(0, ...rec.frames.map((f) => Math.abs(f[1] - pre.st)));
    const result = { variant: v.name, start, before, pre, after: { st: rec.st, edTop: rec.edTop, edBottom: rec.edBottom, h: rec.h }, jumped, lost, collapsedImage, transientScroll, focus: rec.focus, frames: frames.slice(0, 30) };
    results.push(result);
    console.log(JSON.stringify(result));
    fs.writeFileSync(path.join(OUT, "gh745-probe.json"), JSON.stringify({ env: env0, results }, null, 1));
    if (v.name !== "outdent-2-to-1") await browser.keys("Escape");
    await sleep(500);
    await browser.saveScreenshot(path.join(OUT, `gh745-${v.name}.png`)).catch(() => {});
  }
  fs.writeFileSync(path.join(OUT, "gh745-probe.json"), JSON.stringify({ env: env0, results }, null, 1));
  const bad = results.filter((r) => r.jumped || r.lost || r.collapsedImage || r.transientScroll > 120);
  // Probe: the verdict lives in gh745-probe.json; exit 0 so the runner does not retry (doubling the run).
  if (bad.length) console.log("PROBE VERDICT jumped: " + JSON.stringify(bad.map((r) => r.variant)));
  if (bad.length && process.env.GH745_STRICT) throw new Error(`view jumped: ${bad.map((r) => `${r.variant} (st ${r.pre.st}->${r.after.st}, editor ${r.pre.edTop}->${r.after.edTop}, lost=${r.lost})`).join("; ")}`);
  if (!bad.length) console.log("PASS: no structural edit moved the view");
} catch (error) {
  try { await browser?.saveScreenshot(path.join(OUT, "failure.png")); } catch {}
  fs.writeFileSync(path.join(OUT, "failure.json"), JSON.stringify({ commit: process.env.TINE_SOURCE_REVISION || process.env.TINE_BUILD_COMMIT || "working-tree", journey: "GH745 levels 2-1-2", expected: "loaded images and the visible caret keep their viewport geometry", observed: String(error), results }, null, 2));
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

