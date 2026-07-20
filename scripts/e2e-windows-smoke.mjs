// Advisory Windows x64 smoke: real WebView2 + real Tauri backend + real disk.
import { spawn, spawnSync } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import {
  startWebdriverApplication,
  stopWebdriverApplication,
  tauriCapabilities,
  webdriverServerArgs,
} from "./e2e-capabilities.mjs";

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

const env = {
  ...process.env,
  TINE_GRAPH: graph,
  APPDATA: path.join(root, "appdata"),
  LOCALAPPDATA: path.join(root, "localappdata"),
};
const webviewTarget = await startWebdriverApplication(APP, env, Number(process.env.E2E_NATIVE_PORT || 4445));
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
} finally {
  try { await browser?.deleteSession(); } catch {}
  spawnSync("taskkill", ["/PID", String(driver.pid), "/T", "/F"], { stdio: "ignore" });
  stopWebdriverApplication(webviewTarget);
  if (process.env.CI === "true") {
    spawnSync("taskkill", ["/IM", path.basename(APP), "/T", "/F"], { stdio: "ignore" });
  }
  fs.closeSync(log);
}
