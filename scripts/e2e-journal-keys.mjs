// GX4 S5: a journal with Upcoming, native continuous typing and live day changes.
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { execFileSync, spawn } from "node:child_process";
import { setTimeout as delay } from "node:timers/promises";
import { remote } from "webdriverio";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";

const out = path.resolve(process.env.JOURNAL_OUT || "artifacts/gx4-s5/native-after");
const graph = path.resolve(process.env.JOURNAL_GRAPH || "artifacts/gx4-s5/graph");
assert(graph.startsWith(path.resolve("artifacts/gx4-s5") + path.sep), "only generated lane graphs may be changed");
fs.mkdirSync(out, { recursive: true });
const run = fs.mkdtempSync(path.join(out, "run-"));
for (const directory of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, directory), { recursive: true });
for (const directory of ["data", "config", "cache"]) fs.mkdirSync(path.join(run, directory));
const pageFile = name => path.join(graph, "pages", `${name}.md`);
const now = new Date();
const stem = date => `${date.getFullYear()}_${String(date.getMonth() + 1).padStart(2, "0")}_${String(date.getDate()).padStart(2, "0")}`;
const journalFile = path.join(graph, "journals", `${stem(now)}.md`);
fs.writeFileSync(pageFile("S5 typing"), "- Unrelated typing target\n");
fs.writeFileSync(pageFile("S5 dates"), "- Initial date source\n");
fs.writeFileSync(pageFile("S5 sections"), "tine/journal-sections:: true\n\n- {{upcoming}}\n");
fs.writeFileSync(journalFile, "- Journal typing target\n- [[S5 typing]]\n");
const app = path.resolve(process.env.TINE_APP || "target/debug/tine");
const port = Number(process.env.E2E_DRIVER_PORT || 4902);
const env = { ...process.env, TINE_GRAPH: graph, XDG_DATA_HOME: path.join(run, "data"), XDG_CONFIG_HOME: path.join(run, "config"),
  XDG_CACHE_HOME: path.join(run, "cache"), WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const log = fs.openSync(path.join(run, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", webdriverServerArgs(port, port + 1, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"),
  { env, detached: true, stdio: ["ignore", log, log] });
const fullControl = !!process.env.JOURNAL_FULL_CONTROL;
const report = { testedCommit: execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim(), diff: "working tree", mode: fullControl ? "full-refresh control" : "keyed",
  app, graph, loadBefore: execFileSync("uptime", { encoding: "utf8" }).trim(), passed: [], measurements: [] };
let browser, stage = "startup";
const calls = () => browser.execute(() => window.__journalIpc);
async function debugMode(on) {
  await browser.$('button[title="Settings (t s)"]').click();
  await browser.$(".settings-nav-item").waitForExist();
  await browser.execute(() => [...document.querySelectorAll(".settings-nav-item")].find(el => el.textContent.includes("Help & diagnostics")).click());
  await browser.$(".diagnostics-tab input[type=checkbox]").waitForExist();
  await browser.execute(on => { const input = document.querySelector(".diagnostics-tab input[type=checkbox]"); if (input.checked !== on) input.click(); }, on);
  await browser.execute(() => document.querySelector(".modal-overlay").click());
  await browser.$(".settings-modal").waitForExist({ reverse: true });
}
try {
  for (let attempt = 0; ; attempt++) {
    try { if ((await fetch(`http://127.0.0.1:${port}/status`)).ok) break; } catch {}
    if (attempt > 100) throw new Error("WebDriver did not start");
    await delay(100);
  }
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 0, connectionRetryTimeout: 120000,
    capabilities: tauriCapabilities(app, "journal-keys") });
  await ensureMainWindow(browser); await browser.setWindowSize(1600, 1100);
  await browser.$(".app-container").waitForExist({ timeout: 120000 });
  const title = `${now.toLocaleString("en-US", { month: "short" })} ${now.getDate()}${[11, 12, 13].includes(now.getDate()) ? "th" : ({ 1: "st", 2: "nd", 3: "rd" }[now.getDate() % 10] || "th")}, ${now.getFullYear()}`;
  await openPageByName(browser, title);
  await browser.waitUntil(async () => (await browser.$$(".main-content .block-content-wrapper")).length >= 2, { timeout: 120000 });
  // A fast journal body can render before the whole-graph warm publication.
  // The actual sections answer proves graph-backed discovery has settled;
  // counting its initial full publication as a typing save invalidates the A/B.
  await browser.$('.today-sections[data-sections-page="S5 sections"]').waitForExist({ timeout: 120000 });
  // Establish the receiver's first-publication full baseline.
  fs.writeFileSync(pageFile("S5 dates"), "- Baseline date source\n");
  await delay(5000);
  await browser.execute(() => {
    const original = window.fetch; window.__journalIpc = [];
    window.fetch = function(input, init) {
      const url = new URL(typeof input === "string" ? input : input.url);
      let recorded;
      if ((url.protocol === "ipc:" || url.hostname === "ipc.localhost") && window.__journalIpc.length < 10000) {
        let args = {}; if (typeof init?.body === "string") { try { args = JSON.parse(init.body); } catch {} }
        recorded = { at: Date.now(), command: decodeURIComponent(url.pathname.slice(1)), args };
        window.__journalIpc.push(recorded);
      }
      return original.call(this, input, init).then(response => { if (recorded) recorded.finishedAt = Date.now(); return response; });
    };
    [...document.querySelectorAll(".main-content a.page-ref")].find(el => el.textContent.includes("S5 typing"))
      .dispatchEvent(new MouseEvent("click", { bubbles: true, shiftKey: true, button: 0 }));
  });
  await browser.$(".right-sidebar .block-content-wrapper").waitForExist();
  await debugMode(fullControl);
  await delay(2000);
  stage = "continuous native typing beside a journal";
  await browser.execute(() => document.querySelector(".right-sidebar .block-content-wrapper")
    .dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 })));
  await browser.$(".right-sidebar textarea.block-editor").waitForExist();
  await browser.keys(["Control", "End"]);
  await browser.execute(() => { window.__journalIpc = []; });
  const loadBefore = execFileSync("uptime", { encoding: "utf8" }).trim();
  const startedAt = Date.now(), seconds = Number(process.env.JOURNAL_SECONDS || 10);
  const actions = [];
  for (let i = 0; i < seconds * 20; i++) actions.push({ type: "keyDown", value: "x" }, { type: "keyUp", value: "x" }, { type: "pause", duration: 50 });
  await browser.performActions([{ type: "key", id: "journal-typing", actions }]); await browser.releaseActions();
  const stoppedAt = Date.now(), during = await calls();
  assert(during.some(c => c.command === "page_submit"), "native typing must persist through the host");
  await browser.execute(() => document.querySelector(".right-sidebar textarea.block-editor")?.blur());
  await waitForFileText(pageFile("S5 typing"), text => text.includes("xxxxxxxx"));
  await delay(3500);
  const all = await calls();
  const elapsedSeconds = (stoppedAt - startedAt) / 1000;
  report.measurements.push({ startedAt, stoppedAt, elapsedSeconds, loadBefore, loadAfter: execFileSync("uptime", { encoding: "utf8" }).trim(), ipcCalls: during.length, ipcCallsPerSecond: during.length / elapsedSeconds,
    countsDuringTyping: Object.fromEntries([...new Set(during.map(c => c.command))].sort().map(command => [command, during.filter(c => c.command === command).length])),
    feedCallsIncludingPause: all.filter(c => c.command === "journal_feed_page").length });
  fs.writeFileSync(path.join(run, "typing-ipc.json"), JSON.stringify(all, null, 2));
  if (!fullControl) assert.equal(all.filter(c => c.command === "journal_feed_page").length, 0, "unrelated saves must leave Upcoming alone");
  report.passed.push(stage);
  stage = "a related external date edit updates Upcoming";
  await browser.execute(() => { window.__journalIpc = []; });
  fs.writeFileSync(pageFile("S5 dates"), "- [[2099-12-30]] A future day\n");
  await browser.waitUntil(async () => (await calls()).some(c => c.command === "journal_feed_page" && c.finishedAt), { timeout: 60000 });
  await browser.$(".journal-upcoming").waitForExist();
  await browser.execute(() => { document.querySelector(".journal-upcoming").open = true; });
  await browser.waitUntil(async () => (await browser.$(".journal-upcoming").getText()).includes("2099"), { timeout: 30000 });
  report.passed.push(stage);
  fs.writeFileSync(path.join(run, "passed.html"), await browser.getPageSource());
} catch (error) {
  report.failureCapsule = { testedCommit: report.testedCommit, journey: stage, expected: "Unrelated typing preserves journal answers; related day edits update Upcoming", observed: String(error.stack || error), classification: "ambiguous", log: "driver.log / failure.html" };
  if (browser) {
    fs.writeFileSync(path.join(run, "failure.html"), await browser.getPageSource().catch(() => "unavailable"));
    await browser.saveScreenshot(path.join(run, "failure.png")).catch(() => {});
  }
  process.exitCode = 1;
} finally {
  report.loadAfter = execFileSync("uptime", { encoding: "utf8" }).trim();
  fs.writeFileSync(path.join(run, "report.json"), JSON.stringify(report, null, 2)); console.log(JSON.stringify({ run, ...report }));
  await browser?.deleteSession().catch(() => {}); try { process.kill(-driver.pid, "SIGTERM"); } catch {} fs.closeSync(log);
}
