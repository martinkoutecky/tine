// tine#1041: a graph switch refused for a page the app does not show. Alpha's
// page "Repro 964" is deleted on disk while it has unsaved typing (a
// conflicted draft of a page with no file), and the app is killed. Relaunched,
// the host recovers the draft and owes it on every switch barrier, while no
// window page names it. The switch must be refused with a toast that names the
// page and carries no action, and the graph stays alpha.
import fs from "node:fs";
import path from "node:path";
import net from "node:net";
import { spawn, execFileSync } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { remote } from "webdriverio";
import { APP_ID } from "./lib/app-identity.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { editorSelector } from "./lib/e2e-block-editor.mjs";
import { installAccountHome, assertAppLease } from "./lib/e2e-account-home.mjs";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";

const root = path.resolve(import.meta.dirname, "..");
const artifacts = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(root, "artifacts/gh1041-switch-draft"));
fs.mkdirSync(artifacts, { recursive: true });
const run = fs.mkdtempSync(path.join(artifacts, "native-"));
const alpha = path.join(run, "alpha"), beta = path.join(run, "beta");
const KEY = "pages/Repro 964.md";
for (const graph of [alpha, beta]) {
  for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
  fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{:tine/guide-announced? true}\n");
  fs.writeFileSync(path.join(graph, "pages/Notes.md"), `- ${graph === alpha ? "Alpha" : "Beta"} notes\n`);
}
fs.writeFileSync(path.join(alpha, KEY), "- base\n");
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(run, dir));
const data = path.join(run, "data", APP_ID);
fs.mkdirSync(data, { recursive: true });
fs.writeFileSync(path.join(data, "tine-settings.json"), JSON.stringify({
  known_graphs: [{ name: "alpha", path: alpha }, { name: "beta", path: beta }], last_graph_path: alpha,
}));
// Drafts need the app's owner lease: resolve the account home into this run.
const account = installAccountHome(run);
// Pin this run's app across the kill and relaunch.
const app = path.join(run, "tine");
fs.copyFileSync(process.env.TINE_APP || path.join(root, "target/debug/tine"), app);
fs.chmodSync(app, 0o700);
const env = { ...process.env, TINE_GRAPH: alpha, TINE_GH1041_RUN: run, XDG_DATA_HOME: path.join(run, "data"),
  XDG_CONFIG_HOME: path.join(run, "config"), XDG_CACHE_HOME: path.join(run, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const commit = execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim();
const log = fs.openSync(path.join(run, "driver.log"), "w");
const assert = (condition, message) => { if (!condition) throw new Error(message); };
const unusedPort = () => new Promise((resolve, reject) => {
  const server = net.createServer(); server.on("error", reject);
  server.listen(0, "127.0.0.1", () => { const { port } = server.address(); server.close(error => error ? reject(error) : resolve(port)); });
});
let driver, browser, stage = "launch";
const passed = [];

function appPids() {
  return fs.readdirSync("/proc").filter(pid => /^\d+$/.test(pid)).filter(pid => {
    try {
      return fs.readFileSync(`/proc/${pid}/environ`, "utf8").split("\0").includes(`TINE_GH1041_RUN=${run}`)
        && path.basename(fs.readlinkSync(`/proc/${pid}/exe`)) === path.basename(app);
    } catch { return false; }
  }).map(Number);
}
/** A forced quit: nothing is flushed, the durable draft is all that is left. */
async function kill() {
  try { await browser?.deleteSession(); } catch {}
  if (driver) { try { process.kill(-driver.pid, "SIGKILL"); } catch {} }
  for (const pid of appPids()) { try { process.kill(pid, "SIGKILL"); } catch {} }
  for (let n = 0; n < 100 && appPids().length; n++) await sleep(100);
  assert(appPids().length === 0, "the old app must exit before relaunch");
  browser = undefined; driver = undefined;
}
async function launch() {
  const port = await unusedPort();
  let nativePort = await unusedPort();
  while (nativePort === port) nativePort = await unusedPort();
  driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", webdriverServerArgs(port, nativePort, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"),
    { env, stdio: ["ignore", log, log], detached: true });
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 5,
    connectionRetryTimeout: 30_000, capabilities: tauriCapabilities(app, "gh1041") });
  await ensureMainWindow(browser);
  await browser.$(".graph-switch-btn").waitForExist({ timeout: 30_000 });
}
function draftsHold(text) {
  const drafts = path.join(data, "drafts-v2");
  if (!fs.existsSync(drafts)) return false;
  for (const dir of fs.readdirSync(drafts, { withFileTypes: true }).filter(e => e.isDirectory())) {
    for (const file of fs.readdirSync(path.join(drafts, dir.name), { withFileTypes: true }).filter(e => e.isFile())) {
      if (fs.readFileSync(path.join(drafts, dir.name, file.name)).includes(Buffer.from(text))) return true;
    }
  }
  return false;
}
const graphName = () => browser.execute(() => document.querySelector(".graph-switch-name")?.textContent?.trim());
const errorToasts = () => browser.execute(() => [...document.querySelectorAll(".toast")].filter(t => t.querySelector(".toast-msg"))
  .map(t => ({ text: t.querySelector(".toast-msg").textContent, actions: [...t.querySelectorAll(".toast-action:not(.toast-copy)")].map(b => b.textContent) })));
