// Native semantic journey for GH #240 selection-owned heading and drag. It runs
// on Linux (wry + tauri-driver) and on Windows (WebView2 + msedgedriver attach
// mode); see scripts/e2e-capabilities.mjs for why Windows must attach rather
// than let EdgeDriver launch the app.
// It intentionally observes content, active block selection, root order, and
// durable reload only; coordinate values come from live rows and are not an
// oracle. Run with the staged binary explicitly, for example:
// TINE_APP=/path/to/tine TINE_CANDIDATE_COMMIT=$(git rev-parse HEAD) node scripts/e2e-selection-actions.mjs

import { spawn, spawnSync } from "node:child_process";
import { remote, Key } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  freeLoopbackPort,
  startWebdriverApplication,
  stopWebdriverApplication,
  tauriCapabilities,
  webdriverServerArgs,
} from "./e2e-capabilities.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";
import { ensureDisplay, stopDisplay } from "./lib/e2e-display.mjs";
import { openPageByLink } from "./lib/e2e-navigation.mjs";

const CANDIDATE_COMMIT = process.env.TINE_CANDIDATE_COMMIT;
if (!CANDIDATE_COMMIT || !/^[0-9a-f]{40}$/.test(CANDIDATE_COMMIT)) {
  throw new Error("GH #240 native E2E requires the exact 40-character artifact commit through TINE_CANDIDATE_COMMIT.");
}
const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP;
if (!APP) throw new Error("GH #240 native E2E requires the exact artifact through TINE_APP; no worktree-binary fallback is allowed.");
if (!fs.existsSync(APP)) throw new Error(`TINE_APP does not exist: ${APP}`);

await ensureDisplay();

const RUN_ID = `${process.pid}-${Date.now()}`;
// Native Linux test runners can advertise a read-only Node tmpdir. `/tmp` is
// the controlled writable parent there; every child remains uniquely
// GH240-named. Windows has no `/tmp`, so it uses the real temp directory.
const TMP = path.join(
  process.platform === "win32" ? os.tmpdir() : "/tmp",
  `tine-gh240-selection-action-ownership-e2e-${RUN_ID}`,
);
const GRAPH = path.join(TMP, "graph");
const XDG = path.join(TMP, "xdg");
const ARTIFACTS = path.join(TMP, "artifacts");
const PAGE = "GH240 Selection Ownership";
const PAGE_FILE = path.join(GRAPH, "pages", `${PAGE}.md`);
const today = new Date();
const JOURNAL_STEM = `${today.getFullYear()}_${String(today.getMonth() + 1).padStart(2, "0")}_${String(today.getDate()).padStart(2, "0")}`;
const JOURNAL_FILE = path.join(GRAPH, "journals", `${JOURNAL_STEM}.md`);
const EXPECTED_ORDER = ["A", "D", "E", "B", "C"];
const DRIVER = await freeLoopbackPort();
const NATIVE = await freeLoopbackPort(new Set([DRIVER]));

// This is a fresh, named routed page: the only roots are the semantic labels
// used by the journey, so root order can be checked without layout assumptions.
fs.rmSync(TMP, { recursive: true, force: true });
for (const directory of ["pages", "journals", "logseq", "assets"]) {
  fs.mkdirSync(path.join(GRAPH, directory), { recursive: true });
}
for (const directory of ["data", "config", "cache"]) {
  fs.mkdirSync(path.join(XDG, directory), { recursive: true });
}
fs.mkdirSync(ARTIFACTS, { recursive: true });
fs.writeFileSync(path.join(GRAPH, "logseq", "config.edn"), "{}\n");
fs.writeFileSync(PAGE_FILE, "- A\n- B\n- C\n- D\n- E\n");
// Filename-discovered pages are not necessarily in the initial sidebar. Seed
// the app's normal starting journal with a visible page reference, then use
// that rendered reference for each route into this named-page journey.
fs.writeFileSync(JOURNAL_FILE, `- Open [[${PAGE}]]\n`);

