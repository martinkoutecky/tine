// GX4 S3: live targets and a per-save IPC A/B against the debug full-refresh mode.
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { execFileSync, spawn } from "node:child_process";
import { setTimeout as delay } from "node:timers/promises";
import { remote } from "webdriverio";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";

const out = path.resolve(process.env.KEYED_OUT || "artifacts/gx4-s3/native");
fs.mkdirSync(out, { recursive: true });
const run = fs.mkdtempSync(path.join(out, "run-"));
const graph = path.join(run, "graph");
for (const directory of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, directory), { recursive: true });
for (const directory of ["data", "config", "cache"]) fs.mkdirSync(path.join(run, directory));
const ids = Array.from({ length: 48 }, (_, i) => `33333333-3333-4333-8333-${String(i + 1).padStart(12, "0")}`);
const pageFile = name => path.join(graph, "pages", `${name}.md`);
const source = text => ids.map((id, i) => `- ${i ? `Other target ${i}` : text}\n  id:: ${id}\n`).join("");
fs.writeFileSync(pageFile("Source"), source("Original reference"));
fs.writeFileSync(pageFile("Embed page"), "- Original page body\n");
fs.writeFileSync(pageFile("Icon target"), "icon:: A\n\n- Icon source\n");
fs.writeFileSync(pageFile("Unrelated"), "- Typing here\n");
fs.writeFileSync(pageFile("Keyed refresh"), `- [[Unrelated]]\n- ${ids.map(id => `((${id}))`).join(" ")}\n- {{embed ((${ids[0]}))}}\n- {{embed [[Embed page]]}}\n- [[Icon target]]\n`);
const app = path.resolve(process.env.TINE_APP || "target/debug/tine");
const port = Number(process.env.E2E_DRIVER_PORT || 4882);
const env = { ...process.env, TINE_GRAPH: graph, XDG_DATA_HOME: path.join(run, "data"), XDG_CONFIG_HOME: path.join(run, "config"),
  XDG_CACHE_HOME: path.join(run, "cache"), WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const log = fs.openSync(path.join(run, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", webdriverServerArgs(port, port + 1, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"),
  { env, detached: true, stdio: ["ignore", log, log] });
const report = { testedCommit: execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim(), diff: "working tree", app, loadAverage: os.loadavg(), targets: ids.length, passed: [], measurements: [] };
let browser, stage = "startup";

async function shown(selector, text) {
  await browser.waitUntil(() => browser.execute((selector, text) => [...document.querySelectorAll(selector)].some(element => element.textContent.includes(text)), selector, text),
    { timeout: 30000, timeoutMsg: `${selector} did not show ${text}` });
}
async function debugMode(on) {
  await browser.$('button[title="Settings (t s)"]').click();
  await browser.$(".settings-nav-item").waitForExist();
  await browser.execute(() => [...document.querySelectorAll(".settings-nav-item")].find(element => element.textContent.includes("Help & diagnostics")).click());
  await browser.$(".diagnostics-tab input[type=checkbox]").waitForExist();
  await browser.execute(on => {
    const input = document.querySelector(".diagnostics-tab input[type=checkbox]");
    if (input.checked !== on) input.click();
  }, on);
  await browser.execute(() => document.querySelector(".modal-overlay").click());
  await browser.$(".settings-modal").waitForExist({ reverse: true });
}
async function measure(full, token) {
  await debugMode(full);
  await delay(1500);
  await browser.execute(() => {
    window.__keyedIpc = [];
    document.querySelector(".right-sidebar .block-content-wrapper").dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
  });
  await browser.$(".right-sidebar textarea.block-editor").waitForExist();
  await browser.keys(["Control", "End"]); await browser.keys(token);
  await waitForFileText(pageFile("Unrelated"), text => text.includes(token));
  await delay(2200);
  const calls = await browser.execute(() => window.__keyedIpc);
  const relevant = calls.filter(({ command, args }) => command === "resolve_blocks" && args.uuids?.some(id => id.startsWith("33333333-"))
    || command === "page_icons" && args.names?.includes("Icon target")
    || command === "get_page" && ["Embed page", "Source"].includes(args.name));
  const counts = Object.fromEntries(["resolve_blocks", "page_icons", "get_page"].map(command => [command, relevant.filter(call => call.command === command).length]));
  fs.writeFileSync(path.join(run, `${full ? "before" : "after"}-ipc.json`), JSON.stringify(calls, null, 2));
  assert(calls.some(call => call.command === "page_submit"), "IPC instrumentation must observe the real save already proven on disk");
  report.measurements.push({ mode: full ? "full-refresh control" : "keyed", loadAverage: os.loadavg(), counts, resolvedIds: relevant.filter(call => call.command === "resolve_blocks").flatMap(call => call.args.uuids).length });
  if (full) for (const command of Object.keys(counts)) assert(counts[command] > 0, `${command} control did not exercise the target`);
  else for (const command of Object.keys(counts)) assert.equal(counts[command], 0, `${command} refetched an unrelated target`);
}

try {
  for (let attempt = 0; ; attempt++) {
    try { if ((await fetch(`http://127.0.0.1:${port}/status`)).ok) break; } catch {}
    if (attempt > 100) throw new Error("WebDriver did not start");
    await delay(100);
  }
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 0, connectionRetryTimeout: 60000,
    capabilities: tauriCapabilities(app, "keyed-refresh") });
  await ensureMainWindow(browser); await browser.setWindowSize(1600, 1000);
  await browser.$(".app-container").waitForExist({ timeout: 60000 });
  await openPageByName(browser, "Keyed refresh");
  await shown(".main-content .block-ref", "Original reference");
  await shown(".main-content .embed-block", "Original reference");
  await shown(".main-content .embed-page", "Original page body");
  await shown(".main-content .page-ref-icon", "A");
  await delay(1500);
  await browser.execute(() => {
    // Tauri freezes invoke/ipc/postMessage. Observe its native custom-protocol
    // fetch transport, preserving the request and response unchanged.
    const original = window.fetch;
    window.__keyedIpc = [];
    window.fetch = function(input, init) {
      const url = new URL(typeof input === "string" ? input : input.url);
      if ((url.protocol === "ipc:" || url.hostname === "ipc.localhost") && window.__keyedIpc.length < 2000) {
        let args = {};
        if (typeof init?.body === "string") { try { args = JSON.parse(init.body); } catch {} }
        window.__keyedIpc.push({ command: decodeURIComponent(url.pathname.slice(1)), args });
      }
      return original.call(this, input, init);
    };
  });
  stage = "external edits update each rendered target";
  fs.writeFileSync(pageFile("Source"), source("Updated reference"));
  await shown(".main-content .block-ref", "Updated reference");
  await shown(".main-content .embed-block", "Updated reference");
  fs.writeFileSync(pageFile("Embed page"), "- Updated page body\n");
  await shown(".main-content .embed-page", "Updated page body");
  fs.writeFileSync(pageFile("Icon target"), "icon:: B\n\n- Icon source\n");
  await shown(".main-content .page-ref-icon", "B");
  report.passed.push(stage);
  console.log(`PASS: ${stage}`);
  // Shift-click a page link: leave all target readers mounted in the main pane.
  await browser.execute(() => [...document.querySelectorAll(".main-content a.page-ref")].find(element => element.textContent.includes("Unrelated"))
    .dispatchEvent(new MouseEvent("click", { bubbles: true, shiftKey: true, button: 0 })));
  await browser.$(".right-sidebar .block-content-wrapper").waitForExist();
  stage = "full-refresh control on an unrelated typed save";
  await measure(true, " control-save"); report.passed.push(stage); console.log(`PASS: ${stage}`);
  stage = "keyed refresh on an unrelated typed save";
  await measure(false, " keyed-save"); report.passed.push(stage); console.log(`PASS: ${stage}`);
  await shown(".main-content .block-ref", "Updated reference");
  await shown(".main-content .embed-page", "Updated page body");
  await shown(".main-content .page-ref-icon", "B");
  await browser.saveScreenshot(path.join(run, "passed.png"));
} catch (error) {
  report.failureCapsule = { testedCommit: report.testedCommit, journey: stage, expected: "Target edits update each surface; an unrelated typed save fetches none of those targets", observed: String(error.stack || error), classification: "ambiguous", screenshot: "failure.png", log: "driver.log" };
  await browser?.saveScreenshot(path.join(run, "failure.png")).catch(() => {});
  if (browser) fs.writeFileSync(path.join(run, "failure.html"), await browser.getPageSource().catch(() => "unavailable"));
  process.exitCode = 1;
} finally {
  fs.writeFileSync(path.join(run, "report.json"), JSON.stringify(report, null, 2));
  console.log(JSON.stringify({ run, ...report }));
  await browser?.deleteSession().catch(() => {});
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  fs.closeSync(log);
}
