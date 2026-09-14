// Real Tauri/WebDriver proof for GH #412, #413, and #415.
// User-facing state changes use literal WebDriver pointer/keyboard input. DOM
// reads only produce receipts; they never synthesize an editor event.
import { spawn } from "node:child_process";
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
import { ensureDisplay } from "./lib/e2e-display.mjs";

await ensureDisplay();

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const DRIVER = Number(process.env.E2E_DRIVER_PORT || 4628);
const NATIVE = Number(process.env.E2E_NATIVE_PORT || 4629);
const TMP = path.join(os.tmpdir(), `tine-editor-interactions-e2e-${process.pid}`);
const GRAPH = path.join(TMP, "graph");
const PAGE = "Editor interaction proof";
const PAGE_FILE = path.join(GRAPH, "pages", `${PAGE}.md`);
const TARGET = "91919191-9191-4191-8191-919191919191";
const ARTIFACTS = process.env.E2E_ARTIFACT_DIR || TMP;

fs.rmSync(TMP, { recursive: true, force: true });
for (const directory of ["pages", "journals", "logseq", "assets"]) {
  fs.mkdirSync(path.join(GRAPH, directory), { recursive: true });
}
for (const directory of ["data", "config", "cache"]) {
  fs.mkdirSync(path.join(TMP, "xdg", directory), { recursive: true });
}
fs.mkdirSync(ARTIFACTS, { recursive: true });
fs.writeFileSync(path.join(GRAPH, "logseq", "config.edn"), "{}\n");
fs.writeFileSync(PAGE_FILE, [
  "- Above embed host",
  `- {{embed ((${TARGET}))}}`,
  "- Embedded source",
  `  id:: ${TARGET}`,
  "- ```js",
  "  echo hello",
  "  ```",
  "- Scaffold target",
  "- " + "Multiline drag probe has ordinary words which wrap naturally across several visual lines. ".repeat(12),
  "",
].join("\n"));
const now = new Date();
const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(path.join(GRAPH, "journals", `${journal}.md`), `- open [[${PAGE}]]\n`);

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

