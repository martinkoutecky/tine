// Focused Linux native journey and fixed-baseline Ctrl+K probes for GH #151.
// All graph/settings fixtures and receipts stay under artifacts/pinyin.
// Run after source scripts/env.sh, under dbus-run-session and xvfb-run.
import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { remote } from "webdriverio";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { APP_ID } from "./lib/app-identity.mjs";
import { startWebdriverApplication, stopWebdriverApplication, tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";

const root = path.resolve(new URL("../", import.meta.url).pathname);
const out = path.join(root, "artifacts/pinyin", `native-${process.pid}`);
const graph = path.join(out, "graph");
const benchmark = process.env.PINYIN_UI_BENCH_GRAPH;
if (benchmark) fs.cpSync(benchmark, graph, { recursive: true });
const app = process.env.TINE_APP || path.join(root, "target/release/tine");
for (const dir of ["pages", "journals", "logseq", "assets"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(out, "xdg", dir), { recursive: true });
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{}\n");
for (const name of ["北京", "北景", "beijing", "bjmarker", "x beijing"]) fs.writeFileSync(path.join(graph, "pages", `${name}.md`), "- fixture body\n");
const data = path.join(out, "xdg/data", APP_ID);
fs.mkdirSync(data, { recursive: true });
fs.writeFileSync(path.join(data, "tine-settings.json"), JSON.stringify({ "ui_language": "en", "pinyin-page-matching": process.env.PINYIN_UI_ENABLED !== "off" }));
const env = { ...process.env, TINE_GRAPH: graph, XDG_DATA_HOME: path.join(out, "xdg/data"),
  XDG_CONFIG_HOME: path.join(out, "xdg/config"), XDG_CACHE_HOME: path.join(out, "xdg/cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4790), nativePort = port + 1;
const target = await startWebdriverApplication(app, env, nativePort, "pinyin");
const log = fs.openSync(path.join(out, "driver.log"), "w");
const td = spawn(process.env.TAURI_DRIVER || "tauri-driver", webdriverServerArgs(port, nativePort, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"), {
  env: target.env, stdio: ["ignore", log, log], detached: true,
});
let browser;
const receipt = { revision: "508380d5e51b8581b0bfa7769f441489e65f2d2d + run 3 diff", journey: benchmark ? "Fixed warm Ctrl+K latency" : "Pinyin native priority, inline/Capture completion, cross-window setting", sha256: crypto.createHash("sha256").update(fs.readFileSync(app)).digest("hex") };
try {
  await sleep(2500);
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 1, connectionRetryTimeout: 60000,
    capabilities: tauriCapabilities(app, "pinyin", process.platform, target.debuggerAddress) });
  await ensureMainWindow(browser);
  const mainHandle = await browser.getWindowHandle();
  console.log("main", await browser.getUrl());
  if (!benchmark) console.log("initial", await browser.execute(() => ({ ready: document.readyState, text: document.body?.innerText?.slice(0,1000) })));
  await browser.$(".ls-block, .page-title").waitForExist({ timeout: 20000 });
  const type = async text => { for (const key of text) await browser.keys([key]); };
  const names = () => browser.execute(() => [...document.querySelectorAll(".switcher-row .switcher-name")].map(n => n.textContent.trim()));
  const search = async q => {
    await browser.keys(["Control", "k"]);
    await browser.$(".switcher-input").waitForExist({ timeout: 5000 });
    await type(q);
  };
  const completionLabels = () => browser.execute(() => [...document.querySelectorAll(".autocomplete .ac-label")].map(n => n.textContent.trim()));
  const complete = async (query, capture = false) => {
    await browser.execute((q, cap) => {
      const textarea = document.querySelector(cap ? ".capture-shell textarea" : "textarea.block-editor");
      textarea.focus(); textarea.value = `[[${q}]]`;
      textarea.setSelectionRange(q.length + 2, q.length + 2);
      textarea.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: q.at(-1) }));
    }, query, capture);
  };
  const showCapture = async () => {
    const secondary = spawn(app, ["--capture"], { env, stdio: "ignore" });
    await new Promise((resolve, reject) => { secondary.on("error", reject); secondary.on("exit", code => code === 0 ? resolve() : reject(Error(`capture forwarder exit ${code}`))); });
    await browser.waitUntil(async () => {
      for (const handle of await browser.getWindowHandles()) {
        await browser.switchToWindow(handle);
        if ((await browser.getUrl()).includes("capture.html")) return true;
      }
      return false;
    }, { timeout: 10000 });
    await browser.$(".capture-shell textarea").waitForExist({ timeout: 10000 });
  };
  if (benchmark) {
    const measurements = [];
    for (let i = 0; i < 22; i++) {
      const q = i % 2 ? "bj" : "beijing";
      const expected = i % 2 ? "bjmarker" : "beijing";
      const started = performance.now();
      await search(q);
      await browser.waitUntil(async () => (await names()).includes(expected), { timeout: 30000, interval: 10 });
      if (i >= 2) measurements.push(performance.now() - started);
      await browser.keys(["Escape"]);
    }
    measurements.sort((a,b) => a-b);
    receipt.latency = { p50_ms: measurements[10], p95_ms: measurements[19], samples: 20 };
    receipt.outcome = "passed";
    console.log(JSON.stringify(receipt));
  } else {
  await search("beijing");
  await browser.waitUntil(async () => (await names()).includes("北京"), { timeout: 10000, timeoutMsg: "Pinyin result missing" });
  const ranked = await names();
  if (!ranked.includes("beijing") || ranked.indexOf("beijing") > ranked.indexOf("北京")) throw Error("native exact did not precede expansion");
  receipt.nativePriority = ranked;
  await browser.keys(["Escape"]);
  await search("bj");
  await browser.waitUntil(async () => (await names()).includes("北京"), { timeout: 10000 });
  receipt.step = "initials activation";
  // Observe selection, then navigate with literal keyboard input.
  for (let i = 0; i < 20; i++) {
    const active = await browser.execute(() => document.querySelector(".switcher-row.active .switcher-name")?.textContent?.trim());
    if (active === "北京") break;
    await browser.keys(["ArrowDown"]);
  }
  await browser.keys(["Enter"]);
  await browser.waitUntil(async () => (await browser.$("h1.page-title").getText()).trim() === "北京", { timeout: 10000 });
  receipt.initialsActivation = true;
  receipt.step = "inline completion native substring priority";
  await browser.$(".ls-block .block-content").click();
  await browser.$("textarea.block-editor").waitForExist({ timeout: 5000 });
  await complete("beijing");
  await browser.waitUntil(async () => (await completionLabels()).includes("北京"), { timeout: 10000 });
  receipt.inlineOrder = await completionLabels();
  if (receipt.inlineOrder.indexOf("x beijing") < 0 || receipt.inlineOrder.indexOf("x beijing") > receipt.inlineOrder.indexOf("北京")) throw Error("inline expansion outranks native substring");
  await browser.keys(["Escape"]);
  await browser.keys(["Escape"]);

  receipt.step = "warm Capture and native fuzzy priority";
  await showCapture();
  await complete("bj", true);
  await browser.waitUntil(async () => (await completionLabels()).includes("北京"), { timeout: 10000 });
  receipt.captureOrder = await completionLabels();
  if (receipt.captureOrder.indexOf("beijing") < 0 || receipt.captureOrder.indexOf("beijing") > receipt.captureOrder.indexOf("北京")) throw Error("Capture expansion outranks native fuzzy");
  await browser.keys(["Escape"]);
  await browser.keys(["Escape"]);
  await browser.switchToWindow(mainHandle);
  receipt.step = "settings";
  await browser.keys(["Escape"]);
  await browser.$('button[title^="Settings"]').click();
  await browser.$(".settings-modal").waitForExist({ timeout: 5000 });
  await browser.$("//button[contains(@class,'settings-nav-item') and normalize-space(.)='Editor']").click();
  const advanced = await browser.$(".settings-advanced-toggle");
  if ((await advanced.getAttribute("aria-expanded")) !== "true") await advanced.click();
  const toggle = await browser.$('[data-setting-label="Match Chinese page names by Pinyin"] [role="switch"]');
  await toggle.waitForExist({ timeout: 5000 });
  if ((await toggle.getAttribute("aria-checked")) !== "true") throw Error("seeded device setting not enabled");
  await toggle.scrollIntoView();
  await toggle.click();
  await browser.waitUntil(async () => (await toggle.getAttribute("aria-checked")) === "false", { timeout: 5000 });
  await browser.keys(["Escape"]);
  await search("bj");
  await browser.waitUntil(() => browser.execute(() => !document.body.textContent.includes("Searching…")), { timeout: 10000 });
  if ((await names()).includes("北京")) throw Error("disabled matching retained Han result");
  receipt.settingOff = true;
  await browser.keys(["Escape"]);
  receipt.step = "persistent Capture after main preference off";
  await showCapture();
  await complete("bj", true);
  await browser.waitUntil(async () => (await completionLabels()).includes("bjmarker"), { timeout: 10000 });
  receipt.captureAfterOff = await completionLabels();
  if (receipt.captureAfterOff.includes("北京")) throw Error("Capture kept Pinyin after preference off in main");
  await browser.saveScreenshot(path.join(out, "off.png"));
  receipt.outcome = "passed";
  }
} catch (error) {
  receipt.outcome = "failed"; receipt.observed = String(error); receipt.expected = "native-first Han match, successful activation, no Han match after disabling";
  receipt.classification = "ambiguous";
  if (browser && !benchmark) await browser.saveScreenshot(path.join(out, "failure.png")).catch(() => {});
  throw error;
} finally {
  fs.writeFileSync(path.join(out, "receipt.json"), JSON.stringify(receipt, null, 2));
  if (browser) await browser.deleteSession().catch(() => {});
  try { process.kill(-td.pid, "SIGTERM"); } catch {}
  await stopWebdriverApplication(target);
  fs.closeSync(log);
}
