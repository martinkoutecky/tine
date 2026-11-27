// tine#540: an original kept after a cancelled rename is listed in the
// recovery panel, and Copy original text puts its exact text on the clipboard.
// A rename of Draft to Final is cancelled after Draft's removal published
// (Final changed on disk, then Use Disk on Final): the store prepares that
// state in its own process (`kept_original_journey_setup`, the same
// production IO the app's host runs) on this run's graph and in the draft
// namespace the app itself created. Then the real app launches on it, and on
// a routed page the journey opens the recovery panel from the command
// palette, sees pages/Draft.md listed, presses Copy original text, and checks
// that the native clipboard holds Draft's original text byte for byte and
// that no graph file changed.
// Env: TINE_APP (the app binary, default target/release/tine),
// TINE_KEPT_SETUP_ROOT (the checkout whose store prepares the state; default
// this one) and CARGO_TARGET_DIR for it.
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import net from "node:net";
import { spawn, spawnSync, execFileSync } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { remote } from "webdriverio";
import { APP_ID } from "./lib/app-identity.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { installAccountHome, assertAppLease } from "./lib/e2e-account-home.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";

if (process.platform !== "linux") throw new Error("this journey crashes the app with SIGKILL and finds it through /proc");
const root = path.resolve(import.meta.dirname, "..");
const APP = process.env.TINE_APP || path.join(root, "target/release/tine");
const SETUP_ROOT = process.env.TINE_KEPT_SETUP_ROOT || root;
const SETUP_TEST = "page_host::production_tests::production_kept_original_journey::kept_original_journey_setup";
const COMMAND = "Review originals kept after a cancelled rename";
// LF line ends: what the native clipboard does to CRLF is not this journey's claim.
const ORIGINAL = "- the original text, kept ✓\n- second — with a [[Index]] link\n  - its child\n";
// A short path: the app's owner socket lives under this run, and a socket path is capped near 108 bytes.
const artifacts = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(os.tmpdir(), "tine-e2e-gh540"));
fs.mkdirSync(artifacts, { recursive: true });
const run = fs.mkdtempSync(path.join(artifacts, "native-"));
const graph = path.join(run, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{:tine/guide-announced? true}\n");
fs.writeFileSync(path.join(graph, "pages/Draft.md"), ORIGINAL);
fs.writeFileSync(path.join(graph, "pages/Index.md"), "- see [[Draft]]\n");
// The routed page: one the rename does not touch.
fs.writeFileSync(path.join(graph, "pages/Notes.md"), "- notes\n");
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(run, dir));
const appData = path.join(run, "data", APP_ID);
// The store's setup process runs without the account-home shim the app gets.
const setupEnv = { ...process.env };
const account = installAccountHome(run);
const shim = process.env.TINE_E2E_ACCOUNT_SHIM;
setupEnv.LD_PRELOAD = (setupEnv.LD_PRELOAD || "").split(":").filter(entry => entry && entry !== shim).join(":");
const env = { ...process.env, TINE_GRAPH: graph, TINE_GH540_RUN: run, XDG_DATA_HOME: path.join(run, "data"),
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
      return fs.readFileSync(`/proc/${pid}/environ`, "utf8").split("\0").includes(`TINE_GH540_RUN=${run}`)
        && path.basename(fs.readlinkSync(`/proc/${pid}/exe`)) === path.basename(APP);
    } catch { return false; }
  }).map(Number);
}
/** A forced quit of the driver tree and every app process of this run. */
async function kill() {
  try { await browser?.deleteSession(); } catch {}
  if (driver) { try { process.kill(-driver.pid, "SIGKILL"); } catch {} }
  for (const pid of appPids()) { try { process.kill(pid, "SIGKILL"); } catch {} }
  for (let n = 0; n < 100 && appPids().length; n++) await sleep(100);
  assert(appPids().length === 0, "the app must exit");
  browser = undefined; driver = undefined;
}
async function launch(session) {
  const port = await unusedPort();
  let nativePort = await unusedPort();
  while (nativePort === port) nativePort = await unusedPort();
  driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", webdriverServerArgs(port, nativePort, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"),
    { env, stdio: ["ignore", log, log], detached: true });
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 5,
    connectionRetryTimeout: 30_000, capabilities: tauriCapabilities(APP, session) });
  await ensureMainWindow(browser);
  await browser.$(".graph-switch-btn").waitForExist({ timeout: 30_000 });
  await assertAppLease(account);
}
/** The draft namespace the app created for this graph, observed, not recomputed. */
async function draftNamespace() {
  const drafts = path.join(appData, "drafts-v2");
  for (let n = 0; n < 200; n++) {
    const dirs = fs.existsSync(drafts) ? fs.readdirSync(drafts, { withFileTypes: true }).filter(e => e.isDirectory()).map(e => e.name) : [];
    if (dirs.length === 1) return dirs[0];
    assert(dirs.length < 2, `more than one draft namespace: ${dirs}`);
    await sleep(100);
  }
  throw new Error("the app created no draft namespace for the graph");
}
/** Every file under `dir`, with its bytes. */
function tree(dir, out = new Map()) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) tree(full, out);
    else if (entry.isFile()) out.set(path.relative(graph, full), fs.readFileSync(full).toString("base64"));
  }
  return out;
}
const changed = (before, after) => [...new Set([...before.keys(), ...after.keys()])].filter(file => before.get(file) !== after.get(file));
async function nativeClipboard(command, text) {
  const result = await browser.executeAsync((cmd, value, done) => {
    globalThis.__TAURI_INTERNALS__.invoke(`plugin:clipboard-manager|${cmd}`, value === null ? {} : { text: value })
      .then(out => done({ ok: true, out }), error => done({ ok: false, error: String(error) }));
  }, command, text ?? null);
  if (!result.ok) throw new Error(`native clipboard ${command} failed: ${result.error}`);
  return result.out;
}
async function runCommand(label) {
  await browser.keys(["Control", "Shift", "p"]);
  const input = await browser.$(".switcher-input"); await input.waitForDisplayed({ timeout: 10_000 });
  await input.setValue(label);
  const rowIndex = () => browser.execute(l => [...document.querySelectorAll(".switcher-row")]
    .findIndex(r => (r.querySelector(".switcher-name")?.textContent ?? r.textContent).trim() === l), label);
  await browser.waitUntil(async () => await rowIndex() >= 0, { timeout: 10_000, timeoutMsg: `the palette offers no ${JSON.stringify(label)}` });
  await (await browser.$$(".switcher-row"))[await rowIndex()].click();
  await browser.waitUntil(() => browser.execute(() => !document.querySelector(".switcher-input")), { timeout: 10_000, timeoutMsg: "palette stayed open" });
}
const keptRows = () => browser.execute(() => [...document.querySelectorAll(".unsaved-recovery-panel .recovery-kept-row")]
  .map(row => ({ text: row.textContent ?? "", buttons: [...row.querySelectorAll("button")].map(b => b.textContent?.trim()) })));

