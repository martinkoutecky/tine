// UI-PALETTE-POINTER-ACTIVATION: a real pointer keeps its destination pane.
import fs from "node:fs";
import path from "node:path";
import { spawn, execFileSync } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { remote } from "webdriverio";
import { installAccountHome, assertAppLease } from "./lib/e2e-account-home.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";

const root = path.resolve(import.meta.dirname, "..");
const artifacts = process.env.E2E_ARTIFACT_DIR || path.join(root, "target/artifacts/og-palette-click");
fs.mkdirSync(artifacts, { recursive: true });
const out = fs.mkdtempSync(path.join(artifacts, "palette-"));
const graph = path.join(out, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["home", "data", "config", "cache"]) fs.mkdirSync(path.join(out, dir));
fs.writeFileSync(path.join(graph, "pages/Palette Origin.md"), '- ```js\n  const demo = 1;\n  ```\n');
fs.writeFileSync(path.join(graph, "pages/Palette Destination.md"), "- Unique pointer block marker\n");
const account = installAccountHome(out);
const env = { ...process.env, HOME: path.join(out, "home"), TINE_GRAPH: graph,
  XDG_DATA_HOME: path.join(out, "data"), XDG_CONFIG_HOME: path.join(out, "config"), XDG_CACHE_HOME: path.join(out, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4986);
const log = fs.openSync(path.join(out, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver",
  webdriverServerArgs(port, port + 1, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"),
  { env, stdio: ["ignore", log, log], detached: true });
const proof = { commit: execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim(), workingDiff: true,
  journey: "Pointer activation of command, page, block and create-page rows keeps the focused split", checks: [], traces: [] };
let browser;
let stage = "startup";
let paneId;
const screenshot = async (file) => {
  // Match the native editor journeys: repeated WebDriver screenshots can stall.
  if (process.platform === "linux") execFileSync("import", ["-window", "root", file], { timeout: 10000 });
  else await browser.saveScreenshot(file);
};
const checkPane = async (name) => {
  await browser.waitUntil(() => browser.execute((pane, title) => {
    const current = document.querySelector(`[data-pane-id="${pane}"] h1.page-title`);
    return current?.textContent.includes(title) && !document.querySelector(".switcher-input");
  }, paneId, name), { timeout: 10000, timeoutMsg: `Row did not open ${name} in the focused split` });
  const state = await browser.execute(pane => ({
    focused: document.querySelector(".pane-focused")?.getAttribute("data-pane-id"),
    main: document.querySelector('[data-pane-id="main"] h1.page-title')?.textContent,
    target: document.querySelector(`[data-pane-id="${pane}"] h1.page-title`)?.textContent,
  }), paneId);
  if (state.focused !== paneId || !state.main?.includes("Palette Origin")) throw Error(JSON.stringify(state));
  proof.checks.push({ stage, ...state });
};
async function search(query, command = false) {
  await browser.keys(command ? ["Control", "Shift", "p"] : ["Control", "k"]);
  const input = await browser.$(".switcher-input"); await input.waitForDisplayed(); await input.setValue(query);
  await browser.waitUntil(() => browser.execute(() => !document.querySelector('#switcher-results [role="status"]')));
}
async function clickRow(selector) {
  const row = await browser.$(selector); await row.waitForDisplayed();
  await browser.execute(() => { window.palettePointerTrace = []; });
  await row.click();
  proof.traces.push({ stage, events: await browser.execute(() => window.palettePointerTrace) });
}
try {
  await sleep(1200);
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 0,
    connectionRetryTimeout: 30000,
    capabilities: tauriCapabilities(process.env.TINE_APP || path.join(root, "target/release/tine"), "palette-pointer") });
  await ensureMainWindow(browser);
  await browser.$(".app-container").waitForExist({ timeout: 30000 });
  await openPageByName(browser, "Palette Origin");
  await assertAppLease(account);
  await browser.keys(["Control", "Alt", "\\"]);
  await browser.waitUntil(() => browser.execute(() => document.querySelectorAll(".pane-leaf").length === 2));
  paneId = await browser.execute(() => document.querySelector('.pane-leaf:not([data-pane-id="main"])').getAttribute("data-pane-id"));
  await browser.execute(() => {
    for (const type of ["pointerdown", "mousedown", "pointerup", "mouseup", "click"]) {
      document.addEventListener(type, event => {
        if (!event.target?.closest?.(".switcher-row")) return;
        const item = { type, trusted: event.isTrusted, cancelled: event.defaultPrevented };
        window.palettePointerTrace?.push(item);
        queueMicrotask(() => { item.connected = event.target.isConnected; item.cancelled = event.defaultPrevented; });
      }, true);
    }
  });
  stage = "command pointer";
  await search("Toggle code block wrap", true);
  await clickRow(".switcher-row*=Toggle code block wrap");
  await browser.waitUntil(() => browser.execute(() => !document.querySelector(".switcher-input") &&
    [...document.querySelectorAll("pre.code-block")].every(el => el.classList.contains("code-wrapping"))), { timeout: 10000 });
  await checkPane("Palette Origin");
  await screenshot(path.join(out, "command-pointer.png"));
  stage = "command Enter";
  await search("Toggle code block wrap", true);
  await browser.keys(["Enter"]);
  await browser.waitUntil(() => browser.execute(() => !document.querySelector(".switcher-input") &&
    [...document.querySelectorAll("pre.code-block")].every(el => !el.classList.contains("code-wrapping"))), { timeout: 10000 });
  await checkPane("Palette Origin");
  stage = "page pointer";
  await search("Palette Destination");
  await clickRow('//*[@role="option"][not(contains(@class,"block-result"))][contains(.,"Palette Destination")]');
  await checkPane("Palette Destination");
  await screenshot(path.join(out, "page-pointer.png"));
  await search("Palette Origin");
  await clickRow('//*[@role="option"][not(contains(@class,"block-result"))][contains(.,"Palette Origin")]');
  await checkPane("Palette Origin");
  stage = "block pointer";
  await search("Unique pointer block marker");
  await clickRow('//*[@role="option"][contains(@class,"block-result")][contains(.,"Unique pointer block marker")]');
  await checkPane("Palette Destination");
  await screenshot(path.join(out, "block-pointer.png"));
  stage = "create-page pointer";
  await search("Fresh Pointer Page");
  await clickRow(".switcher-row*=Create page: Fresh Pointer Page");
  await checkPane("Fresh Pointer Page");
  await browser.waitUntil(() => fs.existsSync(path.join(graph, "pages/Fresh Pointer Page.md")), { timeout: 10000 });
  await screenshot(path.join(out, "create-page-pointer.png"));
  proof.outcome = "PASS";
  fs.writeFileSync(path.join(out, "proof.json"), JSON.stringify(proof, null, 2));
  console.log(`PASS palette pointer: ${out}`);
} catch (error) {
  try { await screenshot(path.join(out, "failure.png")); } catch {}
  fs.writeFileSync(path.join(out, "failure.json"), JSON.stringify({ ...proof, stage,
    expected: "Each row acts once in the focused split; main page stays Palette Origin; palette closes",
    observed: String(error.stack), classification: "ambiguous", artifacts: out }, null, 2));
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
