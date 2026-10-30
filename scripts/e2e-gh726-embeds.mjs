// GH #726: routed-page host/source ownership, real cross-page cut/paste,
// durable properties/identity/children/folds and undo through the native app.
// D-20261030-EMB: a block embed has one bullet, the embedded root's, and it acts
// on the host; so for EMBED_KIND=block the "host bullet" is that root row and the
// source's own bullet is on the Source page. Page embeds keep both bullets.
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { execFileSync, spawn } from "node:child_process";
import { remote } from "webdriverio";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";

const root = path.resolve(import.meta.dirname, "..");
const testedCommit = execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim();
const kind = process.env.EMBED_KIND || "block";
assert(["block", "page"].includes(kind));
const out = path.resolve("artifacts/gh726-embeds/native");
fs.mkdirSync(out, { recursive: true });
const run = fs.mkdtempSync(path.join(out, `${kind}-`));
const graph = path.join(run, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(run, dir));
const S = "72600000-0000-4000-8000-000000000001";
const C = "72600000-0000-4000-8000-000000000002";
const H = "72600000-0000-4000-8000-000000000003";
const macro = kind === "block" ? `{{embed ((${S}))}}` : "{{embed [[Source]]}}";
const hostFile = path.join(graph, "pages/Embed ownership.md");
const sourceFile = path.join(graph, "pages/Source.md");
const targetFile = path.join(graph, "pages/Destination.md");
const originalSource = `- Source root\n  id:: ${S}\n  custom:: preserved\n  - Folded child\n    id:: ${C}\n    collapsed:: true\n    - Hidden grandchild\n- Source survivor\n`;
const originalHost = `- ${macro}\n  id:: ${H}\n- Mirror\n  - ${macro}\n- Source reference ((${S}))\n- Host survivor\n`;
fs.writeFileSync(sourceFile, originalSource);
fs.writeFileSync(hostFile, originalHost);
fs.writeFileSync(targetFile, "- Destination anchor\n");
const port = Number(process.env.E2E_DRIVER_PORT || 4876);
const app = process.env.TINE_APP || path.join(root, "target/debug/tine");
const env = { ...process.env, TINE_GRAPH: graph, XDG_DATA_HOME: path.join(run, "data"), XDG_CONFIG_HOME: path.join(run, "config"), XDG_CACHE_HOME: path.join(run, "cache"), WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const log = fs.openSync(path.join(run, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", ["--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log] });
let browser;
let bindingGeneration;
let stage = "startup";
const passed = [];
const read = file => fs.readFileSync(file, "utf8");
const sourceRow = `.main-content .embed-block .ls-block[data-block-ref="${S}"]`;
const hostRow = `.main-content .ls-block[data-block-ref="${H}"]`;
const ownBullet = row => `${row} > .block-main > .block-controls .bullet-container`;
const hostBulletRow = kind === "block" ? sourceRow : hostRow;
const sourcePageRow = `.main-content .ls-block[data-block-ref="${S}"]`;
// The source block's own bullet: inside a page embed, or on its own page for a block embed.
async function sourceMenu(label) {
  if (kind === "page") return menu(sourceRow, label);
  await openPageByName(browser, "Source");
  await browser.$(ownBullet(sourcePageRow)).waitForExist({ timeout: 20000 });
  await menu(sourcePageRow, label);
  await openPageByName(browser, "Embed ownership");
}
const sourceVisible = () => browser.$(ownBullet(sourceRow)).waitForExist({ timeout: 20000 });

async function menu(row, label) {
  // Source rows can remount after Undo or page hydration. Like the query-result
  // journey, locate and measure the live bullet in one round trip.
  await browser.waitUntil(async () => {
    if (await browser.$(".ctx-menu").isExisting()) return true;
    const point = await browser.execute(selector => {
    const bullet = document.querySelector(selector);
    if (!bullet) return null;
    bullet.scrollIntoView({ block: "center" });
    const box = bullet.getBoundingClientRect();
    const x = Math.round(box.left + box.width / 2), y = Math.round(box.top + box.height / 2);
    return bullet.contains(document.elementFromPoint(x, y)) ? { x, y } : null;
    }, ownBullet(row));
    if (!point) return false;
    await browser.performActions([{ type: "pointer", id: "mouse", parameters: { pointerType: "mouse" }, actions: [
    { type: "pointerMove", duration: 0, origin: "viewport", x: point.x, y: point.y },
    { type: "pointerDown", button: 2 }, { type: "pointerUp", button: 2 },
    ] }]);
    await browser.releaseActions();
    const opened = await browser.$(".ctx-menu").waitForExist({ timeout: 1500 }).then(() => true, () => false);
    if (!opened) await browser.keys(["Escape"]); // dismiss a native menu after a remount
    return opened;
  }, { timeout: 15000, interval: 150, timeoutMsg: `live bullet menu unavailable: ${row}` });
  await browser.$(`//div[contains(@class,"ctx-item") and normalize-space(.)="${label}"]`).click();
}
async function undoUntil(file, predicate) {
  await browser.keys(["Control", "z"]);
  await waitForFileText(file, predicate);
}
async function pasteAtDestination() {
  await openPageByName(browser, "Destination");
  const content = await browser.$(".main-content .block-content");
  await content.waitForExist();
  await content.click();
  await browser.$("textarea.block-editor").waitForExist();
  await browser.keys(["Control", "End"]);
  await browser.keys(["Control", "v"]);
}
async function pageOutline(name) {
  const page = JSON.parse(await browser.executeAsync((args, done) => {
    window.__TAURI_INTERNALS__.invoke("get_page", { ...args, kind: "page" })
      .then(page => done(JSON.stringify(page)), error => done(JSON.stringify({ error: String(error) })));
  }, { name, bindingGeneration }));
  assert(!page.error, page.error);
  const semantic = block => ({ raw: block.raw, collapsed: block.collapsed, children: block.children.map(semantic) });
  return page.blocks.map(semantic);
}
function pass() { passed.push(stage); console.log(`PASS ${kind}: ${stage}`); }