try {
  stage = "a rename of Draft to Final, cancelled after Draft's removal published";
  await launch("gh540-first");
  const id = await draftNamespace();
  await kill();
  const setup = spawnSync("cargo", ["test", "-q", "-p", "tine-store", "--lib", "--", "--ignored", "--exact", "--nocapture", SETUP_TEST], {
    cwd: SETUP_ROOT, encoding: "utf8", timeout: 1_800_000, maxBuffer: 64 << 20,
    env: { ...setupEnv, TINE_JOURNEY_GRAPH: graph, TINE_JOURNEY_APP_DATA: appData, TINE_JOURNEY_GRAPH_ID: id },
  });
  fs.writeFileSync(path.join(run, "setup.log"), `${setup.stdout}\n${setup.stderr}`);
  assert(setup.status === 0 && setup.stderr.includes("SETUP kept original left"),
    `the store setup failed (exit ${setup.status}${setup.error ? `, ${setup.error}` : ""}); see ${run}/setup.log`);
  assert(!fs.existsSync(path.join(graph, "pages/Draft.md")), "setup: Draft's file is still on disk");
  assert(fs.readFileSync(path.join(graph, "pages/Final.md"), "utf8") === "- other\n", "setup: Final does not hold the disk text");
  const before = tree(graph);
  passed.push(stage);

  stage = "relaunched on the routed Notes page, the palette opens the recovery panel";
  await launch("gh540-kept");
  // The graph is drawn (the journal's blocks) before the switcher is asked for a page.
  await browser.$(".ls-block").waitForExist({ timeout: 30_000, timeoutMsg: "the graph never drew a block" });
  await openPageByName(browser, "Notes");
  await runCommand(COMMAND);
  await browser.$(".unsaved-recovery-panel").waitForExist({ timeout: 10_000, timeoutMsg: "the recovery panel did not open" });
  passed.push(stage);

  stage = "the panel lists the kept original with Copy original text";
  await browser.waitUntil(async () => (await keptRows()).some(row => row.text.includes("pages/Draft.md")),
    { timeout: 15_000, timeoutMsg: "the panel does not list pages/Draft.md" });
  const rows = await keptRows();
  assert(rows.length === 1, `one kept original is listed: ${JSON.stringify(rows)}`);
  assert(JSON.stringify(rows[0].buttons) === JSON.stringify(["Copy original text"]), `the row offers Copy original text: ${JSON.stringify(rows[0])}`);
  passed.push(stage);

  stage = "Copy original text puts Draft's original text on the clipboard, byte for byte";
  await nativeClipboard("write_text", "gh540 sentinel");
  await browser.execute(() => [...document.querySelectorAll(".unsaved-recovery-panel .recovery-kept-row button")]
    .find(b => b.textContent?.trim() === "Copy original text")?.click());
  let clipboard = null;
  await browser.waitUntil(async () => (clipboard = await nativeClipboard("read_text")) !== "gh540 sentinel",
    { timeout: 10_000, interval: 50, timeoutMsg: "Copy original text never reached the native clipboard" });
  assert(Buffer.from(clipboard ?? "", "utf8").equals(Buffer.from(ORIGINAL, "utf8")),
    `the clipboard holds ${JSON.stringify(clipboard)}, not the original ${JSON.stringify(ORIGINAL)}`);
  await browser.waitUntil(() => browser.execute(() => document.querySelector(".unsaved-recovery-panel .recovery-kept-row")?.closest("section")?.querySelector("[role=status]")?.textContent?.startsWith("Copied.")),
    { timeout: 5_000, timeoutMsg: "the panel did not say Copied." });
  passed.push(stage);

  stage = "no graph file changed, and the original stays kept, not restored";
  // The rename's link rewrite in the routed page stands.
  await waitForFileText(path.join(graph, "pages/Index.md"), text => text.includes("[[Final]]"));
  const diff = changed(before, tree(graph));
  assert(diff.length === 0, `graph files changed: ${JSON.stringify(diff)}`);
  assert((await keptRows()).some(row => row.text.includes("pages/Draft.md")), "the row left the panel after Copy");
  passed.push(stage);

  fs.writeFileSync(path.join(run, "result.json"), JSON.stringify({ outcome: "pass", testedCommit: commit, workingDiff: true, app: APP, passed }, null, 2));
  console.log(`PASS: ${passed.join("; ")}. ${run}`);
} catch (error) {
  try { await browser?.saveScreenshot(path.join(run, "failure.png")); } catch {}
  try { fs.writeFileSync(path.join(run, "failure-dom.html"), await browser.execute(() => document.documentElement.outerHTML)); } catch {}
  fs.writeFileSync(path.join(run, "result.json"), JSON.stringify({ outcome: "fail", testedCommit: commit, workingDiff: true, app: APP, journey: stage,
    expected: stage, observed: String(error), classification: "ambiguous", passed }, null, 2));
  console.error(`FAIL ${stage}: ${error}`);
  process.exitCode = 1;
} finally {
  await kill();
  fs.closeSync(log);
  console.log(`Artifacts: ${run}`);
}
