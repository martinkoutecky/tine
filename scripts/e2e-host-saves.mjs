// Saving through the page host, at the real observation boundary (step 3b):
// 1. keystrokes reach the page file while the editor is still open;
// 2. external edit while typing: a conflict, resolved by Keep mine and by
//    Take disk in the page's conflict panel;
// 3. a crash (SIGKILL) with input the disk refused: the relaunch puts it back,
//    names the page ("Recovered unsaved edits"), and saves it.
// Unsaved input is made deterministic with a disk error (pages/ read-only),
// an in-scope threat; text is entered with literal key events only.
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { startWebdriverApplication, stopWebdriverApplication, tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { APP_ID } from "./lib/app-identity.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const TD = process.env.TAURI_DRIVER || "tauri-driver";
const DRIVER = Number(process.env.E2E_DRIVER_PORT || 4710);
const NATIVE = Number(process.env.E2E_NATIVE_PORT || 4711);
const TMP = process.env.E2E_TMP_DIR || path.join(os.tmpdir(), `tine-host-saves-e2e-${process.pid}`);
const GRAPH = path.join(TMP, "graph");
const PAGES = path.join(GRAPH, "pages");
const ARTIFACTS = process.env.E2E_ARTIFACT_DIR || path.join(TMP, "artifacts");
const DRAFTS = path.join(TMP, "xdg", "data", APP_ID, "drafts-v2");
const file = (name) => path.join(PAGES, `${name}.md`);

if (process.platform !== "linux") throw new Error("this journey drives a Linux disk error (chmod) and SIGKILL");
fs.rmSync(TMP, { recursive: true, force: true });
for (const dir of ["pages", "journals", "logseq", "assets"]) fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(TMP, "xdg", dir), { recursive: true });
fs.mkdirSync(ARTIFACTS, { recursive: true });
fs.writeFileSync(path.join(GRAPH, "logseq", "config.edn"), "{}\n");
fs.writeFileSync(file("Keystroke"), "- start\n");
fs.writeFileSync(file("Conflict"), "- base\n");
fs.writeFileSync(file("Crash"), "- before crash\n");

const env = {
  ...process.env,
  TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: path.join(TMP, "xdg", "data"),
  XDG_CONFIG_HOME: path.join(TMP, "xdg", "config"),
  XDG_CACHE_HOME: path.join(TMP, "xdg", "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1",
  WEBKIT_DISABLE_COMPOSITING_MODE: "1",
  LIBGL_ALWAYS_SOFTWARE: "1",
  GDK_BACKEND: "x11",
};
const log = fs.openSync(path.join(ARTIFACTS, "tauri-driver.log"), "w");
const driverArgs = webdriverServerArgs(DRIVER, NATIVE, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver");
let target, driver, browser;

async function launch(session) {
  target = await startWebdriverApplication(APP, env, NATIVE, session);
  driver = spawn(TD, driverArgs, { env: target.env, stdio: ["ignore", log, log], detached: true });
  await sleep(2500);
  browser = await remote({
    hostname: "127.0.0.1", port: DRIVER, path: "/", logLevel: "error", connectionRetryCount: 1, connectionRetryTimeout: 60_000,
    capabilities: tauriCapabilities(APP, session, process.platform, target.debuggerAddress),
  });
  await ensureMainWindow(browser);
  await browser.$(".page-title, .ls-block").waitForExist({ timeout: 20_000 });
}

/** The app processes of this run (its graph in their environment). */
function appPids() {
  return fs.readdirSync("/proc").filter((pid) => /^\d+$/.test(pid)).filter((pid) => {
    try {
      return fs.readFileSync(`/proc/${pid}/environ`, "utf8").split("\0").includes(`TINE_GRAPH=${GRAPH}`)
        && path.basename(fs.readlinkSync(`/proc/${pid}/exe`)) === path.basename(APP);
    } catch { return false; }
  }).map(Number);
}

/** A crash: SIGKILL the driver tree and every app process of this run. */
async function crash() {
  try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  for (const pid of appPids()) { try { process.kill(pid, "SIGKILL"); } catch {} }
  stopWebdriverApplication(target);
  const deadline = Date.now() + 10_000;
  while (appPids().length) {
    if (Date.now() > deadline) throw new Error(`the app survived SIGKILL: ${appPids()}`);
    await sleep(100);
  }
  browser = undefined;
}

const typeKeys = async (text) => { for (const key of text) await browser.keys([key]); };

/** Click the routed page's first block and put the caret at its end. */
async function editFirstBlock() {
  await browser.waitUntil(() => browser.execute(() => {
    const content = document.querySelector(".page .ls-block .block-content-wrapper");
    if (!(content instanceof HTMLElement)) return false;
    content.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0 }));
    content.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, cancelable: true, button: 0 }));
    content.click();
    return document.activeElement instanceof HTMLTextAreaElement;
  }), { timeout: 10_000, interval: 300, timeoutMsg: "the first block's editor did not open" });
  await browser.keys(["End"]);
}

async function toastsText() {
  return browser.execute(() => [...document.querySelectorAll(".toast .toast-msg")].map((node) => node.textContent ?? ""));
}

async function waitForToast(fragment, timeout = 20_000) {
  await browser.waitUntil(async () => (await toastsText()).some((text) => text.includes(fragment)), {
    timeout, interval: 200, timeoutMsg: `no toast containing ${JSON.stringify(fragment)}`,
  });
}

