// GH #746: real WebView typing, expanded references, generated Unicode answer.
import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { remote } from "webdriverio";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { startWebdriverApplication, stopWebdriverApplication, tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";

const out = path.resolve(process.env.STALL_OUT || "artifacts/render-stall/run");
const app = path.resolve(process.env.TINE_APP || "target/release/tine");
const win = process.platform === "win32";
// Provisional Linux calibration; hosted WebView2 calibration is still pending.
// An explicitly empty value selects measurement only, e.g. the first CI run.
const budgetText = (process.env.STALL_BUDGET_MS_PER_SECOND ?? "100").trim();
const budget = budgetText ? Number(budgetText) : undefined;
if (budget !== undefined && (!Number.isFinite(budget) || budget < 0)) throw new Error("STALL_BUDGET_MS_PER_SECOND must be a nonnegative number, or empty for calibration");
const graph = path.join(out, "graph");
for (const directory of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, directory), { recursive: true });
for (const directory of ["data", "config", "cache", "appdata", "localappdata"]) fs.mkdirSync(path.join(out, directory), { recursive: true });
fs.writeFileSync(path.join(graph, "pages", "Render stall.md"), "- Typing target\n");
fs.writeFileSync(path.join(graph, "logseq", "config.edn"), "{:ref/linked-references-collapsed-threshold 1000}\n");
// ~13 MB UTF-8 answer text; the fold must visit accents and CJK, not cheap ASCII.
const pad = " Příliš žluťoučký kůň úpěl ďábelské ódy 中文日本語漢字測試".repeat(Number(process.env.STALL_PAD || 900));
for (let i = 0; i < 196; i++) {
  const mention = i < 40 ? "[[Render stall]]" : `Render stall${pad}`;
  fs.writeFileSync(path.join(graph, "pages", `Source ${i}.md`), `- Mention ${i} of ${mention}\n`);
}
const port = Number(process.env.E2E_DRIVER_PORT || 4720);
const env = { ...process.env, TINE_GRAPH: graph, XDG_DATA_HOME: path.join(out, "data"), XDG_CONFIG_HOME: path.join(out, "config"),
  XDG_CACHE_HOME: path.join(out, "cache"), APPDATA: path.join(out, "appdata"), LOCALAPPDATA: path.join(out, "localappdata"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const target = await startWebdriverApplication(app, env, port + 2, "render-stall");
const log = fs.openSync(path.join(out, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || (win ? "msedgedriver.exe" : `${process.env.CARGO_HOME}/bin/tauri-driver`),
  webdriverServerArgs(port, port + 1, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"),
  { env: target.env, detached: !win, stdio: ["ignore", log, log] });
let browser;
const metadata = fs.existsSync(`${app}.build.json`) ? JSON.parse(fs.readFileSync(`${app}.build.json`, "utf8")) : null;
const report = { app, testedCommit: process.env.STALL_COMMIT || metadata?.sourceRevision || "working-tree (see receipt)", platform: process.platform, budgetMsPerSecond: budget ?? null, fixtureBytes: Buffer.byteLength(pad) * 156, runs: [] };
try {
  for (let i = 0; ; i++) {
    try { if ((await fetch(`http://127.0.0.1:${port}/status`)).ok) break; } catch {}
    if (i > 100) throw new Error("WebDriver did not start");
    await delay(100);
  }
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 0,
    connectionRetryTimeout: 120000, capabilities: tauriCapabilities(app, "render-stall", process.platform, target.debuggerAddress) });
  // Windows attaches to whichever WebView the driver finds first (often the hidden Quick Capture).
  await browser.waitUntil(async () => (await browser.getWindowHandles()).length >= 1, { timeout: 30000 });
  await ensureMainWindow(browser, { what: "the Tine main window" });
  await browser.$(".page-title").waitForExist({ timeout: 60000 });
  await openPageByName(browser, "Render stall");
  for (const section of ["linked", "unlinked"]) {
    await browser.$(`.${section}-references .references-header`).waitForExist({ timeout: 60000 });
    await browser.execute(kind => {
      const section = document.querySelector(`.${kind}-references`);
      if (!section.querySelector(".live-ref-group")) section.querySelector(".references-header").click();
    }, section);
    await browser.$(`.${section}-references .live-ref-group`).waitForExist({ timeout: 60000 });
  }
  await delay(Number(process.env.STALL_SETTLE_MS || 10000));
  await browser.execute(() => {
    const scroller = document.querySelector(".main-content");
    scroller.scrollTop = 0;
    document.querySelector(".main-content .ls-block .block-content-wrapper").dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
  });
  await browser.$("textarea.block-editor").waitForExist();
  const seconds = Number(process.env.STALL_SECONDS || 15);
  for (let run = 0; run < Number(process.env.STALL_RUNS || 3); run++) {
    await browser.execute(() => {
      const p = window.__renderBench = { start: performance.now(), previous: 0, frames: [], inputs: [], active: true };
      p.input = e => { if (e.target.matches("textarea.block-editor")) p.inputs.push(performance.now()); };
      document.addEventListener("input", p.input, true);
      const tick = t => { if (!p.active) return; if (p.previous) p.frames.push(t - p.previous); p.previous = t; requestAnimationFrame(tick); };
      requestAnimationFrame(tick);
    });
    // One native action sequence: no WebDriver round trip between keystrokes.
    const actions = [];
    for (let i = 0; i < Math.ceil(seconds * 1000 / 33); i++) actions.push({ type: "keyDown", value: "x" }, { type: "keyUp", value: "x" }, { type: "pause", duration: 33 });
    await browser.performActions([{ type: "key", id: "held-key", actions }]);
    await browser.releaseActions();
    const raw = await browser.execute(() => {
      const p = window.__renderBench; p.active = false;
      document.removeEventListener("input", p.input, true);
      return { elapsed: performance.now() - p.start, frames: p.frames, inputs: p.inputs, focused: document.activeElement?.matches("textarea.block-editor") };
    });
    const gaps = raw.inputs.slice(1).map((at, i) => at - raw.inputs[i]);
    const metrics = { elapsedMs: raw.elapsed, inputs: raw.inputs.length, blockingMsPerSecond: raw.frames.reduce((sum, gap) => sum + (gap > 50 ? gap - 1000 / 60 : 0), 0) / (raw.elapsed / 1000),
      maxInputGapMs: Math.max(0, ...gaps), framesOver100Ms: raw.frames.filter(gap => gap > 100).length, maxFrameMs: Math.max(0, ...raw.frames) };
    report.runs.push(metrics);
    fs.writeFileSync(path.join(out, `raw-${run}.json`), JSON.stringify(raw));
    console.log(JSON.stringify(metrics));
    if (!raw.focused || raw.inputs.length < seconds * 10) throw new Error("Typing journey lost focus or did not deliver enough native input");
  }
  await browser.saveScreenshot(path.join(out, "expanded.png"));
  if (process.env.STALL_VERIFY_EXPORT === "1") {
    // Export plumbing proof only, outside all benchmark measurements.
    await browser.executeAsync(done => {
      document.activeElement.dispatchEvent(new Event("input", { bubbles: true }));
      const until = performance.now() + 150;
      while (performance.now() < until) { /* known frame stall */ }
      requestAnimationFrame(() => done());
    });
    await browser.keys("Escape");
    await browser.keys(["t", "s"]);
    await browser.$(".settings-nav-item").waitForExist();
    await browser.execute(() => [...document.querySelectorAll(".settings-nav-item")].find(element => element.textContent.includes("Help & diagnostics")).click());
    await browser.$(".diagnostics-tab").waitForExist();
    await browser.execute(() => [...document.querySelectorAll(".diagnostics-actions button")].find(element => element.textContent === "Create diagnostic report").click());
    await browser.waitUntil(() => browser.execute(() => document.querySelector(".diagnostics-preview textarea")?.value.includes('"renderer"')), { timeout: 60000, timeoutMsg: "Diagnostic export did not include renderer stalls" });
    const preview = await browser.$(".diagnostics-preview textarea").getValue();
    fs.writeFileSync(path.join(out, "diagnostic-preview.txt"), preview);
    const renderer = JSON.parse(preview).renderer;
    if (renderer.capacity !== 32 || renderer.stalls.length > 32 || !renderer.stalls.some(stall => stall.duration >= 100)) throw new Error("Diagnostic export lost the injected renderer stall or exceeded its bound");
    report.rendererExportVerified = true;
  }
  if (budget !== undefined && report.runs.some(run => run.blockingMsPerSecond > budget)) throw new Error(`Renderer blocking exceeded ${budget} ms/s`);
} catch (error) {
  report.error = String(error.stack || error);
  report.failureCapsule = { testedCommit: report.testedCommit, journey: "Typing with expanded heavy reference sections", expected: "Native input stays focused and the supplied frame blocking budget holds", observed: String(error), classification: String(error).includes("Renderer blocking exceeded") ? "product" : "ambiguous", screenshot: "failure.png" };
  await browser?.saveScreenshot(path.join(out, "failure.png")).catch(() => {});
  if (browser) fs.writeFileSync(path.join(out, "failure.html"), await browser.getPageSource().catch(() => "unavailable"));
  throw error;
} finally {
  fs.writeFileSync(path.join(out, "report.json"), JSON.stringify(report, null, 2) + "\n");
  try { await browser?.deleteSession(); } catch {}
  if (win) driver.kill(); else { try { process.kill(-driver.pid, "SIGKILL"); } catch {} }
  stopWebdriverApplication(target);
  fs.closeSync(log);
}
