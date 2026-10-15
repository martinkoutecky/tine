// GH #694/#695: durable host fold and selection-seeded Find through native UI.
import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { remote } from "webdriverio";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";

const ROOT = path.resolve(import.meta.dirname, "..");
const OUT = path.resolve("artifacts/embed-find-694-695");
fs.mkdirSync(OUT, { recursive: true });
const RUN = fs.mkdtempSync(path.join(OUT, "native-")), GRAPH = path.join(RUN, "graph");
const org = process.env.E2E_FORMAT === "org";
const ext = org ? "org" : "md";
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(RUN, dir));
fs.writeFileSync(path.join(GRAPH, "logseq/config.edn"), "{}\n");
const source = org ? "* needle source body\n* needle second match\n" : "- needle source body\n- needle second match\n";
const sourcePath = path.join(GRAPH, `pages/Source.${ext}`), hostPath = path.join(GRAPH, `pages/Host.${ext}`);
fs.writeFileSync(sourcePath, source);
fs.writeFileSync(hostPath, org ? "* {{embed [[Source]]}}\n** host child\n* [[Host]]\n" : "- {{embed [[Source]]}}\n  - host child\n- [[Host]]\n");
const nestedSource = org ? "* {{embed [[Source]]}}\n" : "- {{embed [[Source]]}}\n";
const nestedSourcePath = path.join(GRAPH, `pages/NestedSource.${ext}`);
fs.writeFileSync(nestedSourcePath, nestedSource);
fs.writeFileSync(path.join(GRAPH, `pages/NestedHost.${ext}`), org ? "* {{embed [[NestedSource]]}}\n" : "- {{embed [[NestedSource]]}}\n");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const TD = process.env.TAURI_DRIVER || path.join(process.env.CARGO_HOME || "", "bin/tauri-driver");
const port = Number(process.env.E2E_DRIVER_PORT || 4786);
const env = { ...process.env, TINE_GRAPH: GRAPH, XDG_DATA_HOME: path.join(RUN, "data"), XDG_CONFIG_HOME: path.join(RUN, "config"), XDG_CACHE_HOME: path.join(RUN, "cache"), WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const log = fs.openSync(path.join(RUN, "driver.log"), "w");
const driver = spawn(TD, ["--port", String(port), "--native-port", String(port + 1), "--native-driver", "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log], detached: true });
let browser, stage = "startup";
try {
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 2, connectionRetryTimeout: 30000, capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } } });
  await ensureMainWindow(browser);
  await browser.$(".app-container").waitForExist({ timeout: 30000 });
  stage = "nested occurrence keyboard fold preserves source";
  await openPageByName(browser, "NestedHost");
  await browser.$(".embed-page .embed-page .block-content").waitForExist({ timeout: 15000 });
  // Editing an embedded source row must use its occurrence's disclosure owner.
  await browser.execute(() => {
    const row = document.querySelector(".main-content .embed-page .ls-block");
    row.querySelector(".block-content").dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
    document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
  });
  await browser.$("textarea.block-editor").waitForExist();
  await browser.keys(["Control", "ArrowUp", "NULL"]);
  await browser.keys(["Escape"]);
  await browser.waitUntil(() => browser.execute(() => {
    const embed = document.querySelector(".main-content .embed-page .embed-page");
    return embed?.textContent.includes("Source") && !embed.textContent.includes("source body");
  }), { timeout: 10000 });
  assert.equal(fs.readFileSync(nestedSourcePath, "utf8"), nestedSource);
  await browser.execute(() => {
    const row = document.querySelector(".main-content .embed-page .ls-block");
    row.querySelector(".block-content").dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
    document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
  });
  await browser.$("textarea.block-editor").waitForExist();
  await browser.keys(["Control", "ArrowDown", "NULL"]);
  await browser.keys(["Escape"]);
  await browser.$(".embed-page .embed-page .block-content").waitForExist();
  await browser.saveScreenshot(path.join(RUN, "nested-fold.png"));
  assert.equal(fs.readFileSync(nestedSourcePath, "utf8"), nestedSource);
  assert.equal(fs.readFileSync(sourcePath, "utf8"), source);
  await openPageByName(browser, "Host");
  stage = "page embed fold";
  await browser.$(".embed-page .block-content").waitForExist({ timeout: 15000 });
  await browser.execute(() => document.querySelector(".embed-page").closest(".ls-block").querySelector(".collapse-toggle").click());
  await browser.waitUntil(() => browser.execute(() => {
    const host = document.querySelector(".embed-page").closest(".ls-block");
    return host.textContent.includes("Source") && !host.textContent.includes("source body") && !host.textContent.includes("host child");
  }), { timeout: 10000 });
  await browser.waitUntil(() => fs.readFileSync(hostPath, "utf8").toLowerCase().includes(org ? ":collapsed: true" : "collapsed:: true"), { timeout: 15000 });
  assert.equal(fs.readFileSync(sourcePath, "utf8"), source);
  await openPageByName(browser, "Source"); await openPageByName(browser, "Host");
  assert.equal(await browser.$(".embed-page .block-content").isExisting(), false, "host fold survives navigation");
  await browser.execute(() => document.querySelector(".embed-page").closest(".ls-block").querySelector(".collapse-toggle").click());
  await browser.$(".embed-page .block-content").waitForExist();
  // The ordinary page link opens a second instance in the right sidebar.
  await browser.execute(() => {
    const link = [...document.querySelectorAll(".page-ref")].find(el => el.textContent.includes("Host"));
    link.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, shiftKey: true }));
  });
  await browser.$(".right-sidebar .embed-page").waitForExist({ timeout: 10000 });
  await browser.execute(() => document.querySelector(".right-sidebar .embed-page").closest(".ls-block").querySelector(".collapse-toggle").click());
  await browser.waitUntil(() => browser.execute(() => !document.querySelector(".right-sidebar .embed-page .block-content")), { timeout: 10000 });
  await browser.saveScreenshot(path.join(RUN, "fold.png"));
  stage = "rendered selection Find";
  await openPageByName(browser, "Source");
  await browser.execute(() => {
    const text = document.querySelector(".main-content .block-content");
    const walker = document.createTreeWalker(text, NodeFilter.SHOW_TEXT);
    let node;
    while ((node = walker.nextNode()) && !node.nodeValue.includes("needle")) {}
    if (!node) throw new Error("source text is not rendered");
    const start = node.nodeValue.indexOf("needle"), range = document.createRange();
    range.setStart(node, start); range.setEnd(node, start + 6);
    getSelection().removeAllRanges(); getSelection().addRange(range);
  });
  await browser.keys(["Control", "f", "NULL"]);
  await browser.waitUntil(() => browser.execute(() => document.querySelector(".inpage-find-input")?.value === "needle" && document.querySelector(".inpage-find-count")?.textContent.startsWith("1 /")), { timeout: 10000 });
  await browser.keys(["Escape"]);
  stage = "editor selection Find";
  await browser.$(".main-content .block-content").click();
  await browser.$("textarea.block-editor").waitForExist();
  await browser.execute(() => { const el = document.querySelector("textarea.block-editor"); el.focus(); el.setSelectionRange(7, 13); });
  await browser.keys(["Control", "f", "NULL"]);
  await browser.waitUntil(() => browser.execute(() => document.querySelector(".inpage-find-input")?.value === "source" && document.querySelector(".inpage-find-count")?.textContent.startsWith("1 /")), { timeout: 10000 });
  await browser.saveScreenshot(path.join(RUN, "find.png"));
  assert.equal(fs.readFileSync(sourcePath, "utf8"), source);
  assert.equal(fs.readFileSync(nestedSourcePath, "utf8"), nestedSource);
  fs.writeFileSync(path.join(RUN, "receipt.json"), JSON.stringify({ tested: "6ca84ddde + fix round 3 diff", app: APP, format: ext, outcome: "PASS" }, null, 2));
  console.log(`PASS: native page-embed fold and selected-text Find (${RUN})`);
} catch (error) {
  try { await browser?.saveScreenshot(path.join(RUN, "failure.png")); } catch {}
  try { fs.writeFileSync(path.join(RUN, "failure-dom.html"), await browser.getPageSource()); } catch {}
  fs.writeFileSync(path.join(RUN, "failure.txt"), `Tested: 6ca84ddde + fix round 3 diff\nJourney: ${stage}\nExpected: occurrence-only fold, unchanged embedded sources, host-only saved fold and selected text in Find with first match active.\nObserved: ${error.stack}\nClassification: ambiguous until screenshot and log review.\n`);
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
