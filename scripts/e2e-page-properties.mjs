// Real Tauri/WebKit and disk round-trip for GH #163. The literal reporter
// samples travel through page load -> gear panel -> reactive store -> debounced
// native save; helper-only string tests cannot prove this boundary.
import { spawn, spawnSync } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { tauriCapabilities } from "./e2e-capabilities.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, process.platform === "win32" ? "target/release/tine.exe" : "target/release/tine");
const TD = process.env.TAURI_DRIVER || "tauri-driver";
const DRIVER = Number(process.env.E2E_DRIVER_PORT || 4592);
const NATIVE = Number(process.env.E2E_NATIVE_PORT || 4593);
const TMP = path.join(os.tmpdir(), `tine-page-properties-e2e-${process.pid}`);
const GRAPH = path.join(TMP, "graph");

fs.rmSync(TMP, { recursive: true, force: true });
for (const dir of ["pages", "journals", "logseq", "assets"]) fs.mkdirSync(`${GRAPH}/${dir}`, { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(`${TMP}/xdg/${dir}`, { recursive: true });
fs.writeFileSync(`${GRAPH}/logseq/config.edn`, "{}\n");
const detailed = [
  "alias:: Test Record",
  "ai-prompt:: [[Prompt-Test]]",
  "usage-frequency:: [[Frequency-High]]",
  "",
  "page-level:: [[Level-Two]]",
  "layout:: [[Layout-Top-Collapsed]]",
  "component-state:: [[Component-Wide]]",
  "",
  "timestamp:: 20250707092601",
  "observation-target:: [[Object-Test-Page]]",
  "external-impact::",
  "--:: --",
  "methods:: [[Method-A]] [[Method-B]]",
  "key-conclusion:: [[Conclusion-A]] [[Conclusion-B]]",
  "",
  "- Example content block",
  "",
].join("\n");
const simple = "A:: XX\r\nB:: XX\r\nC:: XX\r\n";
fs.writeFileSync(`${GRAPH}/pages/Property detailed.md`, detailed);
fs.writeFileSync(`${GRAPH}/pages/Property simple.md`, simple);
const now = new Date();
const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(`${GRAPH}/journals/${journal}.md`, "- [[Property detailed]]\n- [[Property simple]]\n");

const env = {
  ...process.env,
  TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: `${TMP}/xdg/data`,
  XDG_CONFIG_HOME: `${TMP}/xdg/config`,
  XDG_CACHE_HOME: `${TMP}/xdg/cache`,
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
const log = fs.openSync(path.join(process.env.E2E_ARTIFACT_DIR || TMP, "tauri-driver.log"), "w");
const driverArgs = process.platform === "win32"
  ? ["--port", String(DRIVER)]
  : ["--port", String(DRIVER), "--native-port", String(NATIVE), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"];
const driver = spawn(TD, driverArgs, {
  env,
  stdio: ["ignore", log, log],
  detached: process.platform !== "win32",
});
await sleep(2500);
let browser;

async function openPage(name) {
  if ((await browser.$$(".nav-page")).length === 0) {
    const toggled = await browser.execute(() => {
      const header = [...document.querySelectorAll(".nav-section-header")]
        .find((element) => element.textContent?.includes("ALL PAGES"));
      if (!header) return false;
      header.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, button: 0 }));
      return true;
    });
    if (!toggled) throw new Error("missing ALL PAGES sidebar section");
    await browser.waitUntil(async () => (await browser.$$(".nav-page")).length >= 2, {
      timeout: 5_000,
      timeoutMsg: "ALL PAGES did not reveal fixture pages",
    });
  }
  const opened = await browser.execute((target) => {
    const rows = [...document.querySelectorAll(".nav-page")];
    const row = rows.find((element) => element.textContent?.trim() === target);
    if (!row) return { ok: false, rows: rows.map((element) => element.textContent?.trim()) };
    row.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, button: 0 }));
    return { ok: true, rows: [] };
  }, name);
  if (!opened.ok) {
    const context = await browser.execute(() => ({
      title: document.querySelector("h1.page-title")?.textContent?.trim(),
      body: document.body.textContent?.trim().slice(0, 1_000),
    }));
    throw new Error(`missing ALL PAGES result ${name}: ${JSON.stringify({ rows: opened.rows, context })}`);
  }
  await browser.waitUntil(async () => (await browser.$("h1.page-title").getText()).trim() === name, {
    timeout: 10_000,
    timeoutMsg: `could not open ${name}`,
  });
}

async function setGearField(label, value) {
  await browser.$(".page-gear").click();
  await browser.$(".page-props-panel").waitForExist({ timeout: 5_000 });
  const index = await browser.execute((wanted) => {
    const fields = [...document.querySelectorAll(".page-props-panel .pp-field")];
    return fields.findIndex((field) => field.querySelector(".pp-label")?.textContent?.trim() === wanted);
  }, label);
  if (index < 0) throw new Error(`missing page property field ${label}`);
  const input = (await browser.$$(".page-props-panel .pp-field .pp-input"))[index];
  await input.setValue(value);
  await browser.keys("Enter");
  await browser.$(".page-props-panel").waitForExist({ reverse: true, timeout: 5_000 });
}

async function waitForFile(file, predicate, label) {
  const deadline = Date.now() + 10_000;
  while (Date.now() < deadline) {
    const text = fs.readFileSync(file, "utf8");
    if (predicate(text)) return text;
    await sleep(100);
  }
  throw new Error(`${label} was not persisted: ${fs.readFileSync(file, "utf8")}`);
}

