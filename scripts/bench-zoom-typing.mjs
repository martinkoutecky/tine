// GH #248: native WebKit key-to-paint and process-memory measurements.
// source scripts/env.sh; npm run build; cargo build -j 4 -p tine --features custom-protocol
// LANG=C.UTF-8 xvfb-run -a -s "-screen 0 1920x1080x24" node scripts/bench-zoom-typing.mjs
// BENCH_EDITOR=textarea|cm6, ZOOM_BENCH_SIZES=1500, ZOOM_BENCH_ONLY_ZOOM=1;
// use a fresh ZOOM_BENCH_OUT for each alternating trial.
import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { remote } from "webdriverio";
import { openPageByName, openJournals } from "./lib/e2e-navigation.mjs";
import { editorKind, editorSelector, summary, scratchProfile, startTyping, stopTyping, editorText, webProcessMemory, identifyWebProcess, nativeCalibration, startNativeProfile, stopNativeProfile } from "./lib/native-editor-bench.mjs";

const out = path.resolve(process.env.ZOOM_BENCH_OUT || "artifacts/sw-zoom-typing-lag/before");
fs.mkdirSync(out, { recursive: true });
const app = path.resolve(process.env.TINE_APP || "target/debug/tine");
const tdBin = process.env.TAURI_DRIVER || `${process.env.CARGO_HOME}/bin/tauri-driver`;
const results = [];

async function bullet(browser, sidebar = false) {
  await browser.execute((shift) => {
    const b = document.querySelector('.main-content .ls-block .bullet-container');
    if (!b) throw new Error("missing zoom bullet");
    b.dispatchEvent(new MouseEvent("click", { bubbles: true, button: 0, shiftKey: shift }));
  }, sidebar);
  await browser.$(sidebar ? ".rs-item .ls-block" : ".zoomed-page .ls-block").waitForExist({ timeout: 10000 });
}

async function measure(browser, size, surface) {
  const scope = surface === "sidebar" ? ".rs-item" : ".main-content";
  await browser.execute(selector => {
    const content = document.querySelector(`${selector} .ls-block .block-content-wrapper`);
    if (!content) throw new Error(`no editable content at ${selector}`);
    content.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
  }, scope);
  await browser.$(`${scope} ${editorSelector}`).waitForExist({ timeout: 10000 });
  await browser.keys(["End"]);
  const original = await editorText(browser, scope);
  const calibration = process.env.BENCH_CALIBRATE === "1" ? await nativeCalibration(browser) : undefined;
  if (process.env.BENCH_PROFILE === "1") await startNativeProfile(browser);
  await startTyping(browser);
  const actions = Array.from({ length: 60 }, (_, i) => [
    { type: "keyDown", value: i < 30 ? "x" : "\uE003" }, { type: "keyUp", value: i < 30 ? "x" : "\uE003" }, { type: "pause", duration: 40 },
  ]).flat();
  await browser.performActions([{ type: "key", id: "typing", actions }]);
  await browser.releaseActions();
  const samples = await stopTyping(browser);
  const profile = process.env.BENCH_PROFILE === "1" ? await stopNativeProfile(browser) : undefined;
  const restored = await editorText(browser, scope);
  fs.writeFileSync(path.join(out, String(size), `${surface}-raw.json`), JSON.stringify({ original, restored, ...samples }, null, 2));
  // CM's Backspace keymap edits its state directly, without a DOM input event.
  if (!samples.focused || samples.keys.length !== 60 || samples.inputs.length < 30 || samples.paint.length !== 60 || restored !== original) throw new Error(`native typing lost focus, samples or original editor bytes: ${JSON.stringify({ focused: samples.focused, keys: samples.keys.length, inputs: samples.inputs.length, paint: samples.paint.length, original, restored })}`);
  results.push({ editor: editorKind, size, surface, calibration, profile, keyToPaintMs: summary(samples.paint), framesOver50Ms: samples.frames.filter(n => n > 50).length,
    framesOver100Ms: samples.frames.filter(n => n > 100).length, raw: samples });
  console.log(JSON.stringify({ ...results.at(-1), raw: undefined }));
  await browser.saveScreenshot(path.join(out, String(size), `${surface}.png`));
  await browser.keys(["Escape"]);
}