try {
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 2, connectionRetryTimeout: 30000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: app } } });
  await ensureMainWindow(browser);
  await browser.setWindowSize(1200, 1000);
  await browser.$(".app-container").waitForExist({ timeout: 30000 });
  await openPageByName(browser, "Embed ownership");
  await sourceVisible();
  // As in e2e-multigraph: loading the graph this window already holds returns
  // its current lease without changing it. Keep all read assertions bound.
  const binding = JSON.parse(await browser.executeAsync((root, done) => {
    window.__TAURI_INTERNALS__.invoke("load_graph", { path: root })
      .then(result => done(JSON.stringify(result)), error => done(JSON.stringify({ error: String(error) })));
  }, graph));
  assert.equal(binding.kind, "already_current");
  bindingGeneration = binding.binding_generation;
  const originalOutline = await pageOutline("Source");

  stage = "host bullet cut and paste moves only the embed; undo restores it";
  await menu(hostBulletRow, "Cut block");
  await waitForFileText(hostFile, text => !text.includes(`id:: ${H}`));
  assert.equal(read(sourceFile), originalSource);
  await pasteAtDestination();
  await waitForFileText(targetFile, text => text.includes(macro) && text.includes(`id:: ${H}`));
  assert.equal(read(sourceFile), originalSource);
  await browser.keys(["Escape"]);
  await undoUntil(targetFile, text => !text.includes(macro));
  await undoUntil(hostFile, text => text.includes(`id:: ${H}`));
  assert.equal(read(sourceFile), originalSource);
  pass();

  stage = "source bullet copy preserves both source and embed";
  await openPageByName(browser, "Embed ownership");
  await sourceVisible();
  await sourceMenu("Copy block");
  const copied = await browser.execute(async () => window.__TAURI_INTERNALS__.invoke("plugin:clipboard-manager|read_text"));
  assert(String(copied).includes("Source root"));
  assert(!String(copied).includes("{{embed"));
  assert.equal(read(sourceFile), originalSource);
  assert(read(hostFile).includes(macro));
  pass();

  stage = "source bullet cut and paste really moves identity, properties, folded children; embeds and refs update";
  await sourceMenu("Cut block");
  await waitForFileText(sourceFile, text => !text.includes("Source root") && text.includes("Source survivor"));
  assert(read(hostFile).includes(`id:: ${H}`));
  await browser.waitUntil(() => browser.execute((selector) => !document.querySelector(selector), sourceRow), { timeout: 15000 });
  await pasteAtDestination();
  await waitForFileText(targetFile, text => text.includes(`id:: ${S}`) && text.includes(`id:: ${C}`) && text.includes("custom:: preserved") && text.includes("collapsed:: true") && text.includes("Hidden grandchild"));
  assert.deepEqual((await pageOutline("Destination")).find(block => block.raw.includes(`id:: ${S}`)), originalOutline[0]);
  assert(!read(sourceFile).includes(`id:: ${S}`));
  await browser.keys(["Escape"]);
  await openPageByName(browser, "Embed ownership");
  if (kind === "block") {
    await sourceVisible();
    assert.equal((await browser.$$(sourceRow)).length, 2);
  } else {
    await browser.waitUntil(() => browser.execute((selector) => !document.querySelector(selector), sourceRow), { timeout: 15000 });
    assert((await browser.$(".embed-page").getText()).includes("Source survivor"));
  }
  // The inline reference still resolves the moved identity, in either embed form.
  await browser.waitUntil(() => browser.execute(() => [...document.querySelectorAll(".main-content .block-ref")].some(el => el.textContent.includes("Source root"))), { timeout: 15000 });
  pass();

  stage = "undo source paste and cut restores source and every embed";
  await undoUntil(targetFile, text => !text.includes(`id:: ${S}`));
  await undoUntil(sourceFile, text => text.includes(`id:: ${S}`) && text.includes("Hidden grandchild"));
  await sourceVisible();
  assert.deepEqual(await pageOutline("Source"), originalOutline);
  pass();

  stage = "delete the source through its embed bullet; undo restores it";
  await sourceMenu("Delete block");
  await waitForFileText(sourceFile, text => !text.includes(`id:: ${S}`) && !text.includes("Hidden grandchild"));
  assert(read(hostFile).includes(`id:: ${H}`));
  await browser.waitUntil(() => browser.execute((selector) => !document.querySelector(selector), sourceRow), { timeout: 15000 });
  await undoUntil(sourceFile, text => text.includes(`id:: ${S}`));
  await sourceVisible();
  assert.deepEqual(await pageOutline("Source"), originalOutline);
  assert(read(hostFile).includes(macro));
  pass();
  fs.writeFileSync(path.join(run, "result.json"), JSON.stringify({ outcome: "pass", testedCommit, kind, passed, graph }, null, 2));
} catch (error) {
  if (browser) {
    try { fs.writeFileSync(path.join(run, "failure-dom.html"), await browser.execute(() => document.documentElement.outerHTML)); } catch {}
  }
  fs.writeFileSync(path.join(run, "result.json"), JSON.stringify({ outcome: "fail", testedCommit, kind, classification: "ambiguous", expected: stage, observed: String(error), passed, graph }, null, 2));
  console.error(`FAIL ${kind}: ${stage}: ${error}`);
  process.exitCode = 1;
} finally {
  if (browser) await browser.deleteSession().catch(() => {});
  driver.kill();
  fs.closeSync(log);
}
