// GX4 S4: real query readers, native typing, keyed vs full-refresh control.
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

const out = path.resolve(process.env.QUERY_OUT || "artifacts/gx4-s4/native");
fs.mkdirSync(out, { recursive: true });
const run = fs.mkdtempSync(path.join(out, "run-"));
const graph = process.env.QUERY_GRAPH ? path.resolve(process.env.QUERY_GRAPH) : path.join(run, "graph");
assert(graph.startsWith(path.resolve("artifacts/gx4-s4") + path.sep), "only lane-generated scratch graphs may be changed");
for (const directory of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, directory), { recursive: true });
for (const directory of ["data", "config", "cache"]) fs.mkdirSync(path.join(run, directory));
const pageFile = name => path.join(graph, "pages", `${name}.md`);
fs.writeFileSync(pageFile("Query target"), "- Target seed\n");
fs.writeFileSync(pageFile("Query source"), "- TODO [[Query target]] original matching result\n");
fs.writeFileSync(pageFile("Query unrelated"), "- Unrelated typing body\n");
const journal = path.join(graph, "journals", "2026_10_25.md");
const narrowForm = "(and (task TODO) [[Query target]])";
const linksForm = "@page and any(links, name = 'Query target')";
fs.writeFileSync(journal, `- [[Query unrelated]]\n- {{query ${narrowForm}}}\n- {{tine-query ${linksForm}}}\n`);
const events = path.join(run, "host-events.ndjson");
const app = path.resolve(process.env.TINE_APP || "target/debug/tine");
const port = Number(process.env.E2E_DRIVER_PORT || 4892);
const env = { ...process.env, TINE_GRAPH: graph, TINE_BENCH_EVENTS: events,
  XDG_DATA_HOME: path.join(run, "data"), XDG_CONFIG_HOME: path.join(run, "config"), XDG_CACHE_HOME: path.join(run, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const log = fs.openSync(path.join(run, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", webdriverServerArgs(port, port + 1, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"),
  { env, detached: true, stdio: ["ignore", log, log] });
const report = { testedCommit: execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim(), diff: "working tree", app,
  graph, loadBefore: execFileSync("uptime", { encoding: "utf8" }).trim(), passed: [], measurements: [] };
let browser, stage = "startup";
const scope = ".right-sidebar";
async function shown(selector, text) {
  await browser.waitUntil(() => browser.execute((selector, text) => [...document.querySelectorAll(selector)].some(el => el.textContent.includes(text)), selector, text),
    { timeout: 60000, timeoutMsg: `${selector} did not show ${text}` });
}
async function debugMode(on) {
  await browser.$('button[title="Settings (t s)"]').click();
  await browser.$(".settings-nav-item").waitForExist();
  await browser.execute(() => [...document.querySelectorAll(".settings-nav-item")].find(el => el.textContent.includes("Help & diagnostics")).click());
  await browser.$(".diagnostics-tab input[type=checkbox]").waitForExist();
  await browser.execute(on => { const input = document.querySelector(".diagnostics-tab input[type=checkbox]"); if (input.checked !== on) input.click(); }, on);
  await browser.execute(() => document.querySelector(".modal-overlay").click());
  await browser.$(".settings-modal").waitForExist({ reverse: true });
}
async function focusSource() {
  await browser.execute(() => {
    const el = document.querySelector(".right-sidebar .block-content-wrapper");
    el.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
  });
  await browser.$(`${scope} textarea.block-editor`).waitForExist();
  await browser.keys(["Control", "End"]);
}
async function calls() { return browser.execute(() => window.__queryIpc); }
const question = call => call.args.query?.source?.original === narrowForm ? "narrow"
  : call.args.query?.source?.original === linksForm ? "links" : "other";
async function measure(full) {
  await debugMode(full); await delay(2000); await focusSource();
  await browser.execute(() => { window.__queryIpc = []; });
  const start = Date.now();
  const seconds = Number(process.env.QUERY_SECONDS || 8);
  const actions = [];
  for (let i = 0; i < seconds * 20; i++) actions.push({ type: "keyDown", value: "x" }, { type: "keyUp", value: "x" }, { type: "pause", duration: 50 });
  await browser.performActions([{ type: "key", id: "continuous-query-typing", actions }]); await browser.releaseActions();
  const end = Date.now();
  const during = await calls();
  const queryCalls = during.filter(c => c.command === "query_run");
  assert(during.some(c => c.command === "page_submit"), "instrumentation must see persisted native typing");
  if (!full) assert.equal(queryCalls.length, 0, "neither query may rerun during unrelated continuous typing");
  await waitForFileText(pageFile("Query unrelated"), text => text.includes("xxxxxxxx"));
  await delay(3500);
  await browser.waitUntil(async () => (await calls()).filter(c => c.command === "query_run").every(c => c.finishedAt), { timeout: 30000 });
  const all = await calls();
  const settledAt = Date.now();
  const after = all.filter(c => c.command === "query_run");
  if (!full) {
    assert.equal(after.filter(c => question(c) === "narrow").length, 0, "unrelated edits must not rerun the narrow query");
    assert.equal(after.filter(c => question(c) === "links").length, 1, "conservative links invalidation must coalesce into one run after the pause");
  }
  const host = fs.readFileSync(events, "utf8").trim().split("\n").filter(Boolean).map(line => JSON.parse(line))
    .filter(e => e.ev === "mark" && e.name === "query_run_us" && e.t >= start && e.t <= settledAt);
  const duringHost = host.filter(e => e.t <= end);
  const elapsedSeconds = (end - start) / 1000;
  report.measurements.push({ mode: full ? "full-refresh control" : "keyed", startedAt: start, stoppedAt: end, settledAt, elapsedSeconds, loadAverage: os.loadavg(),
    queryIpcCounts: Object.fromEntries(["query_parse", "query_run", "query_registry", "register_query_subscription"].map(command => [command, during.filter(c => c.command === command).length])),
    queryCallsDuringTyping: queryCalls.length, queryCallsPerSecond: queryCalls.length / elapsedSeconds,
    hostQueryCallsDuringTyping: duringHost.length, hostQueryMsPerSecond: duringHost.reduce((sum, e) => sum + e.n / 1000, 0) / elapsedSeconds,
    targetCallsDuringTyping: Object.fromEntries(["narrow", "links", "other"].map(kind => [kind, queryCalls.filter(c => question(c) === kind).length])),
    targetCallsIncludingPause: Object.fromEntries(["narrow", "links", "other"].map(kind => [kind, after.filter(c => question(c) === kind).length])),
    hostQueryMsIncludingPause: host.reduce((sum, e) => sum + e.n / 1000, 0), queryCallsIncludingPause: after.length });
  fs.writeFileSync(path.join(run, `${full ? "before" : "after"}-ipc.json`), JSON.stringify(all, null, 2));
}
try {
  for (let attempt = 0; ; attempt++) {
    try { if ((await fetch(`http://127.0.0.1:${port}/status`)).ok) break; } catch {}
    if (attempt > 100) throw new Error("WebDriver did not start");
    await delay(100);
  }
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 0, connectionRetryTimeout: 120000,
    capabilities: tauriCapabilities(app, "query-subscriptions") });
  await ensureMainWindow(browser); await browser.setWindowSize(1600, 1100);
  await browser.$(".app-container").waitForExist({ timeout: 120000 });
  await openPageByName(browser, "Query target");
  // Open the physical journal by its standard display name; fixture date is fixed.
  await openPageByName(browser, "Oct 25th, 2026");
  await browser.waitUntil(async () => (await browser.$$(".main-content .query-block")).length === 2, { timeout: 120000 });
  await shown(".main-content .query-block", "original matching result");
  // Establish the required first-publication full baseline before counting.
  fs.writeFileSync(pageFile("Query source"), "- TODO [[Query target]] baseline matching result\n");
  await shown(".main-content .query-block", "baseline matching result"); await delay(2000);
  await browser.execute(() => {
    const original = window.fetch; window.__queryIpc = [];
    window.fetch = function(input, init) {
      const url = new URL(typeof input === "string" ? input : input.url);
      let recorded;
      if ((url.protocol === "ipc:" || url.hostname === "ipc.localhost") && window.__queryIpc.length < 10000) {
        let args = {}; if (typeof init?.body === "string") { try { args = JSON.parse(init.body); } catch {} }
        recorded = { at: Date.now(), command: decodeURIComponent(url.pathname.slice(1)), args };
        window.__queryIpc.push(recorded);
      }
      return original.call(this, input, init).then(response => { if (recorded) recorded.finishedAt = Date.now(); return response; });
    };
    [...document.querySelectorAll(".main-content a.page-ref")].find(el => el.textContent.includes("Query unrelated"))
      .dispatchEvent(new MouseEvent("click", { bubbles: true, shiftKey: true, button: 0 }));
  });
  await browser.$(`${scope} .block-content-wrapper`).waitForExist();
  stage = "full-refresh typing control"; await measure(true); report.passed.push(stage);
  stage = "keyed unrelated typing and one quiet-period links rerun"; await measure(false); report.passed.push(stage);
  stage = "a matching native edit updates both query answers";
  await focusSource();
  await browser.execute(() => { window.__queryIpc = []; });
  const pasted = await browser.executeAsync(done => window.__TAURI_INTERNALS__.invoke("plugin:clipboard-manager|write_text", { text: "TODO [[Query target]] matching native result" }).then(() => done(true), () => done(false)));
  assert(pasted, "native clipboard preparation");
  await browser.keys(["Control", "a"]); await browser.keys(["Control", "v"]);
  await browser.execute(() => document.querySelector(".right-sidebar textarea.block-editor")?.blur());
  await waitForFileText(pageFile("Query unrelated"), text => text.includes("matching native result"));
  await shown(".main-content .query-block", "matching native result");
  await browser.waitUntil(async () => (await calls()).some(c => c.command === "query_run" && c.args.query?.source?.kind === "tql"), { timeout: 30000 });
  await shown(".main-content .query-page-link", "Query unrelated");
  const related = (await calls()).filter(c => c.command === "query_run");
  assert(related.some(c => question(c) === "narrow"), "matching edit reruns the narrow query");
  fs.writeFileSync(path.join(run, "related-ipc.json"), JSON.stringify(related, null, 2));
  report.passed.push(stage);
  fs.writeFileSync(path.join(run, "passed.html"), await browser.getPageSource());
} catch (error) {
  report.failureCapsule = { testedCommit: report.testedCommit, journey: stage,
    expected: "Unrelated typing leaves queries idle; relevant narrow answers refresh and graph-wide work coalesces after a pause", observed: String(error.stack || error),
    classification: "ambiguous", screenshot: null, log: "driver.log / failure.html" };
  if (browser) fs.writeFileSync(path.join(run, "failure.html"), await browser.getPageSource().catch(() => "unavailable"));
  process.exitCode = 1;
} finally {
  report.loadAfter = execFileSync("uptime", { encoding: "utf8" }).trim();
  fs.writeFileSync(path.join(run, "report.json"), JSON.stringify(report, null, 2)); console.log(JSON.stringify({ run, ...report }));
  await browser?.deleteSession().catch(() => {}); try { process.kill(-driver.pid, "SIGTERM"); } catch {} fs.closeSync(log);
}
