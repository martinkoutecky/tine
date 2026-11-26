// tine#1041: a graph switch refused for a page the app does not show. Alpha's
// page "Repro 964" is deleted on disk while it has unsaved typing (a
// conflicted draft of a page with no file). While the window shows that page,
// a switch must be refused with a toast that names the page and carries no
// action, and the graph stays alpha. Then the app is killed. Relaunched on
// another page, the host recovers the draft and no window page names it: the
// switch to beta must go through, with the draft's text still in alpha's
// drafts and the page's file still absent. Reopened on alpha, the app reports
// the recovered edits and names the page.
import fs from "node:fs";
import os from "node:os";
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
// A short path: the app's owner socket lives under this run, and a socket path is capped near 108 bytes.
const artifacts = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(os.tmpdir(), "tine-e2e-gh1041"));
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
/** The draft namespaces (one directory per graph) that hold `text`. */
function draftNamespaces(text) {
  const drafts = path.join(data, "drafts-v2");
  if (!fs.existsSync(drafts)) return [];
  return fs.readdirSync(drafts, { withFileTypes: true }).filter(e => e.isDirectory()).map(e => e.name)
    .filter(name => fs.readdirSync(path.join(drafts, name), { withFileTypes: true }).filter(e => e.isFile())
      .some(file => fs.readFileSync(path.join(drafts, name, file.name)).includes(Buffer.from(text))));
}
const draftsHold = text => draftNamespaces(text).length > 0;
const graphName = () => browser.execute(() => document.querySelector(".graph-switch-name")?.textContent?.trim());
const errorToasts = () => browser.execute(() => [...document.querySelectorAll(".toast")].filter(t => t.querySelector(".toast-msg"))
  .map(t => ({ text: t.querySelector(".toast-msg").textContent, actions: [...t.querySelectorAll(".toast-action:not(.toast-copy)")].map(b => b.textContent) })));
async function switchToBeta() {
  // Escape leaves the editor; with no editor open it would start pane selection, which takes the keys.
  if (await browser.$(editorSelector()).isExisting()) await browser.keys(["Escape"]);
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
  const namespaces = draftNamespaces("base mine");
  assert(namespaces.length === 1, `one graph's drafts hold the text: ${JSON.stringify(namespaces)}`);
  passed.push(stage);

  stage = "shown in the window: the refused switch names the page";
  // Only a notice the switch adds counts (another notice may name the page too).
  let before = new Set((await errorToasts()).map(t => t.text));
  const refusal = async () => (await errorToasts()).find(t => !before.has(t.text) && t.text.includes(KEY));
  await switchToBeta();
  await browser.waitUntil(async () => !!(await refusal()), { timeout: 30_000, timeoutMsg: "the refused switch did not name the page" });
  const named = await refusal();
  assert(named.actions.length === 0, `the notice has no action: ${JSON.stringify(named.actions)}`);
  assert(await graphName() === "alpha", "the switch went through over a conflicted page the window shows");
  passed.push(stage);
  await kill();

  stage = "relaunched on another page: the switch goes through and the draft is kept";
  await launch();
  await openPageByName(browser, "Notes");
  // Right after a launch the page's recovery copy may not be written yet; a
  // switch in that moment is refused once, naming the page. Two repeats at most.
  let attempts = 0;
  for (; attempts < 3 && await graphName() !== "beta"; attempts++) {
    before = new Set((await errorToasts()).map(t => t.text));
    await switchToBeta();
    await browser.waitUntil(async () => await graphName() === "beta" || !!(await refusal()),
      { timeout: 30_000, timeoutMsg: "the switch neither went through nor was refused" });
  }
  assert(await graphName() === "beta", `the switch was refused ${attempts} times over a draft no window shows`);
  // The switch lands on beta's journal; its own Notes page shows beta's text.
  await openPageByName(browser, "Notes");
  await browser.waitUntil(() => browser.execute(() => document.body.textContent.includes("Beta notes")),
    { timeout: 30_000, timeoutMsg: "beta's page did not show" });
  assert(JSON.stringify(draftNamespaces("base mine")) === JSON.stringify(namespaces), "the draft left alpha's drafts");
  assert(!fs.existsSync(path.join(alpha, KEY)), "the switch wrote the page's file");
  assert(fs.readFileSync(path.join(beta, "pages/Notes.md"), "utf8") === "- Beta notes\n", "beta changed");
  passed.push(`${stage} (attempts: ${attempts})`);
  await kill();

  stage = "reopened on the first graph: the recovered edits are reported";
  await launch();
  await browser.waitUntil(async () => await graphName() === "alpha", { timeout: 30_000, timeoutMsg: "the relaunch did not open alpha" });
  await browser.waitUntil(async () => (await errorToasts()).some(t => t.text.includes("Recovered unsaved edits") && t.text.includes(KEY)),
    { timeout: 30_000, timeoutMsg: "the relaunch did not report the recovered page" });
  assert(JSON.stringify(draftNamespaces("base mine")) === JSON.stringify(namespaces), "the draft is gone after the return");
  assert(!fs.existsSync(path.join(alpha, KEY)), "the return wrote the page's file");
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