/** Files in the host's draft store that hold `text`. */
function draftsHolding(text) {
  if (!fs.existsSync(DRAFTS)) return [];
  const found = [];
  for (const graph of fs.readdirSync(DRAFTS)) {
    const dir = path.join(DRAFTS, graph);
    if (!fs.statSync(dir).isDirectory()) continue;
    for (const name of fs.readdirSync(dir)) {
      const full = path.join(dir, name);
      if (fs.statSync(full).isFile() && fs.readFileSync(full).includes(Buffer.from(text))) found.push(full);
    }
  }
  return found;
}

/** Hold unsaved input in the conflict page, then write the file from outside. */
async function conflictWith(typed, theirs) {
  await openPageByName(browser, "Conflict");
  fs.chmodSync(PAGES, 0o555);
  await editFirstBlock();
  await typeKeys(typed);
  await waitForToast("Couldn't save “Conflict” yet");
  fs.writeFileSync(file("Conflict"), theirs);
  await browser.waitUntil(() => browser.execute(() => !!document.querySelector(".page .page-conflict")), {
    timeout: 20_000, interval: 200, timeoutMsg: "no conflict panel on the page after the external edit",
  });
  fs.chmodSync(PAGES, 0o755);
  await browser.keys(["Escape"]);
}

/** Choose one side for every block, then Apply resolution. */
async function resolve(side) {
  const chosen = await browser.execute((title) => {
    const button = document.querySelector(`.page .sync-merge-toolbar-actions button[title="${title}"]`);
    if (!(button instanceof HTMLElement)) return false;
    button.click();
    return true;
  }, side);
  if (!chosen) throw new Error(`no "${side}" choice in the conflict panel`);
  await browser.waitUntil(() => browser.execute(() => {
    const apply = document.querySelector(".page .page-conflict-foot .settings-btn-primary");
    if (!(apply instanceof HTMLButtonElement) || apply.disabled) return false;
    apply.click();
    return true;
  }), { timeout: 10_000, interval: 200, timeoutMsg: "Apply resolution never enabled" });
  await browser.waitUntil(() => browser.execute(() => !document.querySelector(".page .page-conflict")), {
    timeout: 20_000, interval: 200, timeoutMsg: "the conflict panel stayed after Apply resolution",
  });
}

const steps = [];
try {
  await launch("first");

  // 1. Keystroke -> file on disk, with the editor still open.
  await openPageByName(browser, "Keystroke");
  await editFirstBlock();
  await typeKeys(" typed");
  await waitForFileText(file("Keystroke"), (text) => text === "- start typed\n");
  if (!(await browser.execute(() => document.activeElement instanceof HTMLTextAreaElement))) throw new Error("the save closed the editor");
  await browser.keys(["Escape"]);
  steps.push("keystroke saved while editing");

  // 2. External edit while typing -> conflict -> Keep mine.
  await conflictWith(" mine", "- theirs\n");
  await resolve("Your unsaved edits");
  await waitForFileText(file("Conflict"), (text) => text === "- base mine\n");
  steps.push("conflict resolved with Keep mine");

  // ... and -> Take disk: the disk text stays and the page shows it.
  await conflictWith(" again", "- theirs two\n");
  await resolve("The file on disk now");
  await browser.waitUntil(() => browser.execute(() => document.querySelector(".page .ls-block")?.textContent?.includes("theirs two") ?? false), {
    timeout: 10_000, interval: 200, timeoutMsg: "the page did not show the disk text after Take disk",
  });
  await sleep(2500);
  if (fs.readFileSync(file("Conflict"), "utf8") !== "- theirs two\n") throw new Error("Take disk wrote the discarded input");
  steps.push("conflict resolved with Take disk");

  // 3. Crash with input the disk refused -> relaunch -> recovered -> saved.
  await openPageByName(browser, "Crash");
  fs.chmodSync(PAGES, 0o555);
  await editFirstBlock();
  await typeKeys(" unsaved");
  await waitForToast("Couldn't save “Crash” yet");
  const deadline = Date.now() + 20_000;
  while (draftsHolding("before crash unsaved").length === 0) {
    if (Date.now() > deadline) throw new Error("the host never drafted the unsaved input");
    await sleep(100);
  }
  await crash();
  fs.chmodSync(PAGES, 0o755);
  if (fs.readFileSync(file("Crash"), "utf8") !== "- before crash\n") throw new Error("the refused save reached the file");
  await launch("relaunch");
  await waitForToast("Recovered unsaved edits on 1 page(s)");
  if (!(await toastsText()).some((text) => text.includes("pages/Crash.md"))) throw new Error("the recovered notice does not name the page");
  await waitForFileText(file("Crash"), (text) => text === "- before crash unsaved\n");
  steps.push("crash input recovered and saved");

  fs.writeFileSync(path.join(ARTIFACTS, "steps.json"), JSON.stringify(steps, null, 2) + "\n");
  console.log(`PASS: ${steps.join("; ")}`);
} catch (error) {
  try { fs.chmodSync(PAGES, 0o755); } catch {}
  try { await browser?.saveScreenshot(path.join(ARTIFACTS, "failure.png")); } catch {}
  try { fs.writeFileSync(path.join(ARTIFACTS, "toasts.json"), JSON.stringify(await toastsText(), null, 2)); } catch {}
  console.error(`E2E ERROR after [${steps.join("; ")}]: ${String(error).split("\n").slice(0, 5).join(" | ")}`);
  process.exitCode = 1;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  stopWebdriverApplication(target);
  for (const pid of appPids()) { try { process.kill(pid, "SIGKILL"); } catch {} }
  fs.closeSync(log);
}
