// The launch release of a decided superseding pair, in the shipped app (model
// v3 E1). A rename of Source to Receiver is interrupted before Receiver
// publishes; the relaunch recovers it, Receiver publishes its own text, Use
// Disk on Source decides the pair, and the process dies again before the
// decision retires. The store prepares that state in its own process
// (`launch_custody_journey_setup`, the same production IO the app's host runs)
// on this run's graph and in the draft namespace the app itself created; then
// the real app launches on it:
//   released: (a) Source keeps its text on disk and shows it, (b) Receiver
//     shows its own text, (c) the pair is released: no conflict surface, the
//     pair's drafts (its operation and decision, which keep Source's text) are
//     gone and nothing is drafted for Source;
//   held: Receiver is unreadable at launch (chmod 000), so the pair stays held:
//     its drafts stay, and Source shows its text, takes an edit and saves it.
// Env: TINE_APP (the app binary), TINE_CUSTODY_SETUP_ROOT (the checkout whose
// store prepares the state; default this one) and CARGO_TARGET_DIR for it.
import { spawn, spawnSync } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { startWebdriverApplication, stopWebdriverApplication, tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { BLOCK_EDITOR } from "./lib/e2e-block-editor.mjs";
import { APP_ID } from "./lib/app-identity.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";
import { installAccountHome, assertAppLease } from "./lib/e2e-account-home.mjs";

if (process.platform !== "linux") throw new Error("this journey makes a page unreadable with chmod and crashes with SIGKILL");
const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const SETUP_ROOT = process.env.TINE_CUSTODY_SETUP_ROOT || ROOT;
const SETUP_TEST = "page_host::production_tests::production_launch_custody_journey::launch_custody_journey_setup";
const TD = process.env.TAURI_DRIVER || "tauri-driver";
const DRIVER = Number(process.env.E2E_DRIVER_PORT || 4710);
const NATIVE = Number(process.env.E2E_NATIVE_PORT || 4711);
const TMP = fs.realpathSync(fs.mkdtempSync(path.join(process.env.E2E_TMP_DIR || os.tmpdir(), "tine-launch-custody-")));
const ARTIFACTS = process.env.E2E_ARTIFACT_DIR || path.join(TMP, "artifacts");
const SOURCE_TEXT = "- source text kept\n";
const RECEIVER_TEXT = "- receiver text\n";
const testedCommit = process.env.TINE_TESTED_COMMIT
  || `${spawnSync("git", ["rev-parse", "HEAD"], { cwd: ROOT, encoding: "utf8" }).stdout?.trim() || "unknown"} + working tree`;
fs.mkdirSync(ARTIFACTS, { recursive: true });
// The store's setup process runs without the account-home shim the app gets.
const setupEnv = { ...process.env };
const ownerFixture = installAccountHome(TMP, ARTIFACTS);
const shim = process.env.TINE_E2E_ACCOUNT_SHIM;
setupEnv.LD_PRELOAD = (setupEnv.LD_PRELOAD || "").split(":").filter((entry) => entry && entry !== shim).join(":");

const log = fs.openSync(path.join(ARTIFACTS, "tauri-driver.log"), "w");
const driverArgs = webdriverServerArgs(DRIVER, NATIVE, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver");
let run, target, driver, browser;

/** One scenario's disposable graph and app profile. */
function profile(name) {
  const dir = path.join(TMP, name);
  const graph = path.join(dir, "graph");
  for (const sub of ["pages", "journals", "logseq", "assets"]) fs.mkdirSync(path.join(graph, sub), { recursive: true });
  for (const sub of ["data", "config", "cache"]) fs.mkdirSync(path.join(dir, "xdg", sub), { recursive: true });
  fs.writeFileSync(path.join(graph, "logseq", "config.edn"), "{}\n");
  fs.writeFileSync(path.join(graph, "pages", "Source.md"), SOURCE_TEXT);
  const appData = path.join(dir, "xdg", "data", APP_ID);
  return {
    name, graph, appData, drafts: path.join(appData, "drafts-v2"),
    source: path.join(graph, "pages", "Source.md"), receiver: path.join(graph, "pages", "Receiver.md"),
    env: {
      ...process.env, TINE_GRAPH: graph,
      XDG_DATA_HOME: path.join(dir, "xdg", "data"), XDG_CONFIG_HOME: path.join(dir, "xdg", "config"),
      XDG_CACHE_HOME: path.join(dir, "xdg", "cache"),
      WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11",
    },
  };
}

/** Until the WebDriver server accepts connections on `port`. */
async function driverListening(port) {
  const deadline = Date.now() + 30_000;
  for (;;) {
    const open = await new Promise((resolve) => {
      const socket = net.connect(port, "127.0.0.1", () => { socket.end(); resolve(true); });
      socket.on("error", () => resolve(false));
    });
    if (open) return;
    if (Date.now() > deadline) throw new Error(`tauri-driver never listened on ${port}`);
    await sleep(100);
  }
}

async function launch(session) {
  target = await startWebdriverApplication(APP, run.env, NATIVE, session);
  driver = spawn(TD, driverArgs, { env: target.env, stdio: ["ignore", log, log], detached: true });
  await driverListening(DRIVER);
  browser = await remote({
    hostname: "127.0.0.1", port: DRIVER, path: "/", logLevel: "error", connectionRetryCount: 1, connectionRetryTimeout: 60_000,
    capabilities: tauriCapabilities(APP, session, process.platform, target.debuggerAddress),
  });
  await ensureMainWindow(browser);
  await browser.$(".page-title, .ls-block").waitForExist({ timeout: 20_000 });
  await assertAppLease(ownerFixture);
}

/** The app processes of this scenario (its graph in their environment). */
function appPids() {
  return fs.readdirSync("/proc").filter((pid) => /^\d+$/.test(pid)).filter((pid) => {
    try {
      return fs.readFileSync(`/proc/${pid}/environ`, "utf8").split("\0").includes(`TINE_GRAPH=${run.graph}`)
        && path.basename(fs.readlinkSync(`/proc/${pid}/exe`)) === path.basename(APP);
    } catch { return false; }
  }).map(Number);
}

/** A crash: SIGKILL the driver tree and every app process of this scenario. */
async function crash() {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  for (const pid of appPids()) { try { process.kill(pid, "SIGKILL"); } catch {} }
  stopWebdriverApplication(target);
  const deadline = Date.now() + 10_000;
  while (appPids().length) {
    if (Date.now() > deadline) throw new Error(`the app survived SIGKILL: ${appPids()}`);
    await sleep(100);
  }
  browser = undefined;
}

/** Every file under the scenario's draft store that holds `text`. */
function draftsHolding(text) {
  const found = [];
  const scan = (dir) => {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      const full = path.join(dir, entry.name);
      if (entry.isDirectory()) scan(full);
      else if (entry.isFile() && fs.readFileSync(full).includes(Buffer.from(text))) found.push(full);
    }
  };
  if (fs.existsSync(run.drafts)) scan(run.drafts);
  return found;
}

/** The draft namespace the app created for this graph, observed, not recomputed. */
async function draftNamespace() {
  const deadline = Date.now() + 20_000;
  for (;;) {
    const dirs = fs.existsSync(run.drafts)
      ? fs.readdirSync(run.drafts, { withFileTypes: true }).filter((entry) => entry.isDirectory()).map((entry) => entry.name) : [];
    if (dirs.length === 1) return dirs[0];
    if (dirs.length > 1) throw new Error(`more than one draft namespace: ${dirs}`);
    if (Date.now() > deadline) throw new Error("the app created no draft namespace for the graph");
    await sleep(100);
  }
}

/** The store, in its own process, leaves the decided pair (see the header). */
function prepareDecidedPair(id) {
  const result = spawnSync("cargo", ["test", "-q", "-p", "tine-store", "--lib", "--", "--ignored", "--exact", "--nocapture", SETUP_TEST], {
    cwd: SETUP_ROOT, encoding: "utf8", timeout: 1_800_000, maxBuffer: 64 << 20,
    env: { ...setupEnv, TINE_JOURNEY_GRAPH: run.graph, TINE_JOURNEY_APP_DATA: run.appData, TINE_JOURNEY_GRAPH_ID: id },
  });
  fs.writeFileSync(path.join(ARTIFACTS, `${run.name}-setup.log`), `${result.stdout}\n${result.stderr}`);
  if (result.status !== 0 || !result.stderr.includes("SETUP decided pair left")) {
    throw new Error(`the store setup failed (exit ${result.status}${result.error ? `, ${result.error}` : ""}); see ${run.name}-setup.log`);
  }
  if (fs.readFileSync(run.source, "utf8") !== SOURCE_TEXT) throw new Error("setup: Source lost its text");
  if (fs.readFileSync(run.receiver, "utf8") !== RECEIVER_TEXT) throw new Error("setup: Receiver did not publish its text");
  if (!draftsHolding(SOURCE_TEXT).length) throw new Error("setup: no draft keeps Source's text");
  // The pair's custody: every draft the setup left (its operation and decision).
  run.pairDrafts = draftsHolding("");
}

const pairDraftsLeft = () => run.pairDrafts.filter((file) => fs.existsSync(file));

const pageText = () => browser.execute(() => [...document.querySelectorAll(".page .ls-block")].map((node) => node.textContent ?? "").join("\n"));

async function showsText(name, fragment) {
  await openPageByName(browser, name);
  await browser.waitUntil(async () => (await pageText()).includes(fragment), {
    timeout: 15_000, interval: 200, timeoutMsg: `${name} does not show ${JSON.stringify(fragment)}`,
  });
}

const conflictSurfaces = () => browser.execute(() => document.querySelectorAll(".conflict-banner, .page-conflict").length);

/** Click the routed page's first block and put the caret at its end. */
async function editFirstBlock() {
  await browser.waitUntil(() => browser.execute((editor) => {
    const content = document.querySelector(".page .ls-block .block-content-wrapper");
    if (!(content instanceof HTMLElement)) return false;
    content.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0 }));
    content.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, cancelable: true, button: 0 }));
    content.click();
    return document.activeElement?.matches(editor) ?? false;
  }, BLOCK_EDITOR), { timeout: 10_000, interval: 300, timeoutMsg: "the first block's editor did not open" });
  await browser.keys(["End"]);
}