for (const size of (process.env.ZOOM_BENCH_SIZES || "400,1500").split(",").map(Number)) {
  const dir = path.join(out, String(size));
  const graph = path.join(dir, "graph");
  for (const sub of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, sub), { recursive: true });
  for (const sub of ["data", "config", "cache"]) fs.mkdirSync(path.join(dir, sub), { recursive: true });
  // All remaining rows belong to the zoom target (formerly only four did).
  const raw = Array.from({ length: size }, (_, i) => `${"  ".repeat(i === 0 ? 0 : i % 5 === 4 ? 2 : 1)}- ${i === 0 ? "Zoom target" : `Nested row ${i}`}`).join("\n") + "\n";
  const pageFile = path.join(graph, "pages", "Zoom Bench.md");
  fs.writeFileSync(pageFile, raw);
  for (let i = 0; i < 19; i++) fs.writeFileSync(path.join(graph, "pages", `Filler ${i}.md`), "- Fixture\n");
  const today = new Date();
  const journal = `${today.getFullYear()}_${String(today.getMonth() + 1).padStart(2, "0")}_${String(today.getDate()).padStart(2, "0")}`;
  fs.writeFileSync(path.join(graph, "journals", `${journal}.md`), raw);
  const log = fs.openSync(path.join(dir, "driver.log"), "w");
  const port = 4892;
  const td = spawn("dbus-run-session", ["--", tdBin, "--port", String(port), "--native-port", String(port + 1), "--native-driver", "/usr/bin/WebKitWebDriver"], {
    detached: true, stdio: ["ignore", log, log],
    env: { ...process.env, ...scratchProfile(dir), TINE_GRAPH: graph, WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" },
  });
  let browser;
  try {
    await sleep(1500);
    browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 1, connectionRetryTimeout: 60000,
      capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: app } } });
    await browser.$(".page-title").waitForExist({ timeout: 30000 });
    if (process.env.ZOOM_BENCH_ONLY_ZOOM === "1") {
      await openPageByName(browser, "Zoom Bench");
      await bullet(browser);
      const attribution = await identifyWebProcess(browser, td.pid);
      await sleep(2000);
      const memory = [{ opened: 0, ...webProcessMemory(td.pid, attribution.pid) }];
      for (let opened = 1; opened <= 10; opened++) {
        await browser.execute(index => {
          const wrappers = [...document.querySelectorAll(".zoomed-page .ls-block .block-content-wrapper")];
          if (!wrappers[index]) throw new Error("Not enough zoom descendants");
          wrappers[index].dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
        }, opened);
        await browser.$(editorSelector).waitForExist({ timeout: 10000 });
        await sleep(1000);
        if (opened === 1 || opened === 10) memory.push({ opened, activeEditors: (await browser.$$(editorSelector)).length, ...webProcessMemory(td.pid, attribution.pid) });
        await browser.keys(["Escape"]);
      }
      fs.writeFileSync(path.join(dir, "memory.json"), JSON.stringify({ editor: editorKind, attribution, memory }, null, 2));
      await measure(browser, size, "routed-zoom");
      continue;
    }
    await measure(browser, size, "journal-page");
    await bullet(browser);
    await measure(browser, size, "journal-zoom");
    await openPageByName(browser, "Zoom Bench");
    await measure(browser, size, "routed-page");
    await bullet(browser, true);
    await measure(browser, size, "sidebar");
    await browser.execute(() => document.querySelector('.rs-item .rs-close').click());
    await bullet(browser);
    await measure(browser, size, "routed-zoom");
    await openJournals(browser);
    await browser.saveScreenshot(path.join(dir, "journal.png"));
  } catch (error) {
    if (browser) await browser.saveScreenshot(path.join(dir, "failure.png")).catch(() => {});
    fs.writeFileSync(path.join(dir, "failure.txt"), String(error));
    throw error;
  } finally {
    try { await browser?.deleteSession(); } catch {}
    try { process.kill(-td.pid, "SIGKILL"); } catch {}
    fs.closeSync(log);
    fs.writeFileSync(path.join(out, "measurements.json"), JSON.stringify(results, null, 2) + "\n");
  }
}
