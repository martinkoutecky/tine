// GH #809: real Linux UI-thread hang, background flush, SIGKILL, report after
// relaunch. Requires a `cargo test -p tine --lib --features custom-protocol
// --no-run` executable in TINE_FLIGHT_TEST_BINARY. The hook is cfg(test) only.
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { execFileSync, spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { remote } from "webdriverio";
import { accountHomeFixture, assertAppLease } from "./lib/e2e-account-home.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";

assert.equal(process.platform, "linux");
const binary = path.resolve(process.env.TINE_FLIGHT_TEST_BINARY || "");
assert.ok(process.env.TINE_FLIGHT_TEST_BINARY && fs.existsSync(binary), "Set TINE_FLIGHT_TEST_BINARY");
const out = path.resolve(process.env.E2E_ARTIFACT_DIR || "target/artifacts/fix-beta-809-stages/hang");
const run = path.join(out, `fixture-${process.pid}`);
const graph = path.join(run, "synthetic-graph");
for (const dir of ["home", "data", "config", "cache", "state", "graph"]) fs.mkdirSync(path.join(run, dir), { recursive: true });
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
fs.writeFileSync(path.join(graph, "pages/Flight probe.md"), "- synthetic window flight probe\n");
const wrapper = path.join(run, "test-app");
const shellQuote = value => `'${value.replaceAll("'", "'\\''")}'`;
fs.writeFileSync(wrapper, `#!/bin/sh\nexec ${shellQuote(binary)} --exact workspace_windows::tests::flight_hang_app --ignored --nocapture --test-threads=1\n`, { mode: 0o755 });
const fixture = accountHomeFixture(run, out);
const env = { ...fixture.env, HOME: path.join(run, "home"), XDG_DATA_HOME: path.join(run, "data"),
  XDG_CONFIG_HOME: path.join(run, "config"), XDG_CACHE_HOME: path.join(run, "cache"), XDG_STATE_HOME: path.join(run, "state"),
  TINE_GRAPH: graph, TINE_TEST_FLIGHT_APP: "1", TINE_TEST_BLOCK_WORKSPACE_OPEN: "1",
  GDK_BACKEND: "x11", WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1" };
const result = { testedCommit: execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim(),
  journey: "UI thread blocked during popup builder; SIGKILL after background flush; relaunch diagnostics",
  expected: "Previous unclean run retains builder entry and native main-thread stalls", status: "running", steps: [],
  logs: [path.join(out, "hang-driver.log"), path.join(out, "relaunch-driver.log")] };
function failureLogExcerpt() {
  for (const file of [...result.logs].reverse()) {
    try { return fs.readFileSync(file, "utf8").slice(-2000); } catch {}
  }
  return "No driver log was created";
}
const write = () => fs.writeFileSync(path.join(out, "result.json"), JSON.stringify(result, null, 2) + "\n");
const step = (name, facts = {}) => { result.steps.push({ name, ...facts }); write(); console.log(name); };
let browser, driver;
const logs = [];
const port = Number(process.env.E2E_DRIVER_PORT || 4682);
const nativePort = port + 1;
const wm = spawn("openbox", ["--sm-disable"], { env, stdio: "ignore" });
async function startSession(name) {
  const log = fs.openSync(path.join(out, `${name}-driver.log`), "w");
  logs.push(log);
  driver = spawn(process.env.TAURI_DRIVER || "tauri-driver",
    webdriverServerArgs(port, nativePort, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"),
    { env, detached: true, stdio: ["ignore", log, log] });
  await sleep(2000);
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error",
    connectionRetryCount: 0, connectionRetryTimeout: 15_000, capabilities: tauriCapabilities(wrapper) });
  await ensureMainWindow(browser);
  await browser.$(".ls-block").waitForExist({ timeout: 30_000 });
  await assertAppLease(fixture);
}
async function diagnostics() {
  const report = await browser.executeAsync(done => {
    window.__TAURI_INTERNALS__.invoke("diagnostic_report", { buildCommit: "", buildTime: "", renderer: null })
      .then(value => done(value.text), error => done(JSON.stringify({ error: String(error) })));
  });
  return JSON.parse(report);
}
function stopSession() {
  if (driver?.pid) { try { process.kill(-driver.pid, "SIGKILL"); } catch {} }
  driver = undefined;
  browser = undefined;
}
const historyFile = path.join(run, "data/page.tine.TineBeta/diagnostics/history.jsonl");
const watchdog = setTimeout(() => {
  result.status = "failed"; result.classification = "ambiguous"; result.observed = "Hang journey exceeded 150 s";
  result.logExcerpt = failureLogExcerpt();
  write(); stopSession(); wm.kill(); fixture.close(); process.exit(1);
}, 150_000);
try {
  await startSession("hang");
  await sleep(6500); // Includes an actual idle watchdog probe.
  assert.ok(!(await diagnostics()).sessions.current.some(event => event.event === "runtime.main_thread_stall"));
  step("idle-watchdog-quiet");
  await browser.keys(["Control", "k"]);
  await browser.$(".switcher-input").setValue("Flight probe");
  await browser.waitUntil(() => browser.execute(() => {
    const row = [...document.querySelectorAll(".switcher-row:not(.block-result)")]
      .find(el => el.querySelector(".switcher-name")?.textContent?.trim() === "Flight probe");
    if (!row) return false;
    row.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, button: 2, clientX: 400, clientY: 250 }));
    return true;
  }), { timeout: 15_000 });
  const openedAt = Date.now();
  // Do not depend on any WebDriver response once the UI thread is blocked.
  const click = browser.execute(() => {
    const item = [...document.querySelectorAll(".ctx-page-item")].find(el => el.textContent?.trim() === "Open in new window");
    if (!item) throw new Error("New window item absent");
    item.click();
  }).catch(error => ({ error: String(error) }));
  let retained;
  while (Date.now() - openedAt < 70_000) {
    try { retained = fs.readFileSync(historyFile, "utf8").trim().split("\n").map(line => JSON.parse(line)); } catch {}
    if (retained?.some(e => e.stage === "native.builder-entry")
      && retained.some(e => e.event === "runtime.main_thread_stall" && e.thresholdMs === 30000)) break;
    await sleep(1000);
  }
  assert.ok(retained?.some(e => e.stage === "native.builder-entry"), "Background flush did not retain builder entry");
  assert.ok(retained.some(e => e.event === "runtime.main_thread_stall" && e.thresholdMs === 30000), "No persisted 30 s native stall");
  assert.ok(!retained.some(e => e.stage === "native.build-returned"));
  fs.writeFileSync(path.join(out, "frozen-history.json"), JSON.stringify(retained, null, 2));
  const owners = fixture.sample();
  assert.equal(owners.length, 1);
  process.kill(owners[0].pid, "SIGKILL");
  step("killed-after-background-flush", { frozenForMs: Date.now() - openedAt, killedPid: owners[0].pid });
  stopSession();
  await click;
  delete env.TINE_TEST_BLOCK_WORKSPACE_OPEN;
  await sleep(1000);
  await startSession("relaunch");
  const report = await diagnostics();
  fs.writeFileSync(path.join(out, "relaunch-diagnostics.json"), JSON.stringify(report, null, 2));
  assert.equal(report.runtime.previousExitUnclean, true);
  const previous = report.sessions.previous;
  const entry = previous.find(e => e.stage === "native.builder-entry");
  assert.ok(entry);
  for (const stage of ["native.prepare-received", "native.prepared", "native.request-received", "native.builder-entry"]) {
    assert.ok(previous.some(e => e.stage === stage && e.sequence === entry.sequence), `Missing previous ${stage}`);
  }
  for (const thresholdMs of [2000, 10000, 30000]) {
    assert.ok(previous.some(e => e.event === "runtime.main_thread_stall" && e.thresholdMs === thresholdMs && !e.recovered));
  }
  assert.ok(!previous.some(e => e.stage === "native.build-returned"));
  step("relaunch-retained-stages-and-native-stalls", { sequence: entry.sequence });
  fixture.verify(); result.status = "passed"; write();
} catch (error) {
  result.status = "failed"; result.classification = "ambiguous"; result.observed = String(error);
  result.logExcerpt = failureLogExcerpt(); write();
  process.exitCode = 1; console.error(error);
} finally {
  if (browser) await Promise.race([browser.deleteSession().catch(() => {}), sleep(1000)]);
  stopSession(); wm.kill(); fixture.close(); clearTimeout(watchdog);
  for (const log of logs) fs.closeSync(log);
}