const webviewTarget = await startWebdriverApplication(APP, env, NATIVE);
const driverLog = fs.openSync(path.join(ARTIFACTS, "tauri-driver.log"), "w");
const driver = spawn(
  process.env.TAURI_DRIVER || "tauri-driver",
  webdriverServerArgs(DRIVER, NATIVE, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"),
  { env: webviewTarget.env, stdio: ["ignore", driverLog, driverLog], detached: true },
);

let browser;

async function openProofPage() {
  await browser.$(".ls-block, .page-title").waitForExist({ timeout: 20_000 });
  let opened = false;
  for (const selector of [`a.page-ref=${PAGE}`, `span.page-ref=${PAGE}`, `*=${PAGE}`]) {
    const link = await browser.$(selector);
    if (!await link.isExisting()) continue;
    await link.click();
    opened = true;
    break;
  }
  if (!opened) {
    const state = await browser.execute(() => ({
      title: document.querySelector("h1.page-title")?.textContent?.trim() ?? null,
      pageRefs: [...document.querySelectorAll(".page-ref")].map((node) => node.textContent?.trim()),
      blocks: [...document.querySelectorAll(".ls-block")].map((node) => node.textContent?.trim()).slice(0, 10),
    }));
    fs.writeFileSync(path.join(ARTIFACTS, "navigation-failure.json"), `${JSON.stringify(state, null, 2)}\n`);
    throw new Error(`could not find the proof-page reference: ${JSON.stringify(state)}`);
  }
  await browser.waitUntil(async () => (await browser.$("h1.page-title").getText()).trim() === PAGE, {
    timeout: 10_000,
    timeoutMsg: `${PAGE} did not open`,
  });
}

async function blockContentWithText(text) {
  const blocks = await browser.$$(".page-blocks .ls-block");
  for (const block of blocks) {
    const content = await block.$(":scope > .block-main .block-content");
    if (await content.isExisting() && (await content.getText()).includes(text)) return content;
  }
  throw new Error(`could not find rendered block containing ${JSON.stringify(text)}`);
}

async function activeEditorReceipt(label) {
  const receipt = await browser.execute(() => {
    const active = document.activeElement;
    return {
      active: active instanceof HTMLTextAreaElement && active.classList.contains("block-editor"),
      value: active instanceof HTMLTextAreaElement ? active.value : null,
      selectionStart: active instanceof HTMLTextAreaElement ? active.selectionStart : null,
      selectionEnd: active instanceof HTMLTextAreaElement ? active.selectionEnd : null,
      inEmbed: Boolean(active?.closest?.(".embed-block")),
      rowText: active?.closest?.(".ls-block")?.querySelector?.(":scope > .block-main")?.textContent?.trim() ?? null,
    };
  });
  fs.writeFileSync(path.join(ARTIFACTS, `${label}.json`), `${JSON.stringify(receipt, null, 2)}\n`);
  return receipt;
}

async function proveEmbedExit() {
  const embedded = await browser.$(`.block-embed-host .embed-block [data-block-ref="${TARGET}"] > .block-main .block-content`);
  await embedded.waitForExist({ timeout: 15_000 });
  await embedded.click();
  const editor = await browser.$(`.block-embed-host .embed-block [data-block-ref="${TARGET}"] textarea.block-editor`);
  await editor.waitForExist({ timeout: 5_000 });
  await browser.keys(["Home"]);
  await browser.keys(["ArrowUp"]);
  await browser.waitUntil(async () => {
    const receipt = await activeEditorReceipt("415-embed-exit");
    return receipt.active && !receipt.inEmbed && receipt.value === "Above embed host";
  }, { timeout: 5_000, timeoutMsg: "ArrowUp did not exit the embed root to the preceding host-page block" });
}

async function provePayloadOnlySelection() {
  const code = await browser.$(".page-blocks .code-block");
  await code.waitForExist({ timeout: 5_000 });
  await code.click();
  const editor = await browser.$(".page-blocks textarea.block-editor.code-edit");
  await editor.waitForExist({ timeout: 5_000 });
  const initial = await activeEditorReceipt("412-code-body-initial");
  if (initial.value !== "echo hello" || initial.value.includes("```")) {
    throw new Error(`complete code wrapper did not expose only its payload: ${JSON.stringify(initial)}`);
  }
  await browser.keys(["Control", "a"]);
  const selected = await activeEditorReceipt("412-code-body-selected");
  if (selected.selectionStart !== 0 || selected.selectionEnd !== selected.value.length || selected.value.includes("```")) {
    throw new Error(`native first Ctrl+A did not select exactly the payload: ${JSON.stringify(selected)}`);
  }
  // Use the WebDriver element text-input endpoint for ordinary text. The
  // browser-level `keys` helper intentionally presses every supplied character
  // before releasing any of them, which is correct for chords but not prose.
  await editor.addValue("native payload");
  await browser.keys(["Escape"]);
  await browser.waitUntil(() => {
    const bytes = fs.readFileSync(PAGE_FILE, "utf8");
    return bytes.includes("native payload") && !bytes.includes("echo hello");
  }, { timeout: 10_000, timeoutMsg: "payload-only code edit did not persist" });
  const bytes = fs.readFileSync(PAGE_FILE, "utf8");
  if ((bytes.match(/```/g) ?? []).length !== 2) {
    throw new Error(`payload edit did not preserve exactly one opening/closing wrapper: ${JSON.stringify(bytes)}`);
  }
}

async function proveBacktickScaffold() {
  const target = await blockContentWithText("Scaffold target");
  await target.click();
  const editor = await browser.$(".page-blocks textarea.block-editor");
  await editor.waitForExist({ timeout: 5_000 });
  await browser.keys(["Control", "a"]);
  await browser.keys(["Backspace"]);
  // Separate native key commands retain the repeated-key boundaries that the
  // generic symmetric-backtick pairer and the third-key scaffold decision use.
  await browser.keys(["`"]);
  await browser.keys(["`"]);
  await browser.keys(["`"]);
  await browser.waitUntil(async () => {
    const receipt = await activeEditorReceipt("413-backtick-scaffold");
    return receipt.active && !receipt.value.includes("```") && receipt.selectionStart === 0 && receipt.selectionEnd === 0;
  }, { timeout: 5_000, timeoutMsg: "third backtick did not enter the body-only scaffold interior" });
  await browser.keys(["Escape"]);
  await browser.waitUntil(() => (fs.readFileSync(PAGE_FILE, "utf8").match(/```/g) ?? []).length === 4, {
    timeout: 10_000,
    timeoutMsg: "complete backtick scaffold did not persist beside the existing wrapper",
  });
  const bytes = fs.readFileSync(PAGE_FILE, "utf8");
  if (bytes.includes("Scaffold target") || bytes.includes("````")) {
    throw new Error(`scaffold persisted stale text or a four-backtick run: ${JSON.stringify(bytes)}`);
  }
}


async function proveMultilineDrag() {
  for (const row of [4, 6]) {
    const content = await blockContentWithText("Multiline drag probe");
    await content.scrollIntoView({ block: "start" });
    const rect = await browser.execute((element) => {
      const r = element.getBoundingClientRect(); return {x:r.x,y:r.y};
    }, content);
    const pointer = (actions) => browser.performActions([{type:"pointer",id:"drag-proof",parameters:{pointerType:"mouse"},actions}]);
    await pointer([{type:"pointerMove",origin:"viewport",x:Math.round(rect.x+60),y:Math.round(rect.y+12),duration:0},{type:"pointerDown",button:0}]);
    await browser.$("textarea.block-editor").waitForExist({timeout:5000});
    const target = await browser.execute((row) => {
      const ta = document.querySelector("textarea.block-editor");
      const r = ta.getBoundingClientRect(), cs = getComputedStyle(ta);
      return {x:Math.round(r.left+70),y:Math.round(r.top+parseFloat(cs.paddingTop)+parseFloat(cs.borderTopWidth)+parseFloat(cs.lineHeight)*(row+.5)),anchor:ta.selectionStart};
    }, row);
    await pointer([{type:"pointerMove",origin:"viewport",x:target.x,y:target.y,duration:250},{type:"pointerUp",button:0}]);
    const drag = await activeEditorReceipt(`drag-row-${row}`);
    // Independent oracle: a real native click in the now-active textarea at
    // the same pixel. No copied mirror algorithm or synthetic event dispatch.
    await pointer([{type:"pointerDown",button:0},{type:"pointerUp",button:0}]);
    const native = await activeEditorReceipt(`native-caret-row-${row}`);
    if (!drag.active || drag.selectionStart !== target.anchor || drag.selectionEnd !== native.selectionStart) {
      throw new Error(`wrapped drag row ${row} diverged from native pointer: ${JSON.stringify({target,drag,native})}`);
    }
    await browser.keys(["Escape"]);
  }
}

try {
  await sleep(2500);
  browser = await remote({
    hostname: "127.0.0.1",
    port: DRIVER,
    path: "/",
    logLevel: "error",
    connectionRetryCount: 1,
    connectionRetryTimeout: 60_000,
    capabilities: tauriCapabilities(APP, "editor-interactions", process.platform, webviewTarget.debuggerAddress),
  });
  await openProofPage();
  await proveEmbedExit();
  await provePayloadOnlySelection();
  await proveBacktickScaffold();
  await proveMultilineDrag();
  await browser.saveScreenshot(path.join(ARTIFACTS, "final.png"));
  fs.writeFileSync(path.join(ARTIFACTS, "persisted-page.md"), fs.readFileSync(PAGE_FILE, "utf8"));
  console.log("PASS: native code payload selection, three-backtick scaffold, embed-root ArrowUp exit, and wrapped drag endpoints all held in the real app");
} catch (error) {
  try { await browser?.saveScreenshot(path.join(ARTIFACTS, "failure.png")); } catch {}
  try { fs.writeFileSync(path.join(ARTIFACTS, "persisted-page.md"), fs.readFileSync(PAGE_FILE, "utf8")); } catch {}
  console.error(`E2E ERROR: ${String(error).split("\n").slice(0, 6).join(" | ")}`);
  process.exitCode = 1;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  stopWebdriverApplication(webviewTarget);
  fs.closeSync(driverLog);
}
