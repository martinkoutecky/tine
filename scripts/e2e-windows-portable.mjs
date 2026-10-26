// GH #719, advisory Windows x64: a portable Tine (a `tine-data` folder beside
// the exe, as the portable ZIP ships it) keeps its settings, diagnostics and
// WebView2 data in that folder and creates nothing under the host's per-user
// %APPDATA%\<id> or %LOCALAPPDATA%\<id>. Real WebView2 + real Tauri backend +
// real disk. The child keeps the real APPDATA/LOCALAPPDATA and chooses its own
// WebView2 folder, so the user's path is what runs.
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
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { APP_ID } from "./lib/app-identity.mjs";

if (process.platform !== "win32") throw new Error("windows portable must run on Windows");
const BUILT_APP = process.env.TINE_APP;
if (!BUILT_APP || !fs.existsSync(BUILT_APP)) throw new Error("TINE_APP must name the built Tine exe");
const TD = process.env.TAURI_DRIVER || "tauri-driver";
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4444);
const root = path.join(os.tmpdir(), `tine-windows-portable-${process.pid}`);
const portable = path.join(root, "Tine portable");
const data = path.join(portable, "tine-data", APP_ID);
const graph = path.join(root, "graph");
const now = new Date();
const stem = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
const journal = path.join(graph, "journals", `${stem}.md`);
fs.rmSync(root, { recursive: true, force: true });
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
fs.writeFileSync(journal, "- PORTABLE_ORIGINAL\n");
// The portable ZIP's layout: the exe and an (otherwise empty) tine-data folder.
fs.mkdirSync(path.join(portable, "tine-data"), { recursive: true });
const APP = path.join(portable, path.basename(BUILT_APP));
fs.copyFileSync(BUILT_APP, APP);

// The host's per-user folders an installed Tine of this identity uses.
const hostFolders = [process.env.APPDATA, process.env.LOCALAPPDATA]
  .filter(Boolean)
  .map((base) => path.join(base, APP_ID));
if (hostFolders.length !== 2) throw new Error("APPDATA and LOCALAPPDATA must be set on Windows");
function snapshot(dir) {
  if (!fs.existsSync(dir)) return null;
  const entries = [];
  const walk = (current) => {
    for (const entry of fs.readdirSync(current, { withFileTypes: true })) {
      const full = path.join(current, entry.name);
      const stat = fs.lstatSync(full);
      entries.push(`${path.relative(dir, full)}|${stat.size}|${stat.mtimeMs}`);
      if (entry.isDirectory()) walk(full);
    }
  };
  walk(dir);
  return entries.sort();
}
const hostBefore = hostFolders.map(snapshot);

const env = { ...process.env, TINE_GRAPH: graph };
delete env.WEBVIEW2_USER_DATA_FOLDER;
const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4445);
const webviewTarget = await startWebdriverApplication(APP, env, NATIVE_PORT, "default", process.platform, { appOwnsUserDataFolder: true });
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
  const body = await browser.$("body").getText();
  if (!body.includes("PORTABLE_ORIGINAL")) throw new Error("seeded graph did not render");
  await browser.$(".ls-block .block-content").click();
  await browser.$("textarea.block-editor").waitForExist({ timeout: 5000 });
  await browser.execute(() => {
    const editor = document.querySelector("textarea.block-editor");
    if (!editor) return;
    editor.focus();
    editor.value = "PORTABLE_SAVED";
    editor.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: null }));
  });
  await browser.keys(["Escape"]);
  await browser.waitUntil(() => fs.readFileSync(journal, "utf8").includes("PORTABLE_SAVED"), {
    timeout: 10000,
    timeoutMsg: "edited text was not persisted",
  });
  // The local folder holds the WebView2 user data; diagnostics are written at setup.
  await browser.waitUntil(() => fs.existsSync(path.join(data, "local", "EBWebView")), {
    timeout: 10000,
    timeoutMsg: `WebView2 data is not in ${path.join(data, "local")}`,
  });
  if (!fs.existsSync(path.join(data, "roaming", "diagnostics"))) {
    throw new Error(`app data is not in ${path.join(data, "roaming")}`);
  }
} finally {
  try { await browser?.deleteSession(); } catch {}
  spawnSync("taskkill", ["/PID", String(driver.pid), "/T", "/F"], { stdio: "ignore" });
  stopWebdriverApplication(webviewTarget);
  spawnSync("taskkill", ["/IM", path.basename(APP), "/T", "/F"], { stdio: "ignore" });
  fs.closeSync(log);
}

const changed = hostFolders.filter((dir, index) => JSON.stringify(snapshot(dir)) !== JSON.stringify(hostBefore[index]));
if (changed.length) throw new Error(`portable Tine wrote the host's per-user folders: ${changed.join(", ")}`);
console.log("PASS: portable Windows Tine keeps app data and WebView2 data in tine-data and leaves AppData untouched");
