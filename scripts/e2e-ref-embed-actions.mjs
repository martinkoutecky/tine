// GH #417 (with #418's toolbar remainder): Edit link / Open source / Remove on
// one embed or block reference, through the real pointer in the main pane, a
// split pane and the right sidebar, plus keyboard focus on the embed toolbar.
// Remove must change only the containing page's file (never the referenced
// block's), and Undo must bring it back. Replace with text / Replace with
// embed rewrite only the clicked reference occurrence the same way.
import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { remote } from "webdriverio";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";

const ROOT = path.resolve(import.meta.dirname, "..");
const OUT = path.resolve("artifacts/ref-embed-actions-417");
fs.mkdirSync(OUT, { recursive: true });
const RUN = fs.mkdtempSync(path.join(OUT, "native-")), GRAPH = path.join(RUN, "graph");
const org = process.env.E2E_FORMAT === "org";
const ext = org ? "org" : "md";
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(RUN, dir));
fs.writeFileSync(path.join(GRAPH, "logseq/config.edn"), "{}\n");
const T = "41700000-0000-4000-8000-000000000001";
const EMBED = `{{embed ((${T}))}}`, REF = `((${T}))`;
const source = org ? `* target text\n:PROPERTIES:\n:id: ${T}\n:END:\n` : `- target text\n  id:: ${T}\n`;
const host = org
  ? `* ${EMBED}\n* see ${REF} here\n* [[Host]]\n`
  : `- ${EMBED}\n- see ${REF} here\n- [[Host]]\n`;