const env = {
  ...process.env,
  TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: path.join(XDG, "data"),
  XDG_CONFIG_HOME: path.join(XDG, "config"),
  XDG_CACHE_HOME: path.join(XDG, "cache"),
  // Windows reads app state from APPDATA/LOCALAPPDATA, not XDG_*. Without these
  // the journey would run against the runner's own profile instead of its
  // private GH240 state, and the relaunch step would prove nothing.
  APPDATA: path.join(TMP, "appdata"),
  LOCALAPPDATA: path.join(TMP, "localappdata"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1",
  WEBKIT_DISABLE_COMPOSITING_MODE: "1",
  LIBGL_ALWAYS_SOFTWARE: "1",
  GDK_BACKEND: "x11",
};

if (process.platform === "win32" && process.env.CI === "true") {
  spawnSync("taskkill", ["/IM", path.basename(APP), "/T", "/F"], { stdio: "ignore" });
}

const driverLogPath = path.join(ARTIFACTS, "tauri-driver.log");
const driverLog = fs.openSync(driverLogPath, "w");
const DRIVER_BIN = process.env.TAURI_DRIVER || "tauri-driver";
const DRIVER_ARGS = webdriverServerArgs(
  DRIVER,
  NATIVE,
  process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver",
);

// On Linux the wry driver launches the app per WebDriver session, so a session
// IS the app process. On Windows the app must be started first with a fixed
// remote-debugging port and attached to, because EdgeDriver's launch-mode
// DevToolsActivePort handshake is unreliable on hosted runners (see
// scripts/e2e-capabilities.mjs). That also means `deleteSession` alone does not
// restart the app there, so the relaunch step drives the tree explicitly.
let webviewTarget = await startWebdriverApplication(APP, env, NATIVE, "initial");
let driver = spawn(DRIVER_BIN, DRIVER_ARGS, {
  env: webviewTarget.env,
  stdio: ["ignore", driverLog, driverLog],
  detached: process.platform !== "win32",
});

function killDriverTree() {
  try {
    if (process.platform === "win32") driver.kill("SIGKILL");
    else process.kill(-driver.pid, "SIGKILL");
  } catch {}
  stopWebdriverApplication(webviewTarget);
}

async function startDriverTree(session) {
  webviewTarget = await startWebdriverApplication(APP, env, NATIVE, session);
  driver = spawn(DRIVER_BIN, DRIVER_ARGS, {
    env: webviewTarget.env,
    stdio: ["ignore", driverLog, driverLog],
    detached: process.platform !== "win32",
  });
  await sleep(2500);
}

let browser;
let step = "start driver";
let expected = "a fresh named page with five roots A-E";
let failure = false;

function phase(nextStep, nextExpected) {
  step = nextStep;
  expected = nextExpected;
}

function assert(condition, message, observed) {
  if (!condition) throw new Error(`${message}: ${JSON.stringify(observed)}`);
}

async function rootSnapshot() {
  return browser.execute(() => [...document.querySelectorAll(".page-blocks > .ls-block")]
    .filter((root) => !root.hasAttribute("data-page-preamble"))
    .map((root) => {
      const main = root.querySelector(":scope > .block-main");
      const content = root.querySelector(":scope > .block-main > .block-content-wrapper > .block-content");
      return {
        id: root.getAttribute("data-block-id"),
        label: content?.textContent?.trim() ?? "",
        selected: main?.classList.contains("selected") ?? false,
        h2: content?.classList.contains("h2") ?? false,
        classes: content?.className ?? "",
      };
    }));
}

async function selectedLabels() {
  return (await rootSnapshot()).filter((row) => row.selected).map((row) => row.label);
}

async function waitForRoots(predicate, message) {
  await browser.waitUntil(async () => predicate(await rootSnapshot()), {
    timeout: 10_000,
    interval: 100,
    timeoutMsg: message,
  });
}

async function rootElement(label) {
  const roots = await browser.$$(".page-blocks > .ls-block");
  for (const root of roots) {
    if (await root.getAttribute("data-page-preamble")) continue;
    const content = await root.$(":scope > .block-main > .block-content-wrapper > .block-content");
    if (await content.isExisting() && (await content.getText()).trim() === label) return root;
  }
  throw new Error(`could not find root ${label}: ${JSON.stringify(await rootSnapshot())}`);
}

async function installWebviewErrorCapture() {
  await browser.execute(() => {
    const windowWithCapture = window;
    windowWithCapture.__gh240WebviewErrors = [];
    window.addEventListener("error", (event) => {
      windowWithCapture.__gh240WebviewErrors.push(`onerror: ${event.message || String(event.error)}`);
    });
    window.addEventListener("unhandledrejection", (event) => {
      windowWithCapture.__gh240WebviewErrors.push(`unhandledrejection: ${String(event.reason?.message || event.reason)}`);
    });
    const originalError = console.error;
    console.error = (...args) => {
      windowWithCapture.__gh240WebviewErrors.push(`console.error: ${args.map((arg) => arg?.message || String(arg)).join(" ")}`);
      originalError(...args);
    };
  });
}

async function webviewErrors() {
  return browser.execute(() => [...(window.__gh240WebviewErrors || [])]);
}

async function assertNoWebviewErrors(label) {
  const errors = await webviewErrors();
  assert(errors.length === 0, `uncaught webview errors at ${label}`, errors);
}

async function openSession(session = "initial") {
  browser = await remote({
    hostname: "127.0.0.1",
    port: DRIVER,
    path: "/",
    logLevel: "error",
    connectionRetryCount: 1,
    connectionRetryTimeout: 60_000,
    capabilities: tauriCapabilities(APP, session, process.platform, webviewTarget.debuggerAddress),
  });
  await browser.$(".page-title, .ls-block").waitForExist({ timeout: 20_000 });
  await installWebviewErrorCapture();
}

async function openPage(name) {
  // The shared link route already tolerates `[[ ]]` decoration and re-finds the
  // link on each attempt, so a rendered journal link replaced between discovery
  // and click can no longer strand this journey. See
  // scripts/lib/e2e-navigation.mjs.
  await openPageByLink(browser, name, { timeout: 15_000 });
  await waitForRoots((rows) => rows.length === 5, "routed page roots did not load");
}

async function selectBAndC() {
  phase("select B+C with the editor keyboard path", "B+C are the exact active block selection");
  const b = await rootElement("B");
  const content = await b.$(":scope > .block-main > .block-content-wrapper");
  await content.click();
  await browser.$("textarea.block-editor").waitForExist({ timeout: 5_000 });
  await browser.keys([Key.Escape]);
  await waitForRoots((rows) => rows.filter((row) => row.selected).map((row) => row.label).join(",") === "B", "Escape did not select B");
  await browser.keys([Key.Shift, Key.ArrowDown]);
  await waitForRoots((rows) => rows.filter((row) => row.selected).map((row) => row.label).join(",") === "B,C", "Shift+ArrowDown did not extend B selection to C");
}

async function ensureBAndCSelected() {
  const selected = await selectedLabels();
  if (selected.join(",") === "B,C") return "preserved";
  await selectBAndC();
  return "reselected";
}

async function openContextMenuOnA() {
  phase("open actual context menu on unselected A", "the rendered context menu opens on A while B+C remain selected");
  const opened = await browser.execute(() => {
    const root = [...document.querySelectorAll(".page-blocks > .ls-block")].find((candidate) => {
      const content = candidate.querySelector(":scope > .block-main > .block-content-wrapper > .block-content");
      return content?.textContent?.trim() === "A";
    });
    const surface = root?.querySelector(":scope > .block-main > .block-content-wrapper");
    if (!(surface instanceof HTMLElement)) return { found: false };
    const rect = surface.getBoundingClientRect();
    const event = new MouseEvent("contextmenu", {
      bubbles: true,
      cancelable: true,
      clientX: rect.left + Math.max(4, Math.min(rect.width - 4, rect.width / 2)),
      clientY: rect.top + Math.max(4, Math.min(rect.height - 4, rect.height / 2)),
      view: window,
    });
    surface.dispatchEvent(event);
    return { found: true, defaultPrevented: event.defaultPrevented };
  });
  assert(opened.found && opened.defaultPrevented, "context-menu dispatch did not reach the rendered A block menu", opened);
  await browser.$(".ctx-menu").waitForExist({ timeout: 5_000 });
}

async function chooseHeading2() {
  phase("choose H2 through the rendered menu", "only B+C render as H2 and remain selected");
  const h2 = await browser.$('button.ctx-h[title="Heading 2"]');
  await h2.waitForExist({ timeout: 5_000 });
  await h2.click();
  await waitForRoots((rows) => {
    const h2Labels = rows.filter((row) => row.h2).map((row) => row.label);
    const selection = rows.filter((row) => row.selected).map((row) => row.label);
    return h2Labels.join(",") === "B,C" && selection.join(",") === "B,C" && !rows.find((row) => row.label === "A")?.h2;
  }, "H2 did not apply exactly to B+C while retaining their block selection");
}

async function undoHeading() {
  phase("undo the batch H2 operation", "one actual undo removes H2 from both B and C");
  await browser.keys(["Control", "z"]);
  await waitForRoots((rows) => rows.every((row) => !row.h2), "one undo did not remove both H2 renderings");
}

async function dragSelectionFromAToAfterE() {
  phase("drag B+C from A bullet to after E", "visible roots become A,D,E,B,C and B+C remain grouped in order");
  const mode = await ensureBAndCSelected();
  fs.writeFileSync(path.join(ARTIFACTS, "selection-before-drag.json"), JSON.stringify({ mode, roots: await rootSnapshot() }, null, 2) + "\n");
  const points = await browser.execute(() => {
    const rootFor = (label) => [...document.querySelectorAll(".page-blocks > .ls-block")].find((root) =>
      root.querySelector(":scope > .block-main > .block-content-wrapper > .block-content")?.textContent?.trim() === label,
    );
    const aBullet = rootFor("A")?.querySelector(":scope > .block-main .bullet-container");
    const eRoot = rootFor("E");
    const eMain = eRoot?.querySelector(":scope > .block-main");
    if (!(aBullet instanceof HTMLElement) || !(eRoot instanceof HTMLElement) || !(eMain instanceof HTMLElement)) return null;
    const start = aBullet.getBoundingClientRect();
    const targetRoot = eRoot.getBoundingClientRect();
    const target = eMain.getBoundingClientRect();
    return {
      start: { x: start.left + start.width / 2, y: start.top + start.height / 2 },
      // Stay in OG's shallow (<=50 px) sibling strip; moving deeper into the
      // row explicitly requests child insertion.
      end: { x: targetRoot.left + 35, y: target.top + target.height * 0.75 },
    };
  });
  assert(points && Number.isFinite(points.start.x) && Number.isFinite(points.end.y), "live A bullet/E row drag coordinates were unavailable", points);
  const steps = 8;
  const actions = [
    { type: "pointerMove", duration: 0, x: Math.round(points.start.x), y: Math.round(points.start.y) },
    { type: "pointerDown", button: 0 },
  ];
  for (let index = 1; index <= steps; index += 1) {
    actions.push({
      type: "pointerMove",
      duration: 35,
      x: Math.round(points.start.x + ((points.end.x - points.start.x) * index) / steps),
      y: Math.round(points.start.y + ((points.end.y - points.start.y) * index) / steps),
    });
  }
  actions.push({ type: "pointerUp", button: 0 });
  await browser.performActions([{ type: "pointer", id: "gh240-native-mouse", parameters: { pointerType: "mouse" }, actions }]);
  await browser.releaseActions();
  await waitForRoots((rows) => rows.map((row) => row.label).join(",") === EXPECTED_ORDER.join(","), "bullet pointer drag did not produce A,D,E,B,C");
}

function diskOrder(text) {
  return text.split(/\r?\n/).flatMap((line) => {
    const match = /^- ([A-E])$/.exec(line);
    return match ? [match[1]] : [];
  });
}

async function waitForDurableOrder() {
  phase("wait for durable save", "the page file records root order A,D,E,B,C");
  const text = await waitForFileText(PAGE_FILE, (body) => diskOrder(body).join(",") === EXPECTED_ORDER.join(","), "GH #240 root order");
  fs.writeFileSync(path.join(ARTIFACTS, "durable-page.md"), text);
}

async function relaunchAndAssert() {
  phase("end session and relaunch exact artifact", "the same graph/XDG/artifact routes back to the named page with durable A,D,E,B,C");
  await browser.deleteSession();
  browser = undefined;
  killDriverTree();
  await sleep(900);
  await startDriverTree("relaunch");
  await openSession("relaunch");
  await openPage(PAGE);
  await waitForRoots((rows) => rows.map((row) => row.label).join(",") === EXPECTED_ORDER.join(","), "relaunch did not render persisted A,D,E,B,C order");
  await assertNoWebviewErrors("post-relaunch");
}

function failureClassification(error) {
  const message = String(error);
  if (/Failed to initialize gtk|ECONNREFUSED|headers timeout|WebDriver.*(connect|session)|driver.*exit/i.test(message)) return "infrastructure";
  return "ambiguous";
}

async function preserveFailureCapsule(error) {
  const observed = browser ? await rootSnapshot().catch((snapshotError) => ({ snapshotError: String(snapshotError) })) : { browser: "no session" };
  const errors = browser ? await webviewErrors().catch((webviewError) => [`capture failed: ${String(webviewError)}`]) : [];
  try { await browser?.saveScreenshot(path.join(ARTIFACTS, "failure.png")); } catch {}
  try { fs.writeFileSync(path.join(ARTIFACTS, "page-on-failure.md"), fs.readFileSync(PAGE_FILE, "utf8")); } catch {}
  const capsule = {
    classification: failureClassification(error),
    step,
    expected,
    observed,
    webviewErrors: errors,
    error: String(error),
    candidateCommit: CANDIDATE_COMMIT,
    artifact: APP,
    graph: GRAPH,
    xdg: XDG,
    driverLog: driverLogPath,
    screenshot: path.join(ARTIFACTS, "failure.png"),
  };
  fs.writeFileSync(path.join(ARTIFACTS, "failure-capsule.json"), JSON.stringify(capsule, null, 2) + "\n");
  console.error(`GH240 E2E FAILURE (${capsule.classification}): capsule retained at ${path.join(ARTIFACTS, "failure-capsule.json")}`);
}

try {
  phase("launch exact artifact", "the native app opens under its private GH240 graph and XDG state");
  await openSession();
  await openPage(PAGE);
  await selectBAndC();
  await openContextMenuOnA();
  await chooseHeading2();
  await assertNoWebviewErrors("heading command");
  await undoHeading();
  await dragSelectionFromAToAfterE();
  await assertNoWebviewErrors("pointer drag");
  await waitForDurableOrder();
  await relaunchAndAssert();
  console.log(`PASS: GH #240 native selection-owned H2, one undo, pointer bullet drag, and durable relaunch on ${CANDIDATE_COMMIT}`);
} catch (error) {
  failure = true;
  await preserveFailureCapsule(error);
  process.exitCode = 1;
} finally {
  try { await browser?.deleteSession(); } catch {}
  killDriverTree();
  try { fs.closeSync(driverLog); } catch {}
  stopDisplay();
  if (!failure) fs.rmSync(TMP, { recursive: true, force: true });
}
