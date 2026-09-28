// Real-app regressions for GH #154 and GH #166. Copy block ref assigns a fresh
// target's durable identity, a pasted reference updates its badge/panel before
// reload and survives reload, and loaded/unloaded source text follows landed
// transactions.
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const TD = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin", "tauri-driver") : "tauri-driver");
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4530);
const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4531);
const TMP = process.env.E2E_TMP_DIR || `/tmp/tine-block-ref-count-e2e-${process.pid}`;
const ARTIFACTS = process.env.E2E_ARTIFACT_DIR || TMP;
let step = "startup";
const t0 = Date.now();
const mark = (name) => { step = name; console.log(`[${((Date.now() - t0) / 1000).toFixed(1)}s] ${name}`); };
const GRAPH = `${TMP}/graph`;
const COLD_TARGET = "88888888-8888-4888-8888-888888888888";

fs.rmSync(TMP, { recursive: true, force: true });
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(`${GRAPH}/${dir}`, { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(`${TMP}/xdg/${dir}`, { recursive: true });
fs.writeFileSync(`${GRAPH}/logseq/config.edn`, "{}\n");
fs.writeFileSync(`${GRAPH}/pages/Source Target.md`, "- Seed block\n");
fs.writeFileSync(`${GRAPH}/pages/Cold Source.md`, `- Cold source\n  id:: ${COLD_TARGET}\n`);
fs.writeFileSync(`${GRAPH}/pages/Tester.md`, `- \n- ((${COLD_TARGET}))\n`);
const now = new Date();
const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(`${GRAPH}/journals/${journal}.md`, "- Open [[Source Target]]\n- Open [[Tester]]\n");

const env = {
  ...process.env,
  TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: `${TMP}/xdg/data`, XDG_CONFIG_HOME: `${TMP}/xdg/config`, XDG_CACHE_HOME: `${TMP}/xdg/cache`,
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11",
};
fs.mkdirSync(ARTIFACTS, { recursive: true });
const log = fs.openSync(path.join(ARTIFACTS, "block-ref-count-tauri-driver.log"), "w");
const td = spawn(TD, ["--port", String(DRIVER_PORT), "--native-port", String(NATIVE_PORT), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], {
  env, stdio: ["ignore", log, log], detached: true,
});
await sleep(2500);

let browser;
/** Tine's own native clipboard commands (clipboard-manager plugin), from the real webview. */
async function nativeClipboard(command, text) {
  const result = await browser.executeAsync((cmd, value, done) => {
    globalThis.__TAURI_INTERNALS__.invoke(`plugin:clipboard-manager|${cmd}`, value === null ? {} : { text: value })
      .then((out) => done({ ok: true, out }), (error) => done({ ok: false, error: String(error) }));
  }, command, text ?? null);
  if (!result.ok) throw new Error(`native clipboard ${command} failed: ${result.error}`);
  return result.out;
}
async function openPage(label) {
  for (const selector of [`a.page-ref=${label}`, `span.page-ref=${label}`, `*=${label}`]) {
    const link = await browser.$(selector);
    if (await link.isExisting()) {
      await link.click();
      await browser.waitUntil(async () => (await browser.$("h1.page-title").getText()).trim() === label, {
        timeout: 10_000, timeoutMsg: `${label} did not open`,
      });
      return;
    }
  }
  throw new Error(`no page link for ${label}`);
}

try {
  browser = await remote({
    hostname: "127.0.0.1", port: DRIVER_PORT, path: "/", logLevel: "error",
    connectionRetryCount: 1, connectionRetryTimeout: 60_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } },
  });
  await browser.$(".ls-block, .page-title").waitForExist({ timeout: 20_000 });

  mark("fresh target creation");
  // Force the initial empty count map to load while the source block is mounted.
  await openPage("Source Target");
  await sleep(1500);
  if (await browser.$(".block-refs-count").isExisting()) throw new Error("source badge existed before any reference");

  // Create the target through Tine so it has a real transient live key and no
  // durable property yet — the exact state that originally exposed GH #154.
  const addTarget = await browser.$(".page-trailing-block-target");
  await addTarget.waitForExist({ timeout: 5000 });
  await addTarget.click();
  const freshEditor = await browser.$(".page-blocks textarea.block-editor");
  await freshEditor.waitForExist({ timeout: 5000 });
  await freshEditor.setValue("Fresh source target");
  await browser.keys(["Escape"]);
  await browser.waitUntil(() => {
    const raw = fs.readFileSync(`${GRAPH}/pages/Source Target.md`, "utf8");
    return raw.includes("Fresh source target") && !/(?:^|\n)\s*id::/i.test(raw);
  }, {
    timeout: 10_000,
    timeoutMsg: "fresh target without id:: was not created and saved",
  });

  mark("copy block ref receipt");
  // Invoke Tine's real block menu command. The target begins without id::, so
  // the command must synchronously choose the durable UUID used everywhere else.
  const sourceMenuOpened = await browser.execute(() => {
    const freshBlock = [...document.querySelectorAll(".page-blocks > .ls-block")]
      .find((block) => block.querySelector(".block-content-wrapper")?.textContent?.trim() === "Fresh source target");
    const surface = freshBlock?.querySelector(":scope > .block-main > .block-content-wrapper");
    if (!(surface instanceof HTMLElement)) return false;
    const rect = surface.getBoundingClientRect();
    const event = new MouseEvent("contextmenu", {
      bubbles: true,
      cancelable: true,
      clientX: rect.left + Math.max(2, Math.min(20, rect.width / 2)),
      clientY: rect.top + Math.max(2, Math.min(12, rect.height / 2)),
      view: window,
    });
    return !surface.dispatchEvent(event) && event.defaultPrevented;
  });
  if (!sourceMenuOpened) throw new Error("fresh source block did not open Tine's context menu");
  await browser.$(".ctx-menu").waitForExist({ timeout: 5000 });
  const copyRef = await browser.$("//div[contains(concat(' ', normalize-space(@class), ' '), ' ctx-menu ')]//div[contains(concat(' ', normalize-space(@class), ' '), ' ctx-item ') and normalize-space(.) = 'Copy block ref']");
  await copyRef.waitForExist({ timeout: 5000 });
  // Durable receipt (og 10g): the reference the user receives must name an id
  // that is already on disk. Put a sentinel on the native clipboard, then read
  // the target file at the first moment the clipboard holds a reference.
  await nativeClipboard("write_text", "block-ref-count sentinel");
  await copyRef.click();
  let clipboardRef = null;
  let bytesAtReceipt = null;
  await browser.waitUntil(async () => {
    const text = await nativeClipboard("read_text");
    clipboardRef = typeof text === "string" ? text.match(/^\(\(([0-9a-f-]{36})\)\)$/i)?.[1] ?? null : null;
    if (clipboardRef) bytesAtReceipt = fs.readFileSync(`${GRAPH}/pages/Source Target.md`, "utf8");
    return clipboardRef !== null;
  }, { timeout: 10_000, interval: 25, timeoutMsg: "Copy block ref never put a ((uuid)) reference on the native clipboard" });
  const targetUuid = bytesAtReceipt.match(/(?:^|\n)\s*id::\s*([0-9a-f-]{36})(?:\s|$)/i)?.[1];
  if (targetUuid !== clipboardRef) {
    throw new Error(`clipboard held ((${clipboardRef})) before the target file carried that id: ${JSON.stringify(bytesAtReceipt)}`);
  }

  await browser.$('button[title="Go back"]').click();
  await browser.waitUntil(async () => (await browser.$$(".page-ref")).length >= 2, {
    timeout: 10_000, timeoutMsg: "journal links did not return after Back",
  });
  await openPage("Tester");

  mark("cold source lifecycle");
  // The cold source has never been opened in the frontend working set. A
  // filesystem watcher transaction must nevertheless refresh every visible
  // duplicate/reference by UUID, and deletion must invalidate the old value.
  const coldRef = await browser.$(".page-blocks .block-ref");
  await coldRef.waitForExist({ timeout: 10_000 });
  if ((await coldRef.getText()).trim() !== "Cold source") {
    throw new Error(`cold block reference did not resolve: ${JSON.stringify(await coldRef.getText())}`);
  }
  fs.writeFileSync(`${GRAPH}/pages/Cold Source.md`, `- Externally updated cold source\n  id:: ${COLD_TARGET}\n`);
  await browser.waitUntil(async () => (await coldRef.getText()).trim() === "Externally updated cold source", {
    timeout: 10_000,
    timeoutMsg: "visible block reference did not follow an external edit to an unloaded source",
  });
  fs.unlinkSync(`${GRAPH}/pages/Cold Source.md`);
  await browser.waitUntil(async () => await coldRef.getAttribute("class").then((value) => value.includes("block-ref-missing")), {
    timeout: 10_000,
    timeoutMsg: "visible block reference retained an externally deleted unloaded source",
  });

  mark("paste and badge/panel");
  // Paste the clipboard text produced by Copy block ref into another real block.
  await browser.$(".page-blocks .block-content-wrapper").click();
  const editor = await browser.$(".page-blocks textarea.block-editor");
  await editor.waitForExist({ timeout: 5000 });
  await browser.keys(["Control", "v"]);
  await browser.keys(["Escape"]);
  await browser.waitUntil(() => fs.readFileSync(`${GRAPH}/pages/Tester.md`, "utf8").includes(`((${targetUuid}))`), {
    timeout: 10_000, timeoutMsg: "pasted fresh block reference was not saved",
  });

  const ref = await browser.$(".page-blocks .block-ref");
  await ref.waitForExist({ timeout: 10_000 });
  await ref.click();
  await browser.waitUntil(async () => (await browser.$("h1.page-title").getText()).trim() === "Source Target", {
    timeout: 10_000, timeoutMsg: "saved block reference did not open its source",
  });
  const badge = await browser.$(".block-refs-count");
  await badge.waitForExist({ timeout: 10_000 });
  if ((await badge.getText()).trim() !== "1") throw new Error(`expected reference count 1, got ${JSON.stringify(await badge.getText())}`);
  await badge.click();
  const panelHeader = await browser.$(".block-references-header");
  await panelHeader.waitForExist({ timeout: 10_000 });
  if ((await panelHeader.getText()).trim() !== "1 Linked Reference") {
    throw new Error(`fresh target referrer panel did not report its saved reference: ${JSON.stringify(await panelHeader.getText())}`);
  }

  mark("reload badge/panel");
  // A real reload reparses id:: as the normal DTO/store identity. The same
  // durable target must still expose both its badge and its referrer panel.
  await browser.refresh();
  mark("reload: refreshed");
  await browser.waitUntil(async () => (await browser.$("h1.page-title").getText()).trim() === "Source Target", {
    timeout: 20_000,
    timeoutMsg: "fresh target route did not survive reload",
  });
  const reloadedBadge = await browser.$(".block-refs-count");
  await reloadedBadge.waitForExist({ timeout: 10_000 });
  if ((await reloadedBadge.getText()).trim() !== "1") {
    throw new Error(`fresh target reference count did not survive reload: ${JSON.stringify(await reloadedBadge.getText())}`);
  }
  mark("reload: badge present");
  // The reloaded page re-renders its blocks while it settles, so a badge handle
  // taken above may be a detached node by the time it is clicked (AGENTS.md §5:
  // never hold a handle across a re-rendering list). Find and toggle the live
  // badge in one document command, and repeat only while it is not open.
  const reloadClicks = [];
  await browser.waitUntil(async () => {
    const state = await browser.execute(() => {
      if (document.querySelector(".block-references-header")) return "panel";
      const badge = document.querySelector(".page-blocks .block-refs-count");
      if (!(badge instanceof HTMLElement)) return "no-badge";
      if (badge.classList.contains("open")) return "open-without-panel";
      badge.click();
      return "clicked";
    });
    if (state !== "panel" && reloadClicks.at(-1) !== state) reloadClicks.push(state);
    return state === "panel";
  }, { timeout: 10_000, interval: 250, timeoutMsg: "the reloaded badge never opened its referrer panel" }).catch((error) => {
    throw new Error(`${error.message}; badge states ${JSON.stringify(reloadClicks)}`);
  });
  if (reloadClicks.filter((state) => state === "clicked").length > 1) console.log(`note: reloaded badge needed ${JSON.stringify(reloadClicks)}`);
  const reloadedHeader = (await browser.execute(() => document.querySelector(".block-references-header")?.textContent ?? "")).trim();
  if (reloadedHeader !== "1 Linked Reference") throw new Error(`reloaded referrer panel reported ${JSON.stringify(reloadedHeader)}`);

  mark("source edit propagation");
  const sourceEditArmed = await browser.execute(() => {
    const badge = document.querySelector(".page-blocks .block-refs-count");
    const content = badge?.closest(".ls-block")?.querySelector(".block-content");
    if (!(content instanceof HTMLElement)) return false;
    const rect = content.getBoundingClientRect();
    const clientX = rect.left + Math.min(12, Math.max(1, rect.width / 2));
    const clientY = rect.top + Math.max(1, rect.height / 2);
    content.dispatchEvent(new MouseEvent("mousedown", {
      bubbles: true, cancelable: true, button: 0, buttons: 1, clientX, clientY,
    }));
    document.dispatchEvent(new MouseEvent("mouseup", {
      bubbles: true, cancelable: true, button: 0, buttons: 0, clientX, clientY,
    }));
    return true;
  });
  if (!sourceEditArmed) throw new Error("source block content was not available for editing");
  await sleep(200);
  const sourceEditApplied = await browser.execute((value) => {
    const editor = document.querySelector(".page-blocks textarea.block-editor");
    if (!(editor instanceof HTMLTextAreaElement)) return false;
    editor.focus();
    editor.value = value;
    editor.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: null }));
    return true;
  }, "Updated source target");
  if (!sourceEditApplied) throw new Error("source block editor did not open for input");
  await browser.keys(["Escape"]);
  await browser.waitUntil(() => fs.readFileSync(`${GRAPH}/pages/Source Target.md`, "utf8").includes("Updated source target"), {
    timeout: 10_000, timeoutMsg: "updated source block was not saved",
  });

  await browser.$('button[title="Go back"]').click();
  await browser.waitUntil(async () => (await browser.$("h1.page-title").getText()).trim() === "Tester", {
    timeout: 10_000, timeoutMsg: "Tester did not return after source edit",
  });
  await browser.waitUntil(async () => (await browser.$(".page-blocks .block-ref").getText()).trim() === "Updated source target", {
    timeout: 10_000, timeoutMsg: "inline block reference kept stale source text",
  });
  console.log("PASS: fresh Copy block ref identity, badge/panel, reload, and loaded/unloaded reference lifecycles");
} catch (error) {
  // Failure capsule: step, screenshot and the block-ref surface state.
  console.error(`FAIL at step: ${step}`);
  try { await browser?.saveScreenshot(path.join(ARTIFACTS, "block-ref-count-failure.png")); } catch {}
  try {
    console.error("state:", JSON.stringify(await browser.execute(() => ({
      title: document.querySelector("h1.page-title")?.textContent ?? null,
      badges: [...document.querySelectorAll(".block-refs-count")].map((node) => `${node.textContent}${node.classList.contains("open") ? " (open)" : ""}`),
      panels: [...document.querySelectorAll(".block-references-inner, .block-references")].map((node) => node.outerHTML.slice(0, 400)),
      toasts: [...document.querySelectorAll(".toast, [role=status], [role=alert]")].map((node) => node.textContent),
    }))));
  } catch {}
  try {
    const uuid = fs.readFileSync(`${GRAPH}/pages/Source Target.md`, "utf8").match(/id::\s*([0-9a-f-]{36})/i)?.[1];
    if (uuid) console.error("backend block_referrers:", JSON.stringify(await browser.executeAsync((id, done) => {
      globalThis.__TAURI_INTERNALS__.invoke("block_referrers", { uuid: id }).then((groups) => done(groups), (error) => done(`<${error}>`));
    }, uuid)));
  } catch {}
  for (const name of ["Source Target.md", "Tester.md"]) {
    try { console.error(`${name}:`, JSON.stringify(fs.readFileSync(`${GRAPH}/pages/${name}`, "utf8"))); } catch {}
  }
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-td.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
