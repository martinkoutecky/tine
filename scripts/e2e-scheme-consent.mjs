// GH #503 / D-20261103-S503. Reuses the outside-file link journey's recording
// Linux opener to prove real WebKit -> device settings -> native OS handoff.
import { spawn, execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { setTimeout as poll } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { remote } from "webdriverio";
import { APP_ID } from "./lib/app-identity.mjs";
import { ensurePrivateSessionBus } from "./lib/e2e-session-bus.mjs";
import { accountHomeFixture, assertAppLease } from "./lib/e2e-account-home.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";

if (process.argv.includes("--help")) {
  console.log("Usage: xvfb-run -a node scripts/e2e-scheme-consent.mjs\nOptional: TINE_APP, TAURI_DRIVER, WEBKIT_DRIVER, E2E_DRIVER_PORT");
  process.exit(0);
}
ensurePrivateSessionBus();
assert.ok(process.env.DISPLAY, "Run under xvfb-run -a");
const app = path.resolve(process.env.TINE_APP || "target/debug/tine");
const artifacts = path.resolve("artifacts/og-scheme-consent-503");
fs.mkdirSync(artifacts, { recursive: true });
const fixture = fs.mkdtempSync(path.join(artifacts, "native-"));
const graph = path.join(fixture, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["bin", "home", "data", "config", "cache", "state"]) fs.mkdirSync(path.join(fixture, dir));
const pageFile = path.join(graph, "pages/App Links.md");
const bytes = "- [First](shortcuts://first/path)\n- [Second](Shortcuts://second/other)\n- [Other](obsidian://open)\n";
fs.writeFileSync(pageFile, bytes);
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{}\n");
const handoff = path.join(fixture, "handoff.jsonl");
fs.writeFileSync(path.join(fixture, "bin/xdg-open"), `#!/usr/bin/env node\nimport('node:fs').then(fs => fs.appendFileSync(${JSON.stringify(handoff)}, JSON.stringify(process.argv.slice(2))+String.fromCharCode(10)));\n`, { mode: 0o755 });
const settingsFile = path.join(fixture, "data", APP_ID, "tine-settings.json");
fs.mkdirSync(path.dirname(settingsFile), { recursive: true });
fs.writeFileSync(settingsFile, JSON.stringify({ last_graph_path: graph }));
const home = accountHomeFixture(fixture, fixture);
const env = { ...home.env, HOME: path.join(fixture, "home"), PATH: `${path.join(fixture, "bin")}:${process.env.PATH}`, TINE_GRAPH: graph,
  XDG_DATA_HOME: path.join(fixture, "data"), XDG_CONFIG_HOME: path.join(fixture, "config"), XDG_CACHE_HOME: path.join(fixture, "cache"), XDG_STATE_HOME: path.join(fixture, "state"),
  GDK_BACKEND: "x11", WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1" };
const port = Number(process.env.E2E_DRIVER_PORT || 4963);
const log = fs.openSync(path.join(fixture, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || path.join(process.env.CARGO_HOME, "bin/tauri-driver"),
  ["--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"],
  { env, stdio: ["ignore", log, log], detached: true });
const proof = { testedCommit: execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim(),
  binarySha256: createHash("sha256").update(fs.readFileSync(app)).digest("hex"), appPids: [], steps: [] };
let browser;
const calls = () => fs.existsSync(handoff) ? fs.readFileSync(handoff, "utf8").trim().split("\n").filter(Boolean).map(JSON.parse) : [];
const consent = (text) => { try { return JSON.parse(JSON.parse(text).always_open_link_schemes || "[]"); } catch { return []; } };
async function launch() {
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 0, connectionRetryTimeout: 60_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: app } } });
  await browser.setTimeout({ script: 15_000, pageLoad: 15_000, implicit: 0 });
  await browser.$('.app-container').waitForExist({ timeout: 30_000 });
  await openPageByName(browser, "App Links");
  await assertAppLease(home);
  const pids = home.sample().map(lease => lease.pid);
  assert.equal(pids.length, 1, "one fixture app owns the graph");
  proof.appPids.push(pids[0]);
}
async function clickLink(label) {
  const link = await browser.$(`a=${label}`);
  await link.waitForDisplayed({ timeout: 10_000 });
  await link.click();
  console.log(`Clicked ${label}`);
}
async function waitCalls(count) {
  await browser.waitUntil(() => calls().length === count, { timeout: 10_000, timeoutMsg: `expected ${count} OS handoffs` });
}
function screenshot(name) {
  // WebKitWebDriver's screenshot endpoint can hang with software compositing;
  // capture this journey's private Xvfb root instead. No pixel assertions.
  execFileSync("import", ["-display", env.DISPLAY, "-window", "root", path.join(fixture, name)], { env, timeout: 10_000 });
}
try {
  const deadline = Date.now() + 10_000;
  while (true) {
    try { if ((await fetch(`http://127.0.0.1:${port}/status`)).ok) break; } catch {}
    assert.ok(Date.now() < deadline, "driver readiness"); await poll(100);
  }
  await launch();
  console.log("Routed App Links");
  await clickLink("First");
  await browser.$('.link-scheme-prompt').waitForDisplayed({ timeout: 10_000 });
  assert.deepEqual(calls(), []);
  assert.ok((await browser.$('.link-scheme-prompt').getText()).includes("shortcuts://first/path"));
  assert.equal(await browser.$('.link-scheme-prompt input').isSelected(), false);
  screenshot("prompt.png");
  await browser.$('.link-scheme-prompt input').click();
  await browser.$('.link-scheme-prompt button[type="submit"]').click();
  await waitCalls(1);
  await waitForFileText(settingsFile, text => consent(text).includes("shortcuts"));
  proof.steps.push("Explicit checkbox consent opened first URL and persisted shortcuts in the device settings");
  await clickLink("Second"); await waitCalls(2);
  assert.equal(await browser.$('.link-scheme-prompt').isExisting(), false);
  await clickLink("Other"); await browser.$('.link-scheme-prompt').waitForDisplayed({ timeout: 10_000 });
  await browser.keys("Escape");
  await browser.$('.link-scheme-prompt').waitForExist({ reverse: true });
  assert.equal(calls().length, 2);
  proof.steps.push("Another host/path and case opened without prompting; another scheme still prompted and Escape cancelled");
  await browser.deleteSession(); browser = undefined;
  await launch();
  assert.notEqual(proof.appPids[0], proof.appPids[1], "restart uses a new app process");
  await clickLink("Second"); await waitCalls(3);
  assert.equal(await browser.$('.link-scheme-prompt').isExisting(), false);
  proof.steps.push("A new app process restored consent through tine-settings.json");
  await browser.$('button[title^="Settings"]').click();
  await browser.$('.settings-search-input').setValue("Remembered app links");
  await browser.$('.settings-search-result').click();
  const forget = await browser.$('button[aria-label="Forget shortcuts links"]');
  await forget.waitForDisplayed({ timeout: 10_000 });
  screenshot("settings.png");
  await forget.click();
  await waitForFileText(settingsFile, text => consent(text).length === 0);
  await browser.keys("Escape");
  await clickLink("First"); await browser.$('.link-scheme-prompt').waitForDisplayed({ timeout: 10_000 });
  assert.equal(calls().length, 3);
  proof.steps.push("Forgetting in Settings persisted removal and restored the prompt");
  assert.deepEqual(calls(), [["shortcuts://first/path"], ["Shortcuts://second/other"], ["Shortcuts://second/other"]]);
  assert.equal(fs.readFileSync(pageFile, "utf8"), bytes, "link consent does not change graph content");
  home.verify();
  fs.writeFileSync(path.join(fixture, "journey.json"), JSON.stringify({ ...proof, handoff: calls() }, null, 2));
  console.log(`PASS: remember, exact scheme, restart, Settings forget, and real native handoff. ${fixture}`);
} catch (error) {
  try { screenshot("failure.png"); } catch { /* fixture may already be closed */ }
  const visible = await browser?.execute(() => document.body.innerText).catch(String);
  fs.writeFileSync(path.join(fixture, "failure.json"), JSON.stringify({ ...proof, journey: "App link consent",
    expected: "Explicit consent persists across restart; Settings forget restores prompt", observed: String(error), visible,
    classification: "ambiguous until diagnosed" }, null, 2));
  throw error;
} finally {
  await browser?.deleteSession().catch(() => {});
  try { process.kill(-driver.pid, "SIGTERM"); } catch { driver.kill(); }
  home.close(); fs.closeSync(log);
}
