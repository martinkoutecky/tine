// D-20261102-PERF row 3: a page rename and a page delete publish a precise
// change summary. With Target shown in the right sidebar (its linked
// references and two mounted queries), renaming and then deleting Linker from
// its own title must update Target's linked references and the dependent
// query, and must not rerun the unrelated query. All writes come from the
// app's own rename and delete actions on a disposable graph; the account home
// is the e2e preload shim, never the user's.
// Usage: xvfb-run -a node scripts/e2e-page-lifecycle-summary.mjs (TINE_APP = the candidate binary).
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { execFileSync, spawn } from "node:child_process";
import { setTimeout as delay } from "node:timers/promises";
import { remote } from "webdriverio";
import { installAccountHome, assertAppLease } from "./lib/e2e-account-home.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { currentPageTitle, openPageByName } from "./lib/e2e-navigation.mjs";
import { answerNativeDialog } from "./lib/e2e-native-dialog.mjs";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";

const out = path.resolve(process.env.E2E_ARTIFACT_DIR || "target/artifacts/og-perf-page-summary/native");
fs.mkdirSync(out, { recursive: true });
const run = fs.mkdtempSync(path.join(out, "run-"));
const graph = path.join(run, "graph");
for (const directory of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, directory), { recursive: true });
for (const directory of ["data", "config", "cache"]) fs.mkdirSync(path.join(run, directory));
const owner = installAccountHome(run, run);
const pageFile = name => path.join(graph, "pages", `${name}.md`);
const dependent = "(and (task TODO) [[Target]])";
const unrelated = "(and (task NOW) [[Unrelated]])";
// The completion probe (`settle`): a page the journey rewrites from outside,
// shown by a third query, so its rendering proves every earlier publication
// was delivered and flushed to the mounted readers.
const sentinel = "(and (task LATER) [[Sentinel]])";
const targetText = `- target seed\n- {{query ${dependent}}}\n- {{query ${unrelated}}}\n- {{query ${sentinel}}}\n`;
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{:ref/linked-references-collapsed-threshold 100 :tine/guide-announced? true}\n");
fs.writeFileSync(pageFile("Target"), targetText);
fs.writeFileSync(pageFile("Linker"), "- TODO linker body [[Target]]\n");
fs.writeFileSync(pageFile("Unrelated"), "- NOW unrelated body [[Unrelated]]\n");
const app = path.resolve(process.env.TINE_APP || "target/release/tine");
const port = Number(process.env.E2E_DRIVER_PORT || 4932);
const env = { ...process.env, TINE_GRAPH: graph,
  XDG_DATA_HOME: path.join(run, "data"), XDG_CONFIG_HOME: path.join(run, "config"), XDG_CACHE_HOME: path.join(run, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const log = fs.openSync(path.join(run, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", webdriverServerArgs(port, port + 1, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"),
  { env, detached: true, stdio: ["ignore", log, log] });
const report = { testedCommit: execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim(), diff: "working tree", app, graph, passed: [], reruns: {} };
let browser, stage = "startup";
const side = ".right-sidebar";

/** The sidebar's linked-reference group titles and its query result texts. */
const shownState = () => browser.execute((side) => {
  const root = document.querySelector(side);
  const linked = root?.querySelector(".linked-references");
  return {
    linked: linked ? linked.textContent : null,
    queries: [...(root?.querySelectorAll(".query-block") ?? [])].map(el => el.textContent),
  };
}, side);
async function until(predicate, message) {
  await browser.waitUntil(async () => predicate(await shownState()), { timeout: 30000, interval: 200, timeoutMsg: message })
    .catch(async error => { throw new Error(`${error.message}; shown ${JSON.stringify(await shownState())}`); });
}
// WebKitWebDriver can stall on a screenshot after a resize; as the other
// native journeys do (e2e-comment-structure.mjs), capture Xvfb directly.
async function screenshot(name) {
  const file = path.join(run, `${name}.png`);
  if (process.platform === "linux") execFileSync("import", ["-window", "root", file], { timeout: 10_000 });
  else await browser.saveScreenshot(file);
}
/** Every recorded IPC call has answered. */
const ipcIdle = () => browser.waitUntil(() => browser.execute(() => (window.__queryIpc ?? []).every(c => c.finishedAt)),
  { timeout: 30000, interval: 100, timeoutMsg: "a recorded IPC call never answered" });
let sentinels = 0;
/** Completion condition instead of a fixed sleep (review F3). Once the
 * action's own calls (the rename or delete command, the reruns it started)
 * have answered, an outside write rewrites the sentinel. The store publishes
 * in revision order and the window applies and flushes summaries in order, so
 * when the sentinel query shows the new text, every publication of the action
 * has been delivered and flushed to its readers, and every rerun those caused
 * has been issued. The reruns then drain. A late rerun cannot escape the count. */
async function settle(label) {
  await ipcIdle();
  const mark = `sentinel-${++sentinels}-end`;
  fs.writeFileSync(pageFile("Sentinel"), `- LATER ${mark} [[Sentinel]]\n`);
  await until(s => s.queries.some(q => q.includes(mark)), `${label}: the sentinel publication never reached its query`);
  await ipcIdle();
}
const queryRuns = async () => browser.execute(() => window.__queryIpc.filter(c => c.command === "query_run"));
const runsOf = (calls, form) => calls.filter(c => JSON.stringify(c.args).includes(form.replace(/"/g, '\\"'))).length;

try {
  for (let attempt = 0; ; attempt++) {
    try { if ((await fetch(`http://127.0.0.1:${port}/status`)).ok) break; } catch {}
    if (attempt > 100) throw new Error("WebDriver did not start");
    await delay(100);
  }
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 0, connectionRetryTimeout: 120000,
    capabilities: tauriCapabilities(app, "page-lifecycle-summary") });
  await ensureMainWindow(browser); await browser.setWindowSize(1600, 1100);
  await browser.$(".app-container").waitForExist({ timeout: 120000 });
  await assertAppLease(owner);
  await openPageByName(browser, "Linker");
  // Target in the right sidebar, opened from Linker's own link.
  await browser.execute(() => {
    [...document.querySelectorAll(".main-content .page-ref")].find(el => el.textContent.includes("Target"))
      .dispatchEvent(new MouseEvent("click", { bubbles: true, shiftKey: true, button: 0 }));
  });
  await browser.$(`${side} .block-content-wrapper`).waitForExist({ timeout: 30000 });
  stage = "sidebar shows Linker in Target's linked references and the dependent query";
  await until(s => s.linked?.includes("Linker") && s.queries.some(q => q.includes("linker body"))
    && s.queries.some(q => q.includes("unrelated body")), "Target's readers never showed Linker");
  await screenshot("1-before");
  report.passed.push(stage);
  await browser.execute(() => {
    const original = window.fetch; window.__queryIpc = [];
    window.fetch = function(input, init) {
      const url = new URL(typeof input === "string" ? input : input.url);
      let recorded;
      if ((url.protocol === "ipc:" || url.hostname === "ipc.localhost") && window.__queryIpc.length < 10000) {
        let args = {}; if (typeof init?.body === "string") { try { args = JSON.parse(init.body); } catch {} }
        recorded = { at: Date.now(), command: decodeURIComponent(url.pathname.slice(1)), args, stack: new Error().stack };
        window.__queryIpc.push(recorded);
      }
      return original.call(this, input, init).then(response => { if (recorded) recorded.finishedAt = Date.now(); return response; });
    };
  });

  // Each publication's change summary, as the window receives it (evidence
  // for which readers it named).
  await browser.executeAsync(done => {
    window.__summaries = [];
    const internals = window.__TAURI_INTERNALS__;
    const listen = name => internals.invoke("plugin:event|listen", { event: name, target: { kind: "Any" },
      handler: internals.transformCallback(event => {
        window.__summaries.push({ at: Date.now(), event: name, summary: event.payload?.answers?.summary ?? null });
      }) });
    // Own publications arrive as a bulk event with no pages (watcher.rs).
    Promise.all(["graph-changed", "graph-changed-bulk"].map(listen)).then(() => done(true), error => done(String(error)));
  });
  // A window's first publication is full by design (`changeSummary.ts`: no
  // baseline yet). Give it one, from an unrelated outside write (the first
  // sentinel), so the operations below are measured against a baseline, then
  // start counting once its full refresh has drained.
  await settle("warm-up");
  await browser.execute(() => { window.__queryIpc = []; window.__summaries = []; });

  stage = "rename from the title updates Target's linked references and query";
  await browser.execute(() => document.querySelector(".main-content h1.page-title")
    .dispatchEvent(new MouseEvent("dblclick", { bubbles: true, cancelable: true, view: window })));
  await browser.$(".main-content .page-title-input").waitForExist({ timeout: 5000 });
  await browser.execute(() => {
    const input = document.querySelector(".main-content .page-title-input");
    input.focus(); input.value = "Linker Renamed";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
  });
  await browser.waitUntil(() => fs.existsSync(pageFile("Linker Renamed")) && !fs.existsSync(pageFile("Linker")),
    { timeout: 30000, timeoutMsg: "rename did not move the file" });
  await until(s => s.linked?.includes("Linker Renamed") && s.queries.some(q => q.includes("linker body")),
    "Target's linked references did not follow the rename");
  await settle("rename");
  await screenshot("2-after-rename");
  let calls = await queryRuns();
  report.reruns.rename = { dependent: runsOf(calls, dependent), unrelated: runsOf(calls, unrelated), all: calls.length };
  fs.writeFileSync(path.join(run, "rename-calls.json"), JSON.stringify(await browser.execute(() => window.__queryIpc), null, 2));
  fs.writeFileSync(path.join(run, "rename-summaries.json"), JSON.stringify(await browser.execute(() => window.__summaries.splice(0)), null, 2));
  assert.equal(report.reruns.rename.unrelated, 0, "the rename reran the unrelated mounted query");
  report.passed.push(stage);

  stage = "delete from the page menu removes Linker from Target's linked references and query";
  await browser.execute(() => { window.__queryIpc = []; });
  await openPageByName(browser, "Linker Renamed");
  await browser.execute(() => document.querySelector(".main-content h1.page-title")
    .dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 300, clientY: 200 })));
  await browser.$('[data-page-action-id="delete-page"]').waitForExist({ timeout: 5000 });
  await browser.execute(() => document.querySelector('[data-page-action-id="delete-page"]').click());
  await answerNativeDialog("yes", { env });
  await browser.waitUntil(() => !fs.existsSync(pageFile("Linker Renamed")), { timeout: 30000, timeoutMsg: "delete did not remove the file" });
  await until(s => !(s.linked ?? "").includes("Linker") && !s.queries.some(q => q.includes("linker body"))
    && s.queries.some(q => q.includes("unrelated body")), "Target's readers still show the deleted page");
  await settle("delete");
  await screenshot("3-after-delete");
  calls = await queryRuns();
  report.reruns.delete = { dependent: runsOf(calls, dependent), unrelated: runsOf(calls, unrelated), all: calls.length };
  fs.writeFileSync(path.join(run, "delete-calls.json"), JSON.stringify(await browser.execute(() => window.__queryIpc), null, 2));
  fs.writeFileSync(path.join(run, "delete-summaries.json"), JSON.stringify(await browser.execute(() => window.__summaries.splice(0)), null, 2));
  assert.equal(report.reruns.delete.unrelated, 0, "the delete reran the unrelated mounted query");
  assert.notEqual(await currentPageTitle(browser), "Linker Renamed", "the deleted page stayed routed");
  assert.equal(fs.readFileSync(pageFile("Target"), "utf8"), targetText);
  report.passed.push(stage);
} catch (error) {
  report.failure = { journey: stage, observed: String(error.stack || error) };
  if (browser) {
    fs.writeFileSync(path.join(run, "failure.html"), await browser.getPageSource().catch(() => "unavailable"));
    await screenshot("failure").catch(() => {});
  }
  process.exitCode = 1;
} finally {
  fs.writeFileSync(path.join(run, "report.json"), JSON.stringify(report, null, 2)); console.log(JSON.stringify({ run, ...report }));
  await browser?.deleteSession().catch(() => {}); try { process.kill(-driver.pid, "SIGTERM"); } catch {} fs.closeSync(log);
}