const sourcePath = path.join(GRAPH, `pages/Source.${ext}`), hostPath = path.join(GRAPH, `pages/Host.${ext}`);
fs.writeFileSync(sourcePath, source);
fs.writeFileSync(hostPath, host);
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const TD = process.env.TAURI_DRIVER || path.join(process.env.CARGO_HOME || "", "bin/tauri-driver");
const port = Number(process.env.E2E_DRIVER_PORT || 4792);
const env = { ...process.env, TINE_GRAPH: GRAPH, XDG_DATA_HOME: path.join(RUN, "data"), XDG_CONFIG_HOME: path.join(RUN, "config"), XDG_CACHE_HOME: path.join(RUN, "cache"), WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const log = fs.openSync(path.join(RUN, "driver.log"), "w");
const driver = spawn(TD, ["--port", String(port), "--native-port", String(port + 1), "--native-driver", "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log], detached: true });
const read = (p) => fs.readFileSync(p, "utf8");
let browser, stage = "startup";
const receipt = { tested: "8a5539b18 + review r1 fixes", app: APP, format: ext, surfaces: [], screenshots: [], screenshotErrors: [] };

/** Screenshots are evidence, not assertions: a stalled WebKitWebDriver
 *  screenshot GET must not fail a journey whose state checks passed. */
async function shot(name) {
  try { await browser.saveScreenshot(path.join(RUN, name)); receipt.screenshots.push(name); }
  catch (error) { receipt.screenshotErrors.push(`${name}: ${error.message.split("\n")[0]}`); }
}

async function waitFile(predicate, what) {
  await browser.waitUntil(() => predicate(read(hostPath)), { timeout: 15000, timeoutMsg: `${stage}: host file never ${what}:\n${read(hostPath)}` });
  assert.equal(read(sourcePath), source, `${stage}: the referenced page's file changed`);
}

/** Hover the embed under `scope` with the real pointer; its toolbar must become visible. */
async function hoverEmbed(scope) {
  const embed = await browser.$(`${scope} .embed-block`);
  await embed.waitForExist({ timeout: 15000 });
  await browser.waitUntil(() => browser.execute((s) => document.querySelector(`${s} .embed-block`)?.textContent.includes("target text"), scope), { timeout: 15000 });
  await embed.moveTo();
  await browser.waitUntil(() => browser.execute((s) => {
    const bar = document.querySelector(`${s} .embed-block > .embed-actions`);
    return !!bar && getComputedStyle(bar).opacity === "1" && getComputedStyle(bar).pointerEvents !== "none" && bar.getBoundingClientRect().height > 0;
  }, scope), { timeout: 5000, timeoutMsg: `${stage}: hovering the embed did not show its actions` });
}

async function undo() {
  await browser.keys(["Escape"]);
  await browser.keys(["Control", "z", "NULL"]);
}

/** Edit link, then Remove + Undo, on the embed rendered under `scope`. */
async function embedActions(scope, label) {
  stage = `${label}: embed Edit link`;
  await hoverEmbed(scope);
  await shot(`hover-${label}.png`);
  await (await browser.$(`${scope} .embed-block > .embed-actions .embed-action-edit`)).click();
  await browser.waitUntil(() => browser.execute((s, want) => {
    const ed = document.querySelector(`${s} textarea.block-editor`);
    return !!ed && ed.value.slice(ed.selectionStart, ed.selectionEnd) === want;
  }, scope, EMBED), { timeout: 8000, timeoutMsg: `${stage}: the host editor did not open in this surface with the embed selected` });
  assert.equal(await browser.execute(() => document.querySelectorAll("textarea.block-editor").length), 1, `${stage}: more than one editor opened`);
  await browser.keys(["Escape"]);
  await browser.waitUntil(() => browser.execute(() => !document.querySelector("textarea.block-editor")), { timeout: 5000 });
  assert.equal(read(hostPath), host, `${stage}: Edit link changed the file`);

  stage = `${label}: embed Remove + Undo`;
  await hoverEmbed(scope);
  await (await browser.$(`${scope} .embed-block > .embed-actions .embed-action-remove`)).click();
  await waitFile((t) => !t.includes("{{embed") && t.includes(`see ${REF} here`), "lost the embed");
  await undo();
  await waitFile((t) => t === host, "got the embed back on Undo");
  receipt.surfaces.push(label);
}

try {
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 0, connectionRetryTimeout: 60000, capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } } });
  await ensureMainWindow(browser);
  await browser.$(".app-container").waitForExist({ timeout: 30000 });
  await openPageByName(browser, "Host");

  await embedActions('[data-pane-id="main"]', "main");

  stage = "main: keyboard focus keeps the toolbar and Tab walks it";
  await (await browser.$('[data-pane-id="main"] h1.page-title')).moveTo();
  await browser.execute(() => document.querySelector('[data-pane-id="main"] .embed-action-edit').focus());
  await browser.keys(["Tab"]);
  assert.deepEqual(await browser.execute(() => {
    const bar = document.querySelector('[data-pane-id="main"] .embed-block > .embed-actions');
    return [document.activeElement?.className, getComputedStyle(bar).opacity];
  }), ["embed-action-source", "1"], `${stage}: focus left the toolbar or it hid without hover`);
  await browser.execute(() => document.activeElement?.blur());

  stage = "main: block reference Remove reference + Undo";
  const ref = await browser.$('[data-pane-id="main"] .block-content .block-ref');
  await ref.waitForExist({ timeout: 10000 });
  await ref.click({ button: "right" });
  await browser.waitUntil(() => browser.execute(() => [...document.querySelectorAll(".ctx-item")].some((el) => el.textContent === "Remove reference")), { timeout: 5000 });
  await shot("ref-menu.png");
  await browser.execute(() => [...document.querySelectorAll(".ctx-item")].find((el) => el.textContent === "Remove reference").click());
  await waitFile((t) => t.includes("see here") && !t.includes(`see ${REF}`) && t.includes(EMBED), "lost only the reference");
  await undo();
  await waitFile((t) => t === host, "got the reference back on Undo");

  for (const [row, tag, check] of [
    ["Replace with text", "text", (t) => t.includes("see target text here") && !t.includes(`see ${REF}`) && t.includes(EMBED)],
    ["Replace with embed", "embed", (t) => t.includes(`see ${EMBED} here`) && !t.includes(`see ${REF}`)],
  ]) {
    stage = `main: block reference ${row} + Undo`;
    const target = await browser.$('[data-pane-id="main"] .block-content .block-ref');
    await target.waitForExist({ timeout: 10000 });
    await target.click({ button: "right" });
    await browser.waitUntil(() => browser.execute((want) => [...document.querySelectorAll(".ctx-item")].some((el) => el.textContent === want), row), { timeout: 5000, timeoutMsg: `${stage}: the menu has no "${row}" row` });
    if (tag === "text") await shot("ref-menu-replace-rows.png");
    await browser.execute((want) => [...document.querySelectorAll(".ctx-item")].find((el) => el.textContent === want).click(), row);
    await waitFile(check, `got the ${tag} replacement`);
    assert.ok(!read(hostPath).includes("id::") && !read(hostPath).includes(":id:"), `${stage}: the target's id property leaked into the host`);
    await undo();
    await waitFile((t) => t === host, "got the reference back on Undo");
  }

  stage = "main: embed Open source";
  await hoverEmbed('[data-pane-id="main"]');
  await (await browser.$('[data-pane-id="main"] .embed-block > .embed-actions .embed-action-source')).click();
  await browser.waitUntil(() => browser.execute(() => document.querySelector('[data-pane-id="main"] h1.page-title')?.textContent.trim() === "Source"), { timeout: 10000, timeoutMsg: "Open source did not go to the Source page" });
  await openPageByName(browser, "Host");

  stage = "split pane";
  await browser.$('[data-pane-id="main"] .embed-block').waitForExist({ timeout: 15000 });
  await browser.keys(["Control", "Alt", "\\", "NULL"]);
  await browser.waitUntil(() => browser.execute(() => document.querySelectorAll("[data-pane-id]").length === 2), { timeout: 8000, timeoutMsg: "the split pane did not open" });
  const split = await browser.execute(() => [...document.querySelectorAll("[data-pane-id]")].map((p) => p.getAttribute("data-pane-id")).find((id) => id !== "main"));
  await embedActions(`[data-pane-id="${split}"]`, "split");

  stage = "right sidebar";
  await browser.execute(() => {
    const link = [...document.querySelectorAll('[data-pane-id="main"] .page-ref')].find((el) => el.textContent.includes("Host"));
    link.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, shiftKey: true }));
  });
  await browser.$(".right-sidebar .embed-block").waitForExist({ timeout: 10000 });
  await embedActions(".right-sidebar", "sidebar");

  assert.equal(read(sourcePath), source);
  fs.writeFileSync(path.join(RUN, "receipt.json"), JSON.stringify({ ...receipt, outcome: "PASS" }, null, 2));
  console.log(`PASS: native embed and reference actions in ${receipt.surfaces.join(", ")} (${RUN})`);
} catch (error) {
  try { await browser?.saveScreenshot(path.join(RUN, "failure.png")); } catch {}
  try { fs.writeFileSync(path.join(RUN, "failure-dom.html"), await browser.getPageSource()); } catch {}
  fs.writeFileSync(path.join(RUN, "failure.txt"), `Tested: ${receipt.tested}\nJourney: ${stage}\nExpected: hovering an embed shows Edit link/Open source/Remove; Edit link opens only that surface's host editor with the embed selected; Remove and Remove reference change only the host file and Undo restores it byte for byte; the Source file never changes.\nObserved: ${error.stack}\nClassification: ambiguous until screenshot and log review.\n`);
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