async function switchToBeta() {
  await browser.keys(["Escape"]);
  await browser.keys("g");
  await browser.keys(["Shift", "g"]);
  await browser.$(".all-graphs").waitForExist({ timeout: 10_000 });
  await browser.waitUntil(() => browser.execute(() => document.activeElement?.hasAttribute("data-graph-open")), { timeout: 5000 });
  const index = await browser.execute(target => [...document.querySelectorAll("[data-graph-open]")].findIndex(b => b.title === target), beta);
  assert(index >= 0, "beta must be in the graph registry");
  await browser.keys(["Home"]);
  for (let i = 0; i < index; i++) await browser.keys(["ArrowDown"]);
  await browser.keys(["Enter"]);
}

try {
  stage = "a conflicted draft of a page deleted on disk, left by a forced quit";
  await launch();
  await assertAppLease(account);
  await openPageByName(browser, "Repro 964");
  await browser.$(".block-content").click();
  await browser.$(editorSelector()).waitForExist({ timeout: 5000 });
  await browser.keys(["End"]);
  fs.unlinkSync(path.join(alpha, KEY));
  await browser.keys(" mine");
  await browser.waitUntil(() => draftsHold("base mine"), { timeout: 30_000, timeoutMsg: "the typing never reached a durable draft" });
  await browser.$(".conflict-banner").waitForExist({ timeout: 30_000, timeoutMsg: "deleting the page under typing did not conflict" });
  assert(!fs.existsSync(path.join(alpha, KEY)), "the page's file came back before the choice");
  passed.push(stage);
  await kill();

  stage = "relaunched on another page: the refused switch names the page";
  await launch();
  await openPageByName(browser, "Notes");
  await switchToBeta();
  await browser.waitUntil(async () => (await errorToasts()).some(t => t.text.includes(KEY)),
    { timeout: 30_000, timeoutMsg: "the refused switch did not name the page" });
  const named = (await errorToasts()).find(t => t.text.includes(KEY));
  assert(named.actions.length === 0, `the notice has no action: ${JSON.stringify(named.actions)}`);
  assert(await graphName() === "alpha", "the switch went through over an unresolved draft");
  assert(fs.readFileSync(path.join(beta, "pages/Notes.md"), "utf8") === "- Beta notes\n", "beta changed");
  passed.push(stage);

  fs.writeFileSync(path.join(run, "result.json"), JSON.stringify({ outcome: "pass", testedCommit: commit, workingDiff: true, passed }, null, 2));
  console.log(`PASS: ${passed.join("; ")}. ${run}`);
} catch (error) {
  try { await browser?.saveScreenshot(path.join(run, "failure.png")); } catch {}
  try { fs.writeFileSync(path.join(run, "failure-dom.html"), await browser.execute(() => document.documentElement.outerHTML)); } catch {}
  fs.writeFileSync(path.join(run, "result.json"), JSON.stringify({ outcome: "fail", testedCommit: commit, workingDiff: true, journey: stage,
    expected: stage, observed: String(error), classification: "ambiguous", passed }, null, 2));
  console.error(`FAIL ${stage}: ${error}`);
  process.exitCode = 1;
} finally {
  await kill();
  fs.closeSync(log);
  console.log(`Artifacts: ${run}`);
}