const results = { testedCommit, app: APP, setupRoot: SETUP_ROOT, scenarios: {} };

/** Each assertion is recorded on its own, so a build that fails one still reports the others. */
async function check(label, body) {
  const record = results.scenarios[run.name];
  try {
    await body();
    record.passed.push(label);
  } catch (error) {
    record.failed.push({ label, observed: String(error).split("\n").slice(0, 3).join(" | "),
      toasts: await browser?.execute(() => [...document.querySelectorAll(".toast .toast-msg")].map((node) => node.textContent ?? "")).catch(() => null),
      drafts: draftsHolding("").map((file) => path.relative(run.drafts, file)) });
    try { await browser?.saveScreenshot(path.join(ARTIFACTS, `${run.name}-${label.slice(0, 3).replace(/\W/g, "")}-failure.png`)); } catch {}
  }
}

async function scenario(name, judge) {
  run = profile(name);
  results.scenarios[name] = { passed: [], failed: [] };
  try {
    await launch(`${name}-first`);
    const id = await draftNamespace();
    await crash();
    prepareDecidedPair(id);
    await judge();
  } catch (error) {
    results.scenarios[name].failed.push({ label: "scenario", observed: String(error).split("\n").slice(0, 5).join(" | ") });
    try { await browser?.saveScreenshot(path.join(ARTIFACTS, `${name}-failure.png`)); } catch {}
  } finally {
    try { fs.chmodSync(run.receiver, 0o644); } catch {}
    if (target) await crash().catch(() => {});
    target = undefined;
  }
}

