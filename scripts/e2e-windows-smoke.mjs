// Advisory Windows x64 smoke: real WebView2 + real Tauri backend + real disk.
import { spawn, spawnSync } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  startWebdriverApplication,
  stopWebdriverApplication,
  tauriCapabilities,
  webdriverServerArgs,
} from "./e2e-capabilities.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { currentPageTitle, openPageByLink, openPageByName } from "./lib/e2e-navigation.mjs";

if (process.platform !== "win32") throw new Error("windows smoke must run on Windows");
const APP = process.env.TINE_APP;
const TD = process.env.TAURI_DRIVER || "tauri-driver";
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4444);
const root = path.join(os.tmpdir(), `tine-windows-smoke-${process.pid}`);
const graph = path.join(root, "graph");
const now = new Date();
const stem = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
const journal = path.join(graph, "journals", `${stem}.md`);
fs.rmSync(root, { recursive: true, force: true });
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
fs.writeFileSync(journal, "- WINDOWS_SMOKE_ORIGINAL\n");
const historyPages = Array.from({ length: 8 }, (_, i) => `Side button ${i}`);
for (const [i, name] of historyPages.entries()) {
  const next = historyPages[i + 1];
  fs.writeFileSync(path.join(graph, "pages", `${name}.md`), next ? `- [[${next}]]\n` : "- Last history page\n");
}

const env = {
  ...process.env,
  TINE_GRAPH: graph,
  APPDATA: path.join(root, "appdata"),
  LOCALAPPDATA: path.join(root, "localappdata"),
};
const webviewTarget = await startWebdriverApplication(APP, env, Number(process.env.E2E_NATIVE_PORT || 4445));
const historyInput = fileURLToPath(new URL("./lib/e2e-windows-history-input.ps1", import.meta.url));
function pressHistoryButton(button, delivery) {
  const input = spawnSync("powershell.exe", [
    "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", historyInput,
    "-ProcessId", String(webviewTarget.applicationProcess.pid), "-Button", String(button), "-Delivery", delivery,
  ], { encoding: "utf8", windowsHide: true });
  if (input.status !== 0) {
    throw new Error(`GH #948 input harness (${delivery}, XBUTTON${button}): ${input.error ?? input.stderr.trim()}`);
  }
}
const log = fs.openSync(path.join(process.env.E2E_ARTIFACT_DIR || root, "tauri-driver.log"), "w");
const driver = spawn(TD, webdriverServerArgs(DRIVER_PORT), { env: webviewTarget.env, stdio: ["ignore", log, log] });
await sleep(3000);

let browser;
try {
  browser = await remote({
    hostname: "127.0.0.1",
    port: DRIVER_PORT,
    path: "/",
    capabilities: tauriCapabilities(APP, "default", process.platform, webviewTarget.debuggerAddress),
    logLevel: "error",
    connectionRetryCount: 1,
    connectionRetryTimeout: 60000,
  });
  await ensureMainWindow(browser);
  await browser.$(".ls-block").waitForExist({ timeout: 30000 });
  // The first journal block can paint before the application shell's reactive
  // content subtree is mounted on WebView2. The styles below are the actual
  // scroll-containment contract, so wait for both required elements rather than
  // passing a transient null to getComputedStyle().
  await browser.$(".app-container").waitForExist({ timeout: 10_000 });
  await browser.$(".main-content").waitForExist({ timeout: 10_000 });
  const overscroll = await browser.execute(() => {
    const shell = document.querySelector(".app-container");
    const main = document.querySelector(".main-content");
    return {
      html: getComputedStyle(document.documentElement).overscrollBehavior,
      body: getComputedStyle(document.body).overscrollBehavior,
      shell: shell instanceof Element ? getComputedStyle(shell).overscrollBehavior : null,
      mainOverflow: main instanceof Element ? getComputedStyle(main).overflowY : null,
    };
  });
  if (overscroll.html !== "none" || overscroll.body !== "none" || overscroll.shell !== "none") {
    throw new Error(`viewport scroll chain is not contained: ${JSON.stringify(overscroll)}`);
  }
  if (!["auto", "scroll"].includes(overscroll.mainOverflow)) {
    throw new Error(`main pane no longer has ordinary scrolling: ${JSON.stringify(overscroll)}`);
  }
  const body = await browser.$("body").getText();
  if (!body.includes("WINDOWS_SMOKE_ORIGINAL")) throw new Error("seeded graph did not render");
  await browser.$(".ls-block .block-content").click();
  await browser.$("textarea.block-editor").waitForExist({ timeout: 5000 });
  await browser.execute(() => {
    const editor = document.querySelector("textarea.block-editor");
    if (!editor) return;
    editor.focus();
    editor.value = "WINDOWS_SMOKE_SAVED";
    editor.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: null }));
  });
  await browser.keys(["Escape"]);
  await browser.waitUntil(() => fs.readFileSync(journal, "utf8").includes("WINDOWS_SMOKE_SAVED"), {
    timeout: 10000,
    timeoutMsg: "edited text was not persisted",
  });
  await browser.refresh();
  await browser.$(".ls-block").waitForExist({ timeout: 20000 });
  const reloaded = await browser.$("body").getText();
  if (!reloaded.includes("WINDOWS_SMOKE_SAVED") || reloaded.includes("WINDOWS_SMOKE_ORIGINAL")) {
    throw new Error("saved text did not survive reload");
  }
  console.log("PASS: Windows WebView2 launch, overscroll containment, graph render, edit, save, and reload");

  await openPageByName(browser, historyPages[0]);
  for (const name of historyPages.slice(1)) await openPageByLink(browser, name);
  // Leave an extra page in each direction so excess steps stay observable.
  await browser.keys(["Alt", "ArrowLeft"]);
  for (const delivery of ["mouse", "native", "native-first", "browser-first"]) {
    for (const [button, start, delta] of [[1, 6, -1], [2, 0, 1]]) {
      for (let press = 1; press <= 6; press++) {
        const before = historyPages[start + (press - 1) * delta];
        const expected = historyPages[start + press * delta];
        const step = `GH #948 ${delivery}, XBUTTON${button}, press ${press}`;
        await browser.waitUntil(async () => (await currentPageTitle(browser)) === before, {
          timeout: 5_000, timeoutMsg: `${step}: starting page should be ${before}`,
        });
        pressHistoryButton(button, delivery === "browser-first" ? "mouse" : delivery);
        try {
          await browser.waitUntil(async () => (await currentPageTitle(browser)) === expected, { timeout: 5_000 });
          if (delivery === "browser-first") pressHistoryButton(button, "forwarded");
          const observed = await currentPageTitle(browser);
          if (observed !== expected) throw new Error(`unexpected extra step to ${observed}`);
        } catch (error) {
          const observed = await currentPageTitle(browser);
          throw new Error(`${step}: expected ${expected}, observed ${observed || "no routed page"}`, { cause: error });
        }
      }
    }
    console.log(`PASS: GH #948 ${delivery}, six Windows Back and six Forward presses each moved one page`);
  }
} finally {
  try { await browser?.deleteSession(); } catch {}
  spawnSync("taskkill", ["/PID", String(driver.pid), "/T", "/F"], { stdio: "ignore" });
  stopWebdriverApplication(webviewTarget);
  if (process.env.CI === "true") {
    spawnSync("taskkill", ["/IM", path.basename(APP), "/T", "/F"], { stdio: "ignore" });
  }
  fs.closeSync(log);
}
