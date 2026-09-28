// Linux real-app proof for GH #58: dispatch the browser's real ClipboardEvent
// and DataTransfer HTML/plain flavors, then verify the persisted graph outline.
// Also og batch 7 (block Cut/Copy): keyboard Copy and Cut of a selected block
// through the native clipboard, the cut subtree leaving its source only once the
// clipboard holds it, and a keyboard paste reproducing the nested outline.
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { nativeDialogWindows } from "./lib/e2e-native-dialog.mjs";
import { execFileSync } from "node:child_process";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const TD = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin", "tauri-driver") : "tauri-driver");
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4474);
const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4475);
const TMP = process.env.E2E_TMP_DIR || `/tmp/tine-structured-paste-e2e-${process.pid}`;
const ARTIFACTS = process.env.E2E_ARTIFACT_DIR || TMP;

const GRAPH = `${TMP}/graph`;
const PAGE = `${GRAPH}/pages/Paste.md`;
const CUT_SOURCE = `${GRAPH}/pages/Cut Source.md`;
const CUT_TARGET = `${GRAPH}/pages/Cut Target.md`;

fs.rmSync(TMP, { recursive: true, force: true });
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(`${GRAPH}/${dir}`, { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(`${TMP}/xdg/${dir}`, { recursive: true });
fs.writeFileSync(`${GRAPH}/logseq/config.edn`, "{}\n");
fs.writeFileSync(PAGE, "- paste here\n");
const CUT_SOURCE_BYTES = "- keep me\n- cut parent\n  - cut child\n- copy me\n";
fs.writeFileSync(CUT_SOURCE, CUT_SOURCE_BYTES);
fs.writeFileSync(CUT_TARGET, "- target first\n");
const now = new Date();
const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(`${GRAPH}/journals/${journal}.md`, "- open [[Paste]]\n");

const env = {
  ...process.env,
  TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: `${TMP}/xdg/data`, XDG_CONFIG_HOME: `${TMP}/xdg/config`, XDG_CACHE_HOME: `${TMP}/xdg/cache`,
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11",
};
fs.mkdirSync(ARTIFACTS, { recursive: true });
const log = fs.openSync(path.join(ARTIFACTS, "structured-paste-tauri-driver.log"), "w");
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
  await openPageByName(browser, "Paste");
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

  // ---- og batch 7: keyboard Copy / Cut of selected blocks -----------------
  const nativeClipboard = async (command, text) => {
    const out = await browser.executeAsync((cmd, value, done) => {
      const timer = setTimeout(() => done({ ok: false, error: "no answer within 2s" }), 2000);
      globalThis.__TAURI_INTERNALS__.invoke(`plugin:clipboard-manager|${cmd}`, value === null ? {} : { text: value })
        .then((value) => { clearTimeout(timer); done({ ok: true, value }); },
          (error) => { clearTimeout(timer); done({ ok: false, error: String(error) }); });
    }, command, text ?? null);
    if (!out.ok) throw new Error(`native clipboard ${command} failed: ${out.error}`);
    return out.value;
  };
  const read = (file) => fs.readFileSync(file, "utf8");
  // Select one block the way a user does: edit it, then Escape to block selection.
  const selectBlock = async (text) => {
    const content = await browser.$(`//div[contains(concat(' ', normalize-space(@class), ' '), ' block-content ') and normalize-space(.) = '${text}']`);
    await content.waitForExist({ timeout: 10_000, timeoutMsg: `block ${JSON.stringify(text)} is not rendered` });
    await content.click();
    await browser.$("textarea.block-editor").waitForExist({ timeout: 5000 });
    await browser.keys(["Escape"]);
    await browser.waitUntil(() => browser.execute((wanted) => {
      const selected = [...document.querySelectorAll(".block-main.selected")];
      return selected.length === 1 && !document.querySelector("textarea.block-editor")
        && selected[0].querySelector(".block-content")?.textContent?.trim() === wanted;
    }, text), { timeout: 5000, interval: 100, timeoutMsg: `Escape did not leave exactly ${JSON.stringify(text)} selected` });
  };
  // Poll the native clipboard. Each read is bounded on both sides, so a read
  // that never answers is reported as such instead of stalling the journey.
  const waitClipboard = async (predicate, what) => {
    const seen = [];
    const deadline = Date.now() + 10_000;
    for (;;) {
      const started = Date.now();
      let value;
      try {
        value = await Promise.race([
          nativeClipboard("read_text"),
          sleep(3000).then(() => { throw new Error("WebDriver round trip did not return within 3s"); }),
        ]);
      } catch (error) { value = `<${error.message}>`; }
      const entry = JSON.stringify(value);
      if (seen.at(-1) !== entry) seen.push(entry);
      if (Date.now() - started > 1000) seen.push(`slow read ${Date.now() - started}ms`);
      if (predicate(value)) return value;
      if (Date.now() > deadline) throw new Error(`${what}; native clipboard readings ${JSON.stringify(seen)}`);
      await sleep(100);
    }
  };

  await openPageByName(browser, "Cut Source");
  await browser.waitUntil(() => read(CUT_SOURCE) === CUT_SOURCE_BYTES, { timeout: 5000, timeoutMsg: "Cut Source changed on open" });

  // Copy: the clipboard receives the block; the source is untouched.
  await nativeClipboard("write_text", "structured-paste sentinel");
  await selectBlock("copy me");
  await browser.keys(["Control", "c"]);
  await browser.keys(["Control"]);
  await waitClipboard((text) => typeof text === "string" && /copy me/.test(text), "keyboard Copy did not reach the native clipboard");
  await sleep(500);
  if (read(CUT_SOURCE) !== CUT_SOURCE_BYTES) throw new Error(`Copy changed its source:\n${read(CUT_SOURCE)}`);
  await browser.keys(["Escape"]);

  // Cut: the clipboard receives the parent AND its child, then (only then) the
  // subtree leaves the source while its siblings stay byte-identical.
  await selectBlock("cut parent");
  await browser.keys(["Control", "x"]);
  await browser.keys(["Control"]);
  // Tine's default copy setting puts only the SELECTED blocks in the public
  // text (selection.ts, copyIncludeSubtree); the subtree travels in Tine's
  // private clipboard payload, which the paste below must honour.
  const cutText = await waitClipboard((text) => typeof text === "string" && /cut parent/.test(text),
    "keyboard Cut did not put the selected block on the native clipboard");
  await browser.waitUntil(() => read(CUT_SOURCE) === "- keep me\n- copy me\n", {
    timeout: 10_000, timeoutMsg: `Cut did not remove exactly the cut subtree; source is ${JSON.stringify(read(CUT_SOURCE))}`,
  });

  // Paste into another page with the keyboard: the nested outline arrives.
  await openPageByName(browser, "Cut Target");
  const first = await browser.$(`//div[contains(concat(' ', normalize-space(@class), ' '), ' block-content ') and normalize-space(.) = 'target first']`);
  await first.waitForExist({ timeout: 10_000 });
  await first.click();
  await browser.$("textarea.block-editor").waitForExist({ timeout: 5000 });
  await browser.keys(["End"]);
  await browser.keys(["Enter"]);
  await browser.waitUntil(() => browser.execute(() => document.querySelector("textarea.block-editor")?.value === ""), {
    timeout: 5000, timeoutMsg: "Enter did not open an empty sibling editor",
  });
  await browser.keys(["Control", "v"]);
  await browser.keys(["Control"]);
  await sleep(600);
  await browser.keys(["Escape"]);
  await browser.waitUntil(() => /^- target first\n- cut parent\n\s+- cut child\n?$/.test(read(CUT_TARGET)), {
    timeout: 10_000, timeoutMsg: `keyboard paste did not reproduce the cut outline; clipboard ${JSON.stringify(cutText)}, target ${JSON.stringify(read(CUT_TARGET))}`,
  });
  if (read(CUT_SOURCE) !== "- keep me\n- copy me\n") throw new Error(`paste disturbed the cut source:\n${read(CUT_SOURCE)}`);
  console.log("PASS: keyboard Copy kept its source; keyboard Cut moved the nested subtree through the native clipboard");
} catch (error) {
  for (const id of nativeDialogWindows()) {
    let name = "";
    try { name = execFileSync("xdotool", ["getwindowname", id], { encoding: "utf8" }).trim(); } catch {}
    console.error(`native dialog open: ${id} ${JSON.stringify(name)}`);
  }
  try { await browser?.saveScreenshot(path.join(ARTIFACTS, "structured-paste-failure.png")); } catch {}
  for (const file of [PAGE, CUT_SOURCE, CUT_TARGET]) {
    try { console.error(`${path.basename(file)}:`, JSON.stringify(fs.readFileSync(file, "utf8"))); } catch {}
  }
  try { console.error("toasts:", JSON.stringify(await browser.execute(() => [...document.querySelectorAll(".toast")].map((node) => node.textContent)))); } catch {}
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-td.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
