// Real-app regressions for GH #154 and GH #166. A graph-wide reference-count map
// is loaded before a new block reference exists; saving the reference must
// refresh the source block's badge without reopening the graph. Editing that
// already-loaded source must then refresh the inline reference text as well.
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
const TMP = "/tmp/tine-block-ref-count-e2e";
const GRAPH = `${TMP}/graph`;
const TARGET = "77777777-7777-4777-8777-777777777777";
const COLD_TARGET = "88888888-8888-4888-8888-888888888888";

fs.rmSync(TMP, { recursive: true, force: true });
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(`${GRAPH}/${dir}`, { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(`${TMP}/xdg/${dir}`, { recursive: true });
fs.writeFileSync(`${GRAPH}/logseq/config.edn`, "{}\n");
fs.writeFileSync(`${GRAPH}/pages/Source Target.md`, `- Source target\n  id:: ${TARGET}\n`);
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
const log = fs.openSync(`${TMP}/tauri-driver.log`, "w");
const td = spawn(TD, ["--port", String(DRIVER_PORT), "--native-port", String(NATIVE_PORT), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], {
  env, stdio: ["ignore", log, log], detached: true,
});
await sleep(2500);

let browser;
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

  // Force the initial empty count map to load while the source block is mounted.
  await openPage("Source Target");
  await sleep(1500);
  if (await browser.$(".block-refs-count").isExisting()) throw new Error("source badge existed before any reference");

  const back = await browser.$('button[title="Go back"]');
  await back.click();
  await browser.waitUntil(async () => (await browser.$$(".page-ref")).length >= 2, {
    timeout: 10_000, timeoutMsg: "journal links did not return after Back",
  });
  await openPage("Tester");

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

  await browser.$(".page-blocks .block-content-wrapper").click();
  const editor = await browser.$(".page-blocks textarea.block-editor");
  await editor.waitForExist({ timeout: 5000 });
  await editor.setValue(`((${TARGET}))`);
  await browser.keys(["Escape"]);
  await browser.waitUntil(() => fs.readFileSync(`${GRAPH}/pages/Tester.md`, "utf8").includes(`((${TARGET}))`), {
    timeout: 10_000, timeoutMsg: "new block reference was not saved",
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

  const sourceEditArmed = await browser.execute(() => {
    const content = document.querySelector(".page-blocks .block-content");
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

  await back.click();
  await browser.waitUntil(async () => (await browser.$("h1.page-title").getText()).trim() === "Tester", {
    timeout: 10_000, timeoutMsg: "Tester did not return after source edit",
  });
  await browser.waitUntil(async () => (await browser.$(".page-blocks .block-ref").getText()).trim() === "Updated source target", {
    timeout: 10_000, timeoutMsg: "inline block reference kept stale source text",
  });
  console.log("PASS: reference counts and loaded/unloaded external block-reference lifecycles refresh live");
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-td.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
