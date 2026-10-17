// GH #248: native WebKit input-handler and input-to-paint measurements.
// Run after npm run build + cargo build -j 4 -p tine, under xvfb-run.
import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { remote } from "webdriverio";
import { openPageByName, openJournals } from "./lib/e2e-navigation.mjs";

const out = path.resolve(process.env.ZOOM_BENCH_OUT || "artifacts/sw-zoom-typing-lag/before");
fs.mkdirSync(out, { recursive: true });
const app = path.resolve(process.env.TINE_APP || "target/debug/tine");
const tdBin = process.env.TAURI_DRIVER || `${process.env.CARGO_HOME}/bin/tauri-driver`;
const results = [];
const summary = xs => {
  const sorted = [...xs].sort((a, b) => a - b);
  return { median: sorted[Math.floor(sorted.length / 2)], p95: sorted[Math.ceil(sorted.length * .95) - 1], samples: xs };
};

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
  await browser.$(`${scope} textarea.block-editor`).waitForExist({ timeout: 10000 });
  const samples = await browser.executeAsync((selector, done) => {
    const editor = document.querySelector(`${selector} textarea.block-editor`);
    const original = editor.value;
    const handler = [], paint = [];
    let i = 0;
    function tick() {
      if (!editor.isConnected || document.querySelector(`${selector} textarea.block-editor`) !== editor) {
        done({ error: "typing lost the mounted editor" });
        return;
      }
      const deleting = i >= 30;
      editor.value = deleting ? editor.value.slice(0, -1) : editor.value + "x";
      editor.setSelectionRange(editor.value.length, editor.value.length);
      const at = performance.now();
      editor.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: deleting ? "deleteContentBackward" : "insertText", data: deleting ? null : "x" }));
      handler.push(performance.now() - at);
      requestAnimationFrame(() => setTimeout(() => {
        paint.push(performance.now() - at);
        if (++i < 60) tick();
        else done({ handler, paint, restored: editor.isConnected && editor.value === original });
      }, 0));
    }
    tick();
  }, scope);
  if (samples.error) throw new Error(samples.error);
  if (!samples.restored) throw new Error("typing/deletion did not restore editor bytes");
  results.push({ size, surface, handlerMs: summary(samples.handler), inputToPaintMs: summary(samples.paint) });
  console.log(JSON.stringify(results.at(-1)));
  await browser.saveScreenshot(path.join(out, String(size), `${surface}.png`));
  await browser.keys(["Escape"]);
}

for (const size of [400, 1500]) {
  const dir = path.join(out, String(size));
  const graph = path.join(dir, "graph");
  for (const sub of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, sub), { recursive: true });
  for (const sub of ["data", "config", "cache"]) fs.mkdirSync(path.join(dir, sub), { recursive: true });
  const raw = Array.from({ length: size }, (_, i) => `${"  ".repeat(i % 5 === 0 ? 0 : i % 5 === 4 ? 2 : 1)}- ${i === 0 ? "Zoom target" : `Nested row ${i}`}`).join("\n") + "\n";
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
    env: { ...process.env, TINE_GRAPH: graph, XDG_DATA_HOME: path.join(dir, "data"), XDG_CONFIG_HOME: path.join(dir, "config"), XDG_CACHE_HOME: path.join(dir, "cache"), WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" },
  });
  let browser;
  try {
    await sleep(1500);
    browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 1, connectionRetryTimeout: 60000,
      capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: app } } });
    await browser.$(".page-title").waitForExist({ timeout: 30000 });
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