async function nativeTab({ shift = false } = {}) {
  const actions = shift
    ? [{ type: "keyDown", value: "\uE008" }, { type: "keyDown", value: "\uE004" }, { type: "keyUp", value: "\uE004" }, { type: "keyUp", value: "\uE008" }]
    : [{ type: "keyDown", value: "\uE004" }, { type: "keyUp", value: "\uE004" }];
  await browser.performActions([{ type: "key", id: "page-properties-keyboard", actions }]);
  await browser.releaseActions();
}

async function activePagePropertyControl() {
  return browser.execute(() => {
    const panel = document.querySelector(".page-props-panel");
    const active = document.activeElement;
    if (!panel || !active || !panel.contains(active)) return null;
    if (active.classList.contains("pp-input")) {
      return active.closest(".pp-field")?.querySelector(".pp-label")?.textContent?.trim() ?? null;
    }
    if (active.matches(".pp-bool input")) return active.closest(".pp-field")?.querySelector(".pp-label")?.textContent?.trim() ?? null;
    return active.classList.contains("pp-done") ? "Done" : null;
  });
}

async function exerciseNativeFormTabTraversal(aliasValue) {
  // Pointer-open the actual routed named-page panel; the key actions below are
  // W3C native actions, not synthetic DOM events, so WebKit performs its focus
  // default action if the capture handler leaves it alone.
  await browser.$(".page-gear").click();
  await browser.$(".page-props-panel").waitForExist({ timeout: 5_000 });
  const aliases = (await browser.$$(".page-props-panel .pp-input"))[0];
  await aliases.click();
  await aliases.setValue(aliasValue);

  await nativeTab();
  if (await activePagePropertyControl() !== "Tags") {
    throw new Error(`native Tab did not leave Aliases for Tags; active=${JSON.stringify(await activePagePropertyControl())}`);
  }
  await waitForFile(
    `${GRAPH}/pages/Property detailed.md`,
    (text) => text.includes(`alias:: ${aliasValue}`),
    "Aliases blur commit after native Tab",
  );

  await nativeTab({ shift: true });
  if (await activePagePropertyControl() !== "Aliases") {
    throw new Error(`native Shift+Tab did not return to Aliases; active=${JSON.stringify(await activePagePropertyControl())}`);
  }

  for (const expected of ["Tags", "Display title", "Icon", "Public", "Done"]) {
    await nativeTab();
    if (await activePagePropertyControl() !== expected) {
      throw new Error(`native Tab focus order expected ${expected}; active=${JSON.stringify(await activePagePropertyControl())}`);
    }
  }
  await browser.$(".pp-done").click();
  await browser.$(".page-props-panel").waitForExist({ reverse: true, timeout: 5_000 });
}

try {
  browser = await remote({
    hostname: "127.0.0.1",
    port: DRIVER,
    path: "/",
    logLevel: "error",
    connectionRetryCount: 1,
    connectionRetryTimeout: 60_000,
    capabilities: tauriCapabilities(APP),
  });
  await browser.$(".page-title").waitForExist({ timeout: 20_000 });
  // The graph page index warms asynchronously after first paint. Wait for the
  // complete list before using Ctrl+K, otherwise a cold run offers only the
  // misleading "Create page" row for an already-existing file.
  await sleep(3500);

  await openPage("Property detailed");
  await exerciseNativeFormTabTraversal("Test Record, Alternate");
  const detailedAfter = await waitForFile(
    `${GRAPH}/pages/Property detailed.md`,
    (text) => text.includes("alias:: Test Record, Alternate"),
    "detailed page property edit",
  );
  const detailedExpected = detailed.replace("alias:: Test Record", "alias:: Test Record, Alternate");
  if (detailedAfter !== detailedExpected) {
    throw new Error(`detailed page changed outside the edited line\nEXPECTED:\n${detailedExpected}\nACTUAL:\n${detailedAfter}`);
  }

  await openPage("Property simple");
  await setGearField("Icon", "★");
  const simpleAfter = await waitForFile(
    `${GRAPH}/pages/Property simple.md`,
    (text) => text.includes("icon:: ★"),
    "simple page property edit",
  );
  const simpleExpected = "icon:: ★\r\nA:: XX\r\nB:: XX\r\nC:: XX\r\n";
  if (simpleAfter !== simpleExpected) {
    throw new Error(`simple CRLF page changed outside OG's prepended property line\nEXPECTED:\n${JSON.stringify(simpleExpected)}\nACTUAL:\n${JSON.stringify(simpleAfter)}`);
  }
  const lines = simpleAfter.trimEnd().split(/\r?\n/);
  if (lines.some((line) => /^\s*-\s/.test(line) || /^\s{2,}\S/.test(line))) {
    throw new Error(`header properties became outline content: ${JSON.stringify(lines)}`);
  }
  if (!lines.includes("A:: XX") || !lines.includes("B:: XX") || !lines.includes("C:: XX") || !lines.includes("icon:: ★")) {
    throw new Error(`simple page properties were lost or merged: ${JSON.stringify(lines)}`);
  }
  console.log("PASS: page-property native Tab traversal and literal reporter files survive the real Tauri/WebKit save path");
} finally {
  try { await browser?.deleteSession(); } catch {}
  if (process.platform === "win32") {
    spawnSync("taskkill", ["/PID", String(driver.pid), "/T", "/F"], { stdio: "ignore" });
    if (process.env.CI === "true") {
      spawnSync("taskkill", ["/IM", path.basename(APP), "/T", "/F"], { stdio: "ignore" });
    }
  } else try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
