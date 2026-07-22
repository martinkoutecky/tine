// Linux real-app proof for GH #58: dispatch the browser's real ClipboardEvent
// and DataTransfer HTML/plain flavors, then verify the persisted graph outline.
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const TD = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin", "tauri-driver") : "tauri-driver");
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4474);
const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4475);
const TMP = "/tmp/tine-structured-paste-e2e";
const GRAPH = `${TMP}/graph`;
const PAGE = `${GRAPH}/pages/Paste.md`;

fs.rmSync(TMP, { recursive: true, force: true });
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(`${GRAPH}/${dir}`, { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(`${TMP}/xdg/${dir}`, { recursive: true });
fs.writeFileSync(`${GRAPH}/logseq/config.edn`, "{}\n");
fs.writeFileSync(PAGE, "- paste here\n");
const now = new Date();
const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(`${GRAPH}/journals/${journal}.md`, "- open [[Paste]]\n");

const env = {
  ...process.env,
  TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: `${TMP}/xdg/data`, XDG_CONFIG_HOME: `${TMP}/xdg/config`, XDG_CACHE_HOME: `${TMP}/xdg/cache`,
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11",
};
const log = fs.openSync(`${TMP}/tauri-driver.log`, "w");
const td = spawn(TD, ["--port", String(DRIVER_PORT), "--native-port", String(NATIVE_PORT), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], {
  env, stdio: ["ignore", log, log], detached: true,
});
await sleep(2500);

let browser;
try {
  browser = await remote({
    hostname: "127.0.0.1", port: DRIVER_PORT, path: "/", logLevel: "error", connectionRetryCount: 1, connectionRetryTimeout: 60_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } },
  });
  await browser.$(".ls-block, .page-title").waitForExist({ timeout: 20_000 });
  for (const selector of ["a.page-ref=Paste", "span.page-ref=Paste", "*=Paste"]) {
    const link = await browser.$(selector);
    if (await link.isExisting()) { await link.click(); break; }
  }
  await browser.waitUntil(async () => (await browser.$("h1.page-title").getText()).trim() === "Paste", {
    timeout: 10_000, timeoutMsg: "Paste page did not open",
  });
  const target = await browser.$(".ls-block .block-content");
  await target.click();
  await browser.$("textarea.block-editor").waitForExist({ timeout: 5000 });
  const result = await browser.execute(() => {
    const editor = document.querySelector("textarea.block-editor");
    if (!editor) return { ok: false, error: "no editor" };
    if (typeof DataTransfer !== "function" || typeof ClipboardEvent !== "function") {
      return { ok: false, error: "browser clipboard constructors unavailable" };
    }
    editor.value = "";
    editor.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "deleteContent", data: null }));
    const clipboard = new DataTransfer();
    clipboard.setData("text/plain", "Parent\nChild bold\nSibling");
    clipboard.setData("text/html", "<ul><li>Parent<ul><li>Child <strong>bold</strong></li></ul></li><li>Sibling</li></ul>");
    const event = new ClipboardEvent("paste", { bubbles: true, cancelable: true, clipboardData: clipboard });
    editor.dispatchEvent(event);
    return { ok: event.defaultPrevented, types: [...clipboard.types] };
  });
  if (!result.ok || !result.types.includes("text/html") || !result.types.includes("text/plain")) {
    throw new Error(`structured clipboard was not handled: ${JSON.stringify(result)}`);
  }
  await sleep(600);
  await browser.keys(["Escape"]);
  await sleep(1800);
  const saved = fs.readFileSync(PAGE, "utf8");
  if (!/^- Parent\n\s+- Child \*\*bold\*\*\n- Sibling\s*$/m.test(saved)) {
    throw new Error(`wrong persisted structured paste:\n${saved}`);
  }
  console.log("PASS: real ClipboardEvent preserved nested HTML and persisted one atomic outline");
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-td.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