await scenario("released", async () => {
  await launch("released-judged");
  await check("(c) the pair's drafts are gone", async () => {
    await browser.waitUntil(async () => pairDraftsLeft().length === 0, {
      timeout: 20_000, interval: 200, timeoutMsg: `the pair's drafts remain: ${pairDraftsLeft()}`,
    });
    const source = draftsHolding("pages/Source.md");
    if (source.length) throw new Error(`Source is drafted: ${source}`);
  });
  await check("(a) Source keeps its text on disk and shows it", async () => {
    await showsText("Source", "source text kept");
    await waitForFileText(run.source, (text) => text === SOURCE_TEXT);
  });
  await check("(b) Receiver shows its own text", async () => {
    await showsText("Receiver", "receiver text");
    await waitForFileText(run.receiver, (text) => text === RECEIVER_TEXT);
    try { await browser.saveScreenshot(path.join(ARTIFACTS, "released-receiver.png")); } catch {}
  });
  await check("(c) no conflict surface", async () => {
    for (const name of ["Source", "Receiver"]) {
      await openPageByName(browser, name);
      const count = await conflictSurfaces();
      if (count) throw new Error(`${name} shows ${count} conflict surface(s)`);
    }
  });
  // Recorded, not asserted (develop does the same; a follow-up): launch
  // recovery re-drafts the operation's admitted Receiver text over Receiver's
  // newer published text.
  results.scenarios.released.observed = {
    draftsHoldingSourceText: draftsHolding(SOURCE_TEXT).map((file) => path.relative(run.drafts, file)),
    toasts: await browser.execute(() => [...document.querySelectorAll(".toast .toast-msg")].map((node) => node.textContent ?? "")),
  };
});

await scenario("held", async () => {
  fs.chmodSync(run.receiver, 0o000);
  await launch("held-judged");
  await check("(held) Source shows its text", () => showsText("Source", "source text kept"));
  await check("(held) Source stays editable and saves", async () => {
    await editFirstBlock();
    for (const key of " edited") await browser.keys([key]);
    await waitForFileText(run.source, (text) => text === "- source text kept edited\n");
    await browser.keys(["Escape"]);
  });
  // The save above came after the launch's judgment: the pair's drafts stay.
  await check("(held) the pair's drafts stay", async () => {
    const left = pairDraftsLeft();
    if (left.length !== run.pairDrafts.length) throw new Error(`only ${left.length} of ${run.pairDrafts.length} remain`);
  });
});

fs.writeFileSync(path.join(ARTIFACTS, "results.json"), JSON.stringify(results, null, 2) + "\n");
const failed = Object.entries(results.scenarios).flatMap(([name, { failed }]) => failed.map((f) => `${name}: ${f.label}: ${f.observed}`));
for (const [name, { passed }] of Object.entries(results.scenarios)) console.log(`${name}: passed ${passed.length} [${passed.join("; ")}]`);
if (failed.length) {
  console.error(`E2E FAIL (${testedCommit}):\n  ${failed.join("\n  ")}`);
  process.exitCode = 1;
} else console.log(`PASS (${testedCommit})`);
try { fs.closeSync(log); } catch {}
