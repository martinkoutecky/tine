// Real routed-page pointer/keyboard activation and disk round-trip for GH #158.
// This deliberately drives the target and textarea through WebDriver: DOM state
// helpers are evidence only, never the way text is entered or saved.
import { spawn, spawnSync } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, process.platform === "win32" ? "target/release/tine.exe" : "target/release/tine");
const DRIVER = Number(process.env.E2E_DRIVER_PORT || 4592);
const NATIVE = Number(process.env.E2E_NATIVE_PORT || 4593);
const TMP = path.join(os.tmpdir(), `tine-page-trailing-block-e2e-${process.pid}`);
const GRAPH = path.join(TMP, "graph");
const PAGE = "Trailing target";
const PAGE_FILE = path.join(GRAPH, "pages", `${PAGE}.md`);
const ARTIFACTS = process.env.E2E_ARTIFACT_DIR || TMP;

fs.rmSync(TMP, { recursive: true, force: true });
for (const dir of ["pages", "journals", "logseq", "assets"]) fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(TMP, "xdg", dir), { recursive: true });
fs.mkdirSync(ARTIFACTS, { recursive: true });
fs.writeFileSync(path.join(GRAPH, "logseq", "config.edn"), "{}\n");
fs.writeFileSync(PAGE_FILE, "- Existing root\n");

const env = {
  ...process.env,
  TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: path.join(TMP, "xdg", "data"),
  XDG_CONFIG_HOME: path.join(TMP, "xdg", "config"),
  XDG_CACHE_HOME: path.join(TMP, "xdg", "cache"),
  APPDATA: path.join(TMP, "appdata"),
  LOCALAPPDATA: path.join(TMP, "localappdata"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1",
  WEBKIT_DISABLE_COMPOSITING_MODE: "1",
  LIBGL_ALWAYS_SOFTWARE: "1",
  GDK_BACKEND: "x11",
};

if (process.platform === "win32" && process.env.CI === "true") {
  spawnSync("taskkill", ["/IM", path.basename(APP), "/T", "/F"], { stdio: "ignore" });
}
const driverLog = fs.openSync(path.join(ARTIFACTS, "tauri-driver.log"), "w");
const driver = spawn(
  process.env.TAURI_DRIVER || "tauri-driver",
  webdriverServerArgs(DRIVER, NATIVE, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"),
  { env, stdio: ["ignore", driverLog, driverLog], detached: process.platform !== "win32" },
);

let browser;
async function openPage(name) {
  if ((await browser.$$(".nav-page")).length === 0) {
    const expanded = await browser.execute(() => {
      const header = [...document.querySelectorAll(".nav-section-header")]
        .find((element) => element.textContent?.includes("ALL PAGES"));
      if (!header) return false;
      header.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, button: 0 }));
      return true;
    });
    if (!expanded) throw new Error("missing ALL PAGES sidebar section");
  }
  await browser.waitUntil(async () => (await browser.$$(".nav-page")).length > 0, { timeout: 15_000, timeoutMsg: "page index did not load" });
  const result = await browser.execute((wanted) => {
    const row = [...document.querySelectorAll(".nav-page")].find((element) => element.textContent?.trim() === wanted);
    if (!row) return { ok: false, names: [...document.querySelectorAll(".nav-page")].map((element) => element.textContent?.trim()) };
    row.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, button: 0 }));
    return { ok: true, names: [] };
  }, name);
  if (!result.ok) throw new Error(`missing page ${name}: ${JSON.stringify(result.names)}`);
  await browser.waitUntil(async () => (await browser.$("h1.page-title").getText()).trim() === name, { timeout: 10_000, timeoutMsg: `did not route to ${name}` });
}

async function activeEditorReceipt(label) {
  const receipt = await browser.execute(() => {
    const active = document.activeElement;
    const row = active?.closest?.("[data-block-id]");
    return {
      activeTag: active?.tagName ?? null,
      id: row?.getAttribute("data-block-id") ?? null,
      selectionStart: active instanceof HTMLTextAreaElement ? active.selectionStart : null,
      selectionEnd: active instanceof HTMLTextAreaElement ? active.selectionEnd : null,
      textareaCount: row?.querySelectorAll("textarea").length ?? 0,
      inRightSidebar: !!active?.closest?.(".right-sidebar"),
    };
  });
  fs.writeFileSync(path.join(ARTIFACTS, `${label}-active.json`), JSON.stringify(receipt, null, 2) + "\n");
  if (receipt.activeTag !== "TEXTAREA" || receipt.textareaCount !== 1 || receipt.selectionStart !== 0 || receipt.selectionEnd !== 0 || receipt.inRightSidebar) {
    throw new Error(`${label} did not mount one focused main-surface textarea: ${JSON.stringify(receipt)}`);
  }
  return receipt;
}

