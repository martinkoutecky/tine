// Real-app observation-boundary regression for GH #167. Drive the actual
// Tauri/WebKit editor through the browser composition/input event sequence and
// require the visible hashtag picker to find an existing CJK page.
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const TD = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin", "tauri-driver") : "tauri-driver");
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4570);
const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4571);
const TMP = "/tmp/tine-tag-autocomplete-e2e";
const GRAPH = `${TMP}/graph`;
const ARTIFACTS = process.env.E2E_ARTIFACT_DIR || TMP;

fs.rmSync(TMP, { recursive: true, force: true });
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(`${GRAPH}/${dir}`, { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(`${TMP}/xdg/${dir}`, { recursive: true });
fs.mkdirSync(ARTIFACTS, { recursive: true });
fs.writeFileSync(`${GRAPH}/logseq/config.edn`, "{}\n");
fs.writeFileSync(`${GRAPH}/pages/倘若.md`, "- Existing CJK tag page\n");
const now = new Date();
const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(`${GRAPH}/journals/${journal}.md`, "- Compose a tag here\n");

const env = {
  ...process.env,
  TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: `${TMP}/xdg/data`,
  XDG_CONFIG_HOME: `${TMP}/xdg/config`,
  XDG_CACHE_HOME: `${TMP}/xdg/cache`,
  WEBKIT_DISABLE_DMABUF_RENDERER: "1",
  WEBKIT_DISABLE_COMPOSITING_MODE: "1",
  LIBGL_ALWAYS_SOFTWARE: "1",
  GDK_BACKEND: "x11",
};
const log = fs.openSync(`${TMP}/tauri-driver.log`, "w");
const td = spawn(TD, [
  "--port", String(DRIVER_PORT),
  "--native-port", String(NATIVE_PORT),
  "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver",
], { env, stdio: ["ignore", log, log], detached: true });
await sleep(2500);

let browser;
async function setComposedValue(value, data) {
  const applied = await browser.execute((next, committed) => {
    const editor = document.querySelector("textarea.block-editor");
    if (!(editor instanceof HTMLTextAreaElement)) return false;
    editor.focus();
    editor.value = next;
    editor.setSelectionRange(next.length, next.length);
    editor.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true, data: "" }));
    editor.dispatchEvent(new InputEvent("input", {
      bubbles: true,
      inputType: "insertCompositionText",
      data: committed,
      isComposing: true,
    }));
    editor.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true, data: committed }));
    return true;
  }, value, data);
  if (!applied) throw new Error("block editor was not available for composition input");
}

async function autocompleteLabels() {
  return browser.execute(() => [...document.querySelectorAll(".autocomplete .ac-label")]
    .map((node) => node.textContent?.trim() || ""));
}

try {
  browser = await remote({
    hostname: "127.0.0.1",
    port: DRIVER_PORT,
    path: "/",
    logLevel: "error",
    connectionRetryCount: 1,
    connectionRetryTimeout: 60_000,
    capabilities: {
      browserName: "wry",
      "wdio:enforceWebDriverClassic": true,
      "tauri:options": { application: APP },
    },
  });
  await browser.$(".ls-block, .page-title").waitForExist({ timeout: 20_000 });
  await browser.$(".ls-block .block-content-wrapper").click();
  await browser.$("textarea.block-editor").waitForExist({ timeout: 5000 });

  await setComposedValue("#倘", "倘");
  await browser.waitUntil(async () => (await autocompleteLabels()).includes("#倘若"), {
    timeout: 10_000,
    timeoutMsg: "CJK IME hashtag input did not show the existing #倘若 choice",
  });

  const punctuationClosed = await browser.execute(() => {
    const editor = document.querySelector("textarea.block-editor");
    if (!(editor instanceof HTMLTextAreaElement)) return false;
    editor.value = "#倘,";
    editor.setSelectionRange(editor.value.length, editor.value.length);
    editor.dispatchEvent(new InputEvent("input", {
      bubbles: true, inputType: "insertText", data: ",", isComposing: false,
    }));
    return true;
  });
  if (!punctuationClosed) throw new Error("block editor disappeared before hard-stop coverage");
  await browser.waitUntil(async () => (await autocompleteLabels()).length === 0, {
    timeout: 3000,
    timeoutMsg: "hard-stop punctuation did not close hashtag autocomplete",
  });

  await setComposedValue("[[倘", "倘");
  await browser.waitUntil(async () => (await autocompleteLabels()).includes("倘若"), {
    timeout: 10_000,
    timeoutMsg: "page-link CJK composition sibling lost its existing autocomplete behavior",
  });
  await browser.saveScreenshot(path.join(ARTIFACTS, "tag-autocomplete-cjk-ime.png"));
  console.log("PASS: CJK composition opens hashtag and page-link autocomplete and hard stops still close it");
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-td.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
