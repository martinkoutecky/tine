// UI-ZOOM-BACK-SCROLL: literal bullet zoom and history in the native app.
import fs from "node:fs";
import path from "node:path";
import { spawn, execFileSync } from "node:child_process";
import { setTimeout as delay } from "node:timers/promises";
import { remote } from "webdriverio";
import { openPageByName, openJournals } from "./lib/e2e-navigation.mjs";
import { installAccountHome } from "./lib/e2e-account-home.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";

const root = path.resolve(import.meta.dirname, "..");
const artifacts = path.resolve(process.env.E2E_ARTIFACT_DIR || "target/artifacts/fix-zoom-back");
fs.mkdirSync(artifacts, { recursive: true });
const out = fs.mkdtempSync(path.join(artifacts, "journey-"));
const graph = path.join(out, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
const rows = Number(process.env.E2E_ZOOM_ROWS || 700);
const content = Array.from({ length: rows }, (_, i) => `- Reading row ${i}\n  - Detail ${i}\n`).join("");
const now = new Date();
const day = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
const files = [path.join(graph, "pages/Zoom return.md"), path.join(graph, `journals/${day}.md`)];
for (const file of files) fs.writeFileSync(file, content);
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{}\n");
const home = path.join(out, "home");
for (const dir of ["data", "config", "cache", "state"]) fs.mkdirSync(path.join(home, dir), { recursive: true });
installAccountHome(out, out);
const env = { ...process.env, HOME: home, TINE_GRAPH: graph,
  XDG_DATA_HOME: path.join(home, "data"), XDG_CONFIG_HOME: path.join(home, "config"),
  XDG_CACHE_HOME: path.join(home, "cache"), XDG_STATE_HOME: path.join(home, "state"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1",
  LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4670);
const log = fs.openSync(path.join(out, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver",
  webdriverServerArgs(port, port + 1, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"),
  { env, stdio: ["ignore", log, log], detached: true });
const loadMs = Number(process.env.E2E_ZOOM_LOAD_MS || 0);
const quickReturn = process.env.E2E_ZOOM_QUICK === "1";
const proof = { commit: execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim(), rows, loadMs, quickReturn, results: [] };
let browser;
let stage = "startup";
try {
  for (let attempt = 0; ; attempt++) {
    try { if ((await fetch(`http://127.0.0.1:${port}/status`)).ok) break; } catch {}
    if (attempt >= 100) throw new Error("Driver did not start");
    await delay(100);
  }
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 0,
    capabilities: tauriCapabilities(process.env.TINE_APP || path.join(root, "target/debug/tine"), "zoom-back") });
  await ensureMainWindow(browser);
  await browser.$(".app-container").waitForExist({ timeout: 30000 });
  // Observe setters, layout and gestures without changing the restoration code.
  await browser.execute(() => {
    window.__zoomTrace = [];
    const descriptor = Object.getOwnPropertyDescriptor(Element.prototype, "scrollTop");
    Object.defineProperty(Element.prototype, "scrollTop", { ...descriptor, set(value) {
      if (this.matches?.(".main-content")) window.__zoomTrace.push({ event: "set", time: performance.now(),
        value, before: descriptor.get.call(this), height: this.scrollHeight,
        zoom: !!this.querySelector(".zoomed-page"), stack: new Error().stack });
      descriptor.set.call(this, value);
    } });
    for (const event of ["keydown", "pointerdown", "wheel"]) document.addEventListener(event, e => {
      window.__zoomTrace.push({ event, key: e.key, time: performance.now() });
    }, true);
  });
  if (loadMs) await browser.execute(ms => {
    const load = () => {
      const end = performance.now() + ms;
      while (performance.now() < end) { /* Deliberately slow the renderer for the race probe. */ }
      requestAnimationFrame(load);
    };
    requestAnimationFrame(load);
  }, loadMs);
  for (const surface of ["page", "journals"]) {
    if (surface === "page") await openPageByName(browser, "Zoom return");
    else await openJournals(browser);
    for (let iteration = 0; iteration < Number(process.env.E2E_ZOOM_RUNS || 20); iteration++) {
      const method = iteration % 2 ? "button" : "Alt+Left";
      stage = `${surface} ${method} ${iteration + 1}`;
      await browser.execute(() => { document.querySelector(".main-content").scrollTop = 12000; });
      const reading = () => browser.execute(() => {
        const sc = document.querySelector(".main-content"), box = sc.getBoundingClientRect();
        const row = [...sc.querySelectorAll(".ls-block > .block-main")].find(el => {
          const r = el.getBoundingClientRect();
          return r.top > box.top + 100 && r.bottom < box.bottom - 50 && el.textContent.includes("Reading row");
        });
        if (!row) return null;
        const bullet = row.querySelector(".bullet-container"), b = bullet.getBoundingClientRect();
        return { id: row.closest(".ls-block").dataset.blockId, text: row.textContent,
          scroll: sc.scrollTop, x: Math.round(b.x + b.width / 2), y: Math.round(b.y + b.height / 2) };
      });
      await browser.waitUntil(async () => (await reading())?.scroll > 5000, { timeout: 10000, timeoutMsg: "deep reading row missing" });
      const before = await reading();
      await browser.execute(() => { window.__zoomTrace = []; });
      await browser.performActions([{ type: "pointer", id: "mouse", parameters: { pointerType: "mouse" }, actions: [
        { type: "pointerMove", duration: 0, x: before.x, y: before.y },
        { type: "pointerDown", button: 0 }, { type: "pointerUp", button: 0 },
      ] }]);
      await browser.releaseActions();
      await browser.$(".zoomed-page").waitForExist({ timeout: 10000 });
      if (!quickReturn) await browser.waitUntil(() => browser.execute(() => {
        const sc = document.querySelector(".main-content");
        const zoom = sc.querySelector(".zoomed-block");
        if (!zoom) return false;
        const r = zoom.getBoundingClientRect(), box = sc.getBoundingClientRect();
        return sc.scrollTop < 1 && r.bottom > box.top && r.top < box.bottom;
      }), { timeout: 10000, timeoutMsg: "zoomed block did not come into view" });
      if (method === "button") await browser.$('button[title="Go back"]').click();
      else await browser.keys(["Alt", "ArrowLeft"]);
      const state = () => browser.execute(text => {
        const sc = document.querySelector(".main-content"), box = sc.getBoundingClientRect();
        const row = [...sc.querySelectorAll(".ls-block > .block-main")].find(el => el.textContent === text);
        const r = row?.getBoundingClientRect();
        return { zoom: !!sc.querySelector(".zoomed-page"), scroll: sc.scrollTop, height: sc.scrollHeight,
          visible: !!r && r.bottom > box.top && r.top < box.bottom, top: r?.top };
      }, before.text);
      let passed = true;
      try { await browser.waitUntil(async () => { const s = await state(); return !s.zoom && s.visible; },
        { timeout: 5000, interval: 100, timeoutMsg: `${stage}: reading position was lost` }); }
      catch { passed = false; }
      const result = { stage, before, after: await state(), passed, trace: await browser.execute(() => window.__zoomTrace) };
      proof.results.push(result);
      fs.writeFileSync(path.join(out, "proof.json"), JSON.stringify(proof, null, 2));
      if (!passed) fs.writeFileSync(path.join(out, `failure-${surface}-${iteration + 1}.json`), JSON.stringify({
        commit: proof.commit, journey: stage, expected: "The reading block visible before bullet zoom returns to view",
        observed: result.after, trace: result.trace, classification: "product",
      }, null, 2));
      console.log(`${passed ? "PASS" : "FAIL"} ${stage}: ${before.scroll} -> ${result.after.scroll}`);
    }
  }
  for (const file of files) if (fs.readFileSync(file, "utf8") !== content) throw new Error("Browsing changed graph bytes");
  const failures = proof.results.filter(r => !r.passed);
  if (failures.length) throw new Error(`${failures.length}/${proof.results.length} returns lost reading position`);
  console.log(`PASS: ${proof.results.length} returns; ${out}`);
} catch (error) {
  fs.writeFileSync(path.join(out, "failure.json"), JSON.stringify({ commit: proof.commit, journey: stage,
    expected: "Returning from bullet zoom restores the visible reading block", observed: String(error),
    classification: proof.results.some(r => !r.passed) ? "product" : "ambiguous" }, null, 2));
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