async function waitForFile(text, label) {
  const deadline = Date.now() + 10_000;
  while (Date.now() < deadline) {
    const body = fs.readFileSync(PAGE_FILE, "utf8");
    if (body.includes(text)) return body;
    await sleep(100);
  }
  throw new Error(`${label} did not persist: ${JSON.stringify(fs.readFileSync(PAGE_FILE, "utf8"))}`);
}

async function target() {
  const button = await browser.$(".page-trailing-block-target");
  await button.waitForExist({ timeout: 5_000 });
  return button;
}

try {
  await sleep(2500);
  browser = await remote({
    hostname: "127.0.0.1", port: DRIVER, path: "/", logLevel: "error", connectionRetryCount: 1, connectionRetryTimeout: 60_000,
    capabilities: tauriCapabilities(APP),
  });
  await browser.$(".page-title").waitForExist({ timeout: 20_000 });
  await sleep(2500);
  await openPage(PAGE);

  // Literal pointer path: WebDriver clicks the target, then ordinary keyboard
  // text enters the mounted textarea immediately.
  await (await target()).click();
  const pointer = await activeEditorReceipt("pointer");
  await (await browser.$(`[data-block-id="${pointer.id}"] textarea`)).addValue("native trailing text");
  await browser.keys("Tab");
  const persisted = await waitForFile("native trailing text", "pointer text");
  if ((persisted.match(/native trailing text/g) || []).length !== 1) throw new Error(`pointer text duplicated: ${JSON.stringify(persisted)}`);

  await browser.refresh();
  await browser.$(".page-title").waitForExist({ timeout: 15_000 });
  await openPage(PAGE);
  const renderedCopies = await browser.execute(() => (document.body.textContent?.match(/native trailing text/g) || []).length);
  if (renderedCopies !== 1) throw new Error("reloaded page did not render exactly one persisted text copy");

  // The first activation after typed content creates one blank root; the next
  // must reuse it rather than stacking another blank bullet.
  await (await target()).click();
  const blank = await activeEditorReceipt("new-empty-tail");
  await browser.keys("Tab");
  const rootsBeforeReuse = await browser.$$('[data-block-id]');
  await (await target()).click();
  const reused = await activeEditorReceipt("reused-empty-tail");
  const rootsAfterReuse = await browser.$$('[data-block-id]');
  if (reused.id !== blank.id || rootsAfterReuse.length !== rootsBeforeReuse.length) throw new Error("existing empty tail was not reused exactly once");
  await browser.keys("Tab");

  // Keyboard accessibility is a separate native path: focus the actual target,
  // then activate with Enter and Space, each requiring the same focused caret.
  const enterTarget = await target();
  await browser.execute((element) => element.focus(), enterTarget);
  await browser.keys("Enter");
  await activeEditorReceipt("keyboard-enter");
  await browser.keys("Tab");
  const spaceTarget = await target();
  await browser.execute((element) => element.focus(), spaceTarget);
  await browser.keys(" ");
  await activeEditorReceipt("keyboard-space");
  await browser.keys("Tab");

  fs.writeFileSync(path.join(ARTIFACTS, "persisted-page.md"), fs.readFileSync(PAGE_FILE, "utf8"));
  console.log("PASS: routed page trailing target supports pointer, immediate native typing, persistence, reuse, Enter, and Space");
} catch (error) {
  try { await browser?.saveScreenshot(path.join(ARTIFACTS, "failure.png")); } catch {}
  try { fs.writeFileSync(path.join(ARTIFACTS, "persisted-page.md"), fs.readFileSync(PAGE_FILE, "utf8")); } catch {}
  console.error(`E2E ERROR: ${String(error).split("\n").slice(0, 5).join(" | ")}`);
  process.exitCode = 1;
} finally {
  try { await browser?.deleteSession(); } catch {}
  if (process.platform === "win32") {
    spawnSync("taskkill", ["/PID", String(driver.pid), "/T", "/F"], { stdio: "ignore" });
  } else {
    try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  }
  fs.closeSync(driverLog);
}
