// M-19: cold, real-WebView reference panel expansion and native typing.
// Run sequentially under Xvfb; REF_BENCH_OUT and TINE_APP select an A/B build.
import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { remote } from "webdriverio";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";

const out = path.resolve(process.env.REF_BENCH_OUT || "artifacts/m19-wave4/baseline");
const app = path.resolve(process.env.TINE_APP || "target/release/tine");
const runs = Number(process.env.REF_BENCH_RUNS || 5);
const rows = Number(process.env.REF_BENCH_ROWS || 500);
const seconds = Number(process.env.REF_BENCH_SECONDS || 10);
// Isolate the fallback as a separate experiment by holding the source-page read
// at the IPC boundary. DTOs and the panel itself still come from the real backend.
// Default runs leave hydration untouched; never pool the two experiments.
const fallbackOnly = process.env.REF_BENCH_FALLBACK_ONLY === "1";
const basePort = Number(process.env.E2E_DRIVER_PORT || 4720);
if (process.platform !== "linux") throw new Error("This calibration harness requires Linux WebKitGTK");
if (![runs, rows, seconds].every(n => Number.isFinite(n) && n > 0)) throw new Error("Invalid benchmark dimensions");
fs.mkdirSync(out, { recursive: true });
const report = { testedCommit: process.env.REF_BENCH_COMMIT || "working tree", app, rows, seconds, fallbackOnly, runs: [] };
for (let run = 0; run < runs; run++) {
  const port = basePort + run * 3;
  const dir = path.join(out, String(run));
  if (fs.existsSync(dir)) throw new Error(`Refusing a warm profile: choose a new REF_BENCH_OUT (${dir} exists)`);
  const graph = path.join(dir, "graph");
  for (const part of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, part), { recursive: true });
  for (const part of ["data", "config", "cache"]) fs.mkdirSync(path.join(dir, part), { recursive: true });
  fs.writeFileSync(path.join(graph, "pages/Reference bench.md"), "- Typing target\n");
  fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{:ref/linked-references-collapsed-threshold 0}\n");
  const source = [];
  for (let i = 0; i < rows; i++) {
    const text = [
      `- Plain ${i} [[Reference bench]] **readable text**`,
      `- Code ${i} [[Reference bench]]\n  \`\`\`javascript\n  const answer${i} = ${i};\n  \`\`\``,
      `- TODO Date ${i} [[Reference bench]]\n  SCHEDULED: <2026-11-12 Thu>`,
      `- Parent ${i} [[Reference bench]]\n  - Nested child ${i}\n    - Grandchild ${i}`,
    ][i % 4];
    source.push(text);
  }
  fs.writeFileSync(path.join(graph, "pages/Reference source.md"), source.join("\n") + "\n");
  const env = { ...process.env, TINE_GRAPH: graph, XDG_DATA_HOME: path.join(dir, "data"),
    XDG_CONFIG_HOME: path.join(dir, "config"), XDG_CACHE_HOME: path.join(dir, "cache"),
    WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
  const log = fs.openSync(path.join(dir, "driver.log"), "w");
  const driver = spawn(process.env.TAURI_DRIVER || `${process.env.CARGO_HOME}/bin/tauri-driver`,
    webdriverServerArgs(port, port + 1, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"),
    { env, detached: true, stdio: ["ignore", log, log] });
  let browser;
  try {
    for (let i = 0; ; i++) {
      try { if ((await fetch(`http://127.0.0.1:${port}/status`)).ok) break; } catch {}
      if (i > 100) throw new Error("WebDriver did not start");
      await delay(100);
    }
    browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 0,
      connectionRetryTimeout: 120000, capabilities: tauriCapabilities(app, "ref-body-bench") });
    fs.writeFileSync(path.join(dir, "session.json"), JSON.stringify({ sessionId: browser.sessionId, port }));
    await browser.$(".page-title").waitForExist({ timeout: 60000 });
    await openPageByName(browser, "Reference bench");
    await browser.waitUntil(() => browser.execute(expected => document.querySelector(".linked-references .references-header")?.textContent.includes(String(expected)), rows),
      { timeout: 60000, timeoutMsg: `Expected ${rows} reference roots` });
    // Expansion is the clock origin. Two rAF callbacks bracket a paint opportunity;
    // settled means all 500 roots + descendants mounted, no DTO fallback, and no
    // DOM mutation for 250 ms. This is a paint proxy, not a compositor timestamp.
    await browser.execute(({ expected, fallbackOnly }) => {
      const panel = document.querySelector(".linked-references");
      window.__tineBench = true;
      window.__tineParseStats = { calls: 0, hits: 0, misses: 0 };
      if (fallbackOnly) {
        // Tauri freezes invoke/ipc. Intercept the engine transport, whose native
        // postMessage prototype method and fetch are writable, and release every
        // held real read after the screenshot. No application hooks are added.
        const fetch = window.fetch;
        const handler = window.webkit.messageHandlers.ipc;
        const proto = Object.getPrototypeOf(handler);
        const postMessage = proto.postMessage;
        const releases = [];
        window.__refHeldReads = 0;
        window.__refReleaseSource = () => {
          window.fetch = fetch;
          proto.postMessage = postMessage;
          releases.forEach(release => release());
        };
        window.fetch = function(input, init) {
          const url = String(input);
          if (url.includes("get_page_by_path") || url.endsWith("/get_page")) {
            window.__refHeldReads++;
            return new Promise((resolve, reject) => releases.push(() => fetch.call(this, input, init).then(resolve, reject)));
          }
          return fetch.call(this, input, init);
        };
        proto.postMessage = function(message) {
          const command = JSON.parse(message).cmd;
          if (command === "get_page" || command === "get_page_by_path") {
            window.__refHeldReads++;
            releases.push(() => postMessage.call(this, message));
            return;
          }
          return postMessage.call(this, message);
        };
      }
      const p = window.__refBench = { start: performance.now(), frames: [], previous: performance.now(), active: true,
        firstPaintMs: null, settledMs: null, maxFallbackRows: 0, longTasks: [], loaf: [], supported: PerformanceObserver.supportedEntryTypes || [] };
      for (const [type, key] of [["longtask", "longTasks"], ["long-animation-frame", "loaf"]]) {
        if (p.supported.includes(type)) {
          const observer = new PerformanceObserver(list => p[key].push(...list.getEntries().map(e => ({ start: e.startTime, duration: e.duration }))));
          observer.observe({ type }); (p.observers ||= []).push(observer);
        }
      }
      p.lastMutation = p.start;
      p.mutations = new MutationObserver(() => {
        p.lastMutation = performance.now();
        p.maxFallbackRows = Math.max(p.maxFallbackRows, panel.querySelectorAll(".ref-block").length);
      });
      p.mutations.observe(panel, { subtree: true, childList: true, characterData: true, attributes: true });
      const tick = t => {
        if (!p.active) return;
        if (p.previous) p.frames.push(t - p.previous);
        p.previous = t;
        const count = panel.querySelectorAll(".ls-block").length;
        p.maxFallbackRows = Math.max(p.maxFallbackRows, panel.querySelectorAll(".ref-block").length);
        if (p.firstPaintMs === null && count >= expected) requestAnimationFrame(() => { p.firstPaintMs ??= performance.now() - p.start; });
        const sourceSettled = fallbackOnly ? panel.querySelectorAll(".ref-block").length >= expected : !panel.querySelector(".ref-block");
        if (p.settledMs === null && count >= expected && sourceSettled && t - p.lastMutation >= 250) {
          p.settledMs = p.lastMutation - p.start;
          p.settleConfirmedMs = performance.now() - p.start;
          p.rowCount = count;
          p.heapBytes = performance.memory?.usedJSHeapSize ?? null;
          p.parseStats = { ...window.__tineParseStats };
        }
        requestAnimationFrame(tick);
      };
      requestAnimationFrame(tick);
      panel.querySelector(".references-header").click();
    }, { expected: rows, fallbackOnly });
    await browser.waitUntil(() => browser.execute(() => window.__refBench.settledMs !== null), { timeout: 60000, timeoutMsg: "Panel did not settle" });
    const expansion = await browser.execute(() => {
      const p = window.__refBench; p.active = false; p.mutations.disconnect(); p.observers?.forEach(o => o.disconnect());
      return { firstPaintMs: p.firstPaintMs, settledMs: p.settledMs, settleConfirmedMs: p.settleConfirmedMs,
        rowCount: p.rowCount, maxFallbackRows: p.maxFallbackRows, heapBytes: p.heapBytes,
        parseStats: p.parseStats,
        heldSourceReads: window.__refHeldReads || 0,
        supported: p.supported, longTasks: p.longTasks, loaf: p.loaf, frames: p.frames };
    });
    if (fallbackOnly && expansion.heldSourceReads < 1) throw new Error("Fallback-only experiment did not hold a source read");
    // Keep panel visible below the one-line editable target.
    await browser.execute(() => {
      document.querySelector(".main-content").scrollTop = 0;
      document.querySelector(".main-content .ls-block .block-content-wrapper").dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
    });
    await browser.$("textarea.block-editor").waitForExist();
    await browser.execute(() => {
      const p = window.__typingBench = { start: performance.now(), frames: [], inputs: [], previous: 0, active: true };
      p.input = e => { if (e.target.matches("textarea.block-editor")) p.inputs.push(performance.now()); };
      document.addEventListener("input", p.input, true);
      const tick = t => { if (!p.active) return; if (p.previous) p.frames.push(t - p.previous); p.previous = t; requestAnimationFrame(tick); };
      requestAnimationFrame(tick);
    });
    const actions = [];
    for (let i = 0; i < Math.ceil(seconds * 1000 / 33); i++) actions.push({ type: "keyDown", value: "x" }, { type: "keyUp", value: "x" }, { type: "pause", duration: 33 });
    await browser.performActions([{ type: "key", id: "typing", actions }]);
    await browser.releaseActions();
    const typing = await browser.execute(() => {
      const p = window.__typingBench; p.active = false; document.removeEventListener("input", p.input, true);
      return { elapsed: performance.now() - p.start, frames: p.frames, inputs: p.inputs,
        focused: document.activeElement?.matches("textarea.block-editor") };
    });
    if (!typing.focused || typing.inputs.length < seconds * 10) throw new Error("Native typing lost focus or inputs");
    const blockingMsPerSecond = typing.frames.reduce((sum, gap) => sum + (gap > 50 ? gap - 1000 / 60 : 0), 0) / (typing.elapsed / 1000);
    await browser.saveScreenshot(path.join(dir, "panel.png"));
    if (fallbackOnly) await browser.execute(() => window.__refReleaseSource());
    fs.writeFileSync(path.join(dir, "raw.json"), JSON.stringify({ expansion, typing }, null, 2));
    const result = { ...expansion, frames: undefined,
      expansionFramesOver50Ms: expansion.frames.filter(gap => gap > 50).length,
      expansionMaxFrameMs: Math.max(0, ...expansion.frames),
      expansionBlockingMs: expansion.frames.reduce((sum, gap) => sum + (gap > 50 ? gap - 1000 / 60 : 0), 0),
      blockingMsPerSecond, maxTypingFrameMs: Math.max(0, ...typing.frames), inputs: typing.inputs.length };
    report.runs.push(result);
    console.log(JSON.stringify({ run, ...result }));
  } catch (error) {
    report.failureCapsule = { testedCommit: report.testedCommit, journey: "500-row reference expansion and typing", expected: "Panel settles and native typing stays focused", observed: String(error.stack || error), classification: "ambiguous", screenshot: `${run}/failure.png` };
    await browser?.execute(() => {
      if (window.__refBench) { window.__refBench.active = false; window.__refBench.mutations.disconnect(); }
      if (window.__typingBench) window.__typingBench.active = false;
      window.__refReleaseSource?.();
    }).catch(() => {});
    await browser?.saveScreenshot(path.join(dir, "failure.png")).catch(() => {});
    if (browser) fs.writeFileSync(path.join(dir, "failure.html"), await browser.getPageSource().catch(() => "unavailable"));
    throw error;
  } finally {
    fs.writeFileSync(path.join(out, "report.json"), JSON.stringify(report, null, 2) + "\n");
    try { await browser?.deleteSession(); } catch {}
    const exited = driver.exitCode !== null || driver.signalCode !== null
      ? Promise.resolve() : new Promise(resolve => driver.once("exit", resolve));
    try { process.kill(-driver.pid, "SIGKILL"); } catch {}
    await exited;
    fs.closeSync(log);
  }
}
