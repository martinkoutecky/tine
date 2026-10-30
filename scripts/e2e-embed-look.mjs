// D-20261030-EMB (#417, #748): block embeds are back to 0.8's one bullet. "Where a block sits is per page;
// what it says is shared." One block is embedded on two pages. The journey checks that the embed shows one
// bullet, at the host's place; that folding its root on page A is saved on A's host and leaves page B open,
// and that both survive an app restart; that a fold below the root is saved on the source and shows on every
// page; that Tab/Shift+Tab on the edited root move the host; and that the bullet's menu deletes the host only.
// Run under xvfb-run -s "-screen 0 1920x1080x24" with TINE_APP set to a custom-protocol build.
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { startWebdriverApplication, stopWebdriverApplication, tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const TD = process.env.TAURI_DRIVER || "tauri-driver";
const DRIVER = Number(process.env.E2E_DRIVER_PORT || 4642);
const NATIVE = Number(process.env.E2E_NATIVE_PORT || 4643);
const OUT = process.env.E2E_ARTIFACT_DIR || path.join(ROOT, "artifacts/embed-look");
const TMP = path.join(OUT, `native-${process.pid}`);
const GRAPH = path.join(TMP, "graph");
fs.mkdirSync(OUT, { recursive: true });
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
for (const dir of ["data", "config", "cache", "home"]) fs.mkdirSync(path.join(TMP, "xdg", dir), { recursive: true });
fs.writeFileSync(path.join(GRAPH, "logseq/config.edn"), "{:tine/guide-announced? true}\n");
const S = "41700000-0000-4000-8000-000000000748";
const SOURCE = path.join(GRAPH, "pages/Embed source.md");
const PAGE_A = path.join(GRAPH, "pages/Embed A.md");
const PAGE_B = path.join(GRAPH, "pages/Embed B.md");
const MACRO = `{{embed ((${S}))}}`;
const original = {
  source: `- Embedded root\n  id:: ${S}\n  - Shared child\n    - Deep grandchild\n- Source survivor\n`,
  a: `- Before A\n- ${MACRO}\n`,
  b: `- Before B\n- ${MACRO}\n`,
};
fs.writeFileSync(SOURCE, original.source);
fs.writeFileSync(PAGE_A, original.a);
fs.writeFileSync(PAGE_B, original.b);
const read = (file) => fs.readFileSync(file, "utf8");
const sharedFold = (text) => /- Shared child\n\s+collapsed:: true\n/.test(text);

const env = { ...process.env, HOME: path.join(TMP, "xdg/home"), TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: path.join(TMP, "xdg/data"), XDG_CONFIG_HOME: path.join(TMP, "xdg/config"), XDG_CACHE_HOME: path.join(TMP, "xdg/cache"),
  APPDATA: path.join(TMP, "appdata"), LOCALAPPDATA: path.join(TMP, "localappdata"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const webviewTarget = await startWebdriverApplication(APP, env, NATIVE);
const log = fs.openSync(path.join(OUT, "embed-look-tauri-driver.log"), "w");
const driver = spawn(TD, webdriverServerArgs(DRIVER, NATIVE, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"),
  { env: webviewTarget.env, stdio: ["ignore", log, log], detached: process.platform !== "win32" });
await sleep(2500);

const ROW = `.main-content .embed-block .ls-block[data-block-ref="${S}"]`;
let browser;
let stage = "startup";
const passed = [];
const pass = () => { passed.push(stage); console.log(`PASS: ${stage}`); };

async function launch() {
  browser = await remote({ hostname: "127.0.0.1", port: DRIVER, path: "/", logLevel: "error", connectionRetryCount: 1, connectionRetryTimeout: 60_000,
    capabilities: tauriCapabilities(APP, "default", process.platform, webviewTarget.debuggerAddress) });
  await ensureMainWindow(browser);
  try { await browser.setWindowRect(0, 0, 1200, 1000); } catch {}
  await browser.$(".app-container").waitForExist({ timeout: 30_000 });
}
async function openEmbedPage(name) {
  await openPageByName(browser, name);
  await browser.$(`${ROW} > .block-main .block-content`).waitForExist({ timeout: 20_000 });
}
/** Visible text of the embed on the routed page. */
const embedText = () => browser.execute((row) => document.querySelector(row)?.innerText ?? "", ROW);
async function waitEmbed(text, visible) {
  await browser.waitUntil(async () => (await embedText()).includes(text) === visible,
    { timeout: 10_000, timeoutMsg: `${text}: expected visible=${visible}, embed shows ${JSON.stringify(await embedText())}` });
}
/** The full pointer sequence on a row's disclosure control (as e2e-block-embed.mjs). */
async function toggle(rowSelector) {
  const found = await browser.execute((selector) => {
    const control = document.querySelector(selector);
    if (!control) return false;
    for (const type of ["mousedown", "mouseup", "click"]) control.dispatchEvent(new MouseEvent(type, { bubbles: true, button: 0 }));
    return true;
  }, `${rowSelector} > .block-main .collapse-toggle`);
  assert(found, `no disclosure control on ${rowSelector}`);
}
async function editRoot() {
  await browser.$(`${ROW} > .block-main .block-content`).click();
  await browser.$(`${ROW} textarea.block-editor`).waitForExist({ timeout: 5000 });
}

try {
  await launch();
  stage = "a block embed shows one bullet, at the host's place";
  await openEmbedPage("Embed A");
  const shape = await browser.execute((row) => {
    const root = document.querySelector(row);
    const host = root?.closest(".ls-block:not(.embed-block .ls-block)");
    const before = [...document.querySelectorAll(".main-content .ls-block")].find((el) => el.querySelector(":scope > .block-main .block-content")?.textContent.trim() === "Before A");
    const x = (el) => el?.querySelector(":scope > .block-main > .block-controls .bullet")?.getBoundingClientRect().left ?? null;
    return { hostControls: host?.querySelectorAll(":scope > .block-main > .block-controls").length ?? -1,
      hostIsHost: host?.classList.contains("block-embed-host") ?? false,
      bullets: document.querySelectorAll(".main-content .bullet-container").length,
      rootX: x(root), beforeX: x(before) };
  }, ROW);
  assert.equal(shape.hostIsHost, true, JSON.stringify(shape));
  assert.equal(shape.hostControls, 0, `the host still has its own bullet: ${JSON.stringify(shape)}`);
  assert.equal(shape.bullets, 4, `expected Before A + root + child + grandchild bullets: ${JSON.stringify(shape)}`);
  assert(shape.rootX !== null && shape.beforeX !== null && Math.abs(shape.rootX - shape.beforeX) < 2,
    `the embed's bullet is not at the host's place: ${JSON.stringify(shape)}`);
  pass();

  stage = "folding the root on page A saves A's host only";
  await toggle(ROW);
  await waitEmbed("Shared child", false);
  await waitForFileText(PAGE_A, (text) => text === `- Before A\n- ${MACRO}\n  collapsed:: true\n`);
  assert.equal(read(SOURCE), original.source);
  assert.equal(read(PAGE_B), original.b);
  await openEmbedPage("Embed B");
  await waitEmbed("Shared child", true);
  pass();

  stage = "a fold below the root saves the source and shows on every page";
  const child = await browser.execute((row) => {
    const el = [...document.querySelectorAll(`${row} .ls-block`)].find((r) => r.querySelector(":scope > .block-main .block-content")?.textContent.trim() === "Shared child");
    if (!el) return null;
    el.setAttribute("data-e2e-child", "1");
    return true;
  }, ROW);
  assert(child, "no Shared child row in page B's embed");
  await toggle(`${ROW} .ls-block[data-e2e-child="1"]`);
  await waitEmbed("Deep grandchild", false);
  await waitForFileText(SOURCE, sharedFold);
  assert.equal(read(PAGE_B), original.b);
  pass();

  stage = "Tab and Shift+Tab on the edited root move the host";
  await editRoot();
  await browser.keys(["Tab"]);
  await waitForFileText(PAGE_B, (text) => text === `- Before B\n  - ${MACRO}\n`);
  assert(sharedFold(read(SOURCE)) && read(SOURCE).startsWith(`- Embedded root\n  id:: ${S}\n`), read(SOURCE));
  await browser.waitUntil(() => browser.execute((row) => !!document.activeElement?.closest(row), ROW),
    { timeout: 5000, timeoutMsg: "the caret left the embed after Tab" });
  await browser.keys(["Shift", "Tab", "NULL"]);
  await waitForFileText(PAGE_B, (text) => text === original.b);
  await browser.keys(["Escape"]);
  pass();

  stage = "both pages keep their folds after an app restart";
  await browser.deleteSession();
  await launch();
  await openEmbedPage("Embed B");
  await waitEmbed("Shared child", true);
  await waitEmbed("Deep grandchild", false);
  await openEmbedPage("Embed A");
  await waitEmbed("Shared child", false);
  await openPageByName(browser, "Embed source");
  await browser.waitUntil(() => browser.execute(() => {
    const text = document.querySelector(".main-content")?.innerText ?? "";
    return text.includes("Shared child") && !text.includes("Deep grandchild");
  }), { timeout: 10_000, timeoutMsg: "the source page does not show the shared fold" });
  pass();

  stage = "the bullet's menu deletes the host only; Undo restores it";
  await openEmbedPage("Embed A");
  const point = await browser.execute((row) => {
    const bullet = document.querySelector(`${row} > .block-main > .block-controls .bullet-container`);
    if (!bullet) return null;
    const box = bullet.getBoundingClientRect();
    return { x: Math.round(box.left + box.width / 2), y: Math.round(box.top + box.height / 2) };
  }, ROW);
  assert(point, "no bullet on the embed root row");
  await browser.performActions([{ type: "pointer", id: "mouse", parameters: { pointerType: "mouse" }, actions: [
    { type: "pointerMove", duration: 0, origin: "viewport", x: point.x, y: point.y },
    { type: "pointerDown", button: 2 }, { type: "pointerUp", button: 2 }] }]);
  await browser.releaseActions();
  await browser.$(".ctx-menu").waitForExist({ timeout: 5000 });
  await browser.$('//div[contains(@class,"ctx-item") and normalize-space(.)="Delete block"]').click();
  await waitForFileText(PAGE_A, (text) => text === "- Before A\n");
  assert(sharedFold(read(SOURCE)) && read(SOURCE).includes("Source survivor"), read(SOURCE));
  await browser.keys(["Control", "z", "NULL"]);
  await waitForFileText(PAGE_A, (text) => text.includes(MACRO));
  pass();
  fs.writeFileSync(path.join(OUT, "embed-look-result.json"), JSON.stringify({ outcome: "pass", passed, graph: GRAPH }, null, 2));
  console.log("PASS: one bullet; root fold per page across restart; nested fold shared; Tab and the menu act on the host");
} catch (error) {
  try { await browser?.saveScreenshot(path.join(OUT, "failure.png")); } catch {}
  try { fs.writeFileSync(path.join(OUT, "failure-dom.html"), await browser.execute(() => document.documentElement.outerHTML)); } catch {}
  fs.writeFileSync(path.join(OUT, "failure.json"), JSON.stringify({
    commit: process.env.TINE_SOURCE_REVISION || process.env.TINE_BUILD_COMMIT || "working-tree", journey: "D-20261030-EMB one-bullet block embeds",
    expected: stage, observed: String(error), passed, files: { source: read(SOURCE), a: read(PAGE_A), b: read(PAGE_B) },
    classification: "ambiguous; inspect the DOM and driver log" }, null, 2));
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  stopWebdriverApplication(webviewTarget);
  fs.closeSync(log);
}
