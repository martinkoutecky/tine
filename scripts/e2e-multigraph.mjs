// Real-app regression for ADR 0038: one process, two graph windows. Uses only
// disposable /tmp graphs and an isolated app-data directory.
//
// og batches 7-10 (I-20, late landing): an edit typed in one graph and not yet
// saved when the same window switches to another graph must land in the graph
// it was typed in, never in the new one, and nothing from the old graph may
// paint into the window after the switch has settled.
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { APP_ID } from "./lib/app-identity.mjs";
import { openJournals, openPageByName } from "./lib/e2e-navigation.mjs";
import { ensurePrivateSessionBus } from "./lib/e2e-session-bus.mjs";

ensurePrivateSessionBus();

const APP = process.env.TINE_APP || "/tmp/tine-multiprocess";
const TD = process.env.TAURI_DRIVER || "/aux/koutecky/logseq/.toolchain/cargo/bin/tauri-driver";
const WD = process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver";
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4454);
const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4455);
const WAIT_TIMEOUT = Number(process.env.E2E_WAIT_TIMEOUT_MS || 60_000);
const ROOT = process.env.E2E_TMP_DIR || `/tmp/tine-multigraph-e2e-${process.pid}`;
const A = `${ROOT}/alpha`;
const B = `${ROOT}/beta`;
const XDG = `${ROOT}/xdg`;
const now = new Date();
const JOURNAL_FILE = [now.getFullYear(), now.getMonth() + 1, now.getDate()]
  .map((value, index) => (index === 0 ? String(value) : String(value).padStart(2, "0")))
  .join("_") + ".md";
const journalPath = (root) => `${root}/journals/${JOURNAL_FILE}`;

function seed(root, sentinel, page) {
  fs.mkdirSync(`${root}/pages`, { recursive: true });
  fs.mkdirSync(`${root}/journals`, { recursive: true });
  fs.mkdirSync(`${root}/logseq`, { recursive: true });
  fs.writeFileSync(journalPath(root), `- ${sentinel}\n- [[${page}]]\n`);
  fs.writeFileSync(`${root}/pages/${page}.md`, `- ${page} body\n`);
}

fs.rmSync(ROOT, { recursive: true, force: true });
seed(A, "ALPHA_SENTINEL", "Alpha Page");
seed(B, "BETA_SENTINEL", "Beta Page");
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(`${XDG}/${dir}`, { recursive: true });
// Both graphs are known, so the in-window graph switcher offers them.
fs.mkdirSync(`${XDG}/data/${APP_ID}`, { recursive: true });
fs.writeFileSync(`${XDG}/data/${APP_ID}/tine-settings.json`, JSON.stringify({
  known_graphs: [{ name: "alpha", path: A }, { name: "beta", path: B }],
  last_graph_path: A,
}, null, 2));
const LATE = "LATE_ALPHA_EDIT";
const graphFiles = (root) => ["pages", "journals"].flatMap((dir) =>
  fs.readdirSync(`${root}/${dir}`).map((name) => `${root}/${dir}/${name}`));
const graphContains = (root, text) => graphFiles(root).filter((file) => fs.readFileSync(file, "utf8").includes(text));

const env = {
  ...process.env,
  TINE_GRAPH: A,
  XDG_DATA_HOME: `${XDG}/data`,
  XDG_CONFIG_HOME: `${XDG}/config`,
  XDG_CACHE_HOME: `${XDG}/cache`,
  WEBKIT_DISABLE_DMABUF_RENDERER: "1",
  LIBGL_ALWAYS_SOFTWARE: "1",
  WEBKIT_DISABLE_COMPOSITING_MODE: "1",
  GDK_BACKEND: "x11",
};

// Keep the native-driver trace with the suite evidence even when the scenario
// fails before it can print a useful WebDriver error.
const tdLogPath = process.env.E2E_ARTIFACT_DIR
  ? `${process.env.E2E_ARTIFACT_DIR}/tauri-driver.log`
  : `${ROOT}/tauri-driver.log`;
const tdLog = fs.openSync(tdLogPath, "w");
const td = spawn(
  TD,
  ["--port", String(DRIVER_PORT), "--native-port", String(NATIVE_PORT), "--native-driver", WD],
  { env, stdio: ["ignore", tdLog, tdLog], detached: true }
);
await sleep(2500);

let browser;
const forwarded = [];
const productFailures = [];
async function waitForGraphName(name) {
  await browser.waitUntil(() => browser.execute((wanted) =>
    document.querySelector(".graph-switch-name")?.textContent?.trim() === wanted, name), {
    timeout: WAIT_TIMEOUT, interval: 200, timeoutMsg: `the window did not switch to graph ${name}`,
  });
}
async function bodyEventually(text, timeout = 15_000) {
  const deadline = Date.now() + timeout;
  let toasts = [];
  while (Date.now() < deadline) {
    const state = await browser.execute((wanted) => ({
      has: document.body.innerText.includes(wanted),
      toasts: [...document.querySelectorAll(".toast")].map((node) => node.textContent),
    }), text);
    toasts = [...new Set([...toasts, ...state.toasts])];
    if (state.has) return { ok: true, toasts };
    await sleep(200);
  }
  return { ok: false, toasts };
}
/** Pick a known graph from this window's graph switcher (in-place switch). */
async function chooseGraph(root) {
  await browser.execute(() => document.querySelector(".graph-switch-btn")?.dispatchEvent(
    new MouseEvent("click", { bubbles: true, button: 0, view: window })));
  await browser.waitUntil(() => browser.execute(() => document.querySelectorAll(".graph-switch-row").length > 0), {
    timeout: 5000, timeoutMsg: "the graph switcher did not list known graphs",
  });
  return browser.execute((target) => {
    const row = [...document.querySelectorAll(".graph-switch-row")].find((candidate) => candidate.getAttribute("title") === target);
    row?.dispatchEvent(new MouseEvent("click", { bubbles: true, button: 0, view: window }));
    return Boolean(row);
  }, root);
}
try {
  browser = await remote({
    hostname: "127.0.0.1",
    port: DRIVER_PORT,
    path: "/",
    capabilities: {
      browserName: "wry",
      "wdio:enforceWebDriverClassic": true,
      "tauri:options": { application: APP },
    },
    logLevel: "error",
    connectionRetryCount: 1,
    connectionRetryTimeout: 60000,
  });

  await browser.$(".graph-switch-btn").waitForExist({ timeout: WAIT_TIMEOUT });
  await browser.waitUntil(async () => (await browser.$("body").getText()).includes("ALPHA_SENTINEL"), {
    timeout: WAIT_TIMEOUT,
    timeoutMsg: "alpha graph never painted",
  });
  // ---- I-20: an unsaved edit racing an in-window graph switch -------------
  // Runs before any peer window exists: after a graph has had a peer window
  // that WebDriver closed, choosing it in place is a silent no-op on both og
  // and master (RECEIPT-12E P3), which is not this journey's contract.
  await openPageByName(browser, "Alpha Page");
  const alphaBlock = await browser.$("//div[contains(concat(' ', normalize-space(@class), ' '), ' block-content ') and normalize-space(.) = 'Alpha Page body']");
  await alphaBlock.waitForExist({ timeout: 10_000 });
  await alphaBlock.click();
  await browser.$("textarea.block-editor").waitForExist({ timeout: 5000 });
  await browser.keys(["End"]);
  await browser.keys(` ${LATE}`);
  // Switch while the edit is still unsaved (no Escape, no wait for the save).
  const switched = await chooseGraph(B);
  if (!switched) throw new Error("the beta row was not offered by the graph switcher");
  await waitForGraphName("beta");
  // The switched window shows beta's own journal. A feed that fails to load
  // here is recorded as a product failure and the journey continues.
  const betaFeed = await bodyEventually("BETA_SENTINEL");
  if (!betaFeed.ok) productFailures.push(`after the in-place switch the beta window never showed beta's journal; toasts ${JSON.stringify(betaFeed.toasts)}`);
  // The typed edit lands in alpha's own page file, and nowhere in beta.
  await browser.waitUntil(() => fs.readFileSync(`${A}/pages/Alpha Page.md`, "utf8").includes(LATE), {
    timeout: 10_000,
    timeoutMsg: `the edit typed before the switch never reached alpha: ${JSON.stringify(fs.readFileSync(`${A}/pages/Alpha Page.md`, "utf8"))}; in beta: ${JSON.stringify(graphContains(B, LATE))}`,
  });
  await sleep(2000);
  const leakedFiles = graphContains(B, LATE);
  if (leakedFiles.length) throw new Error(`the alpha edit landed in beta: ${JSON.stringify(leakedFiles)}`);
  if (fs.existsSync(`${B}/pages/Alpha Page.md`)) throw new Error("the switch created an Alpha Page in beta");
  const settled = await browser.execute(() => ({
    name: document.querySelector(".graph-switch-name")?.textContent?.trim() ?? "",
    text: document.body.innerText,
    editing: Boolean(document.querySelector("textarea.block-editor")),
  }));
  if (settled.name !== "beta") throw new Error(`the switched window names graph ${JSON.stringify(settled.name)}`);
  for (const alphaText of [LATE, "Alpha Page body", "ALPHA_SENTINEL"]) {
    if (settled.text.includes(alphaText)) throw new Error(`alpha content ${JSON.stringify(alphaText)} painted into the beta window after the switch settled`);
  }
  // Switching back shows the edit where it was typed.
  const back = await chooseGraph(A);
  if (!back) throw new Error("the alpha row was not offered by the graph switcher");
  await waitForGraphName("alpha");
  await openPageByName(browser, "Alpha Page");
  if (!(await bodyEventually(LATE)).ok) throw new Error("the edit is not shown after switching back to alpha");
  // Leave alpha on its journal, the state the multi-window checks below start from.
  await openJournals(browser);
  if (!(await bodyEventually("ALPHA_SENTINEL")).ok) throw new Error("alpha's journal did not return after switching back");

  const initial = await browser.getWindowHandles();
  // Tauri exposes the hidden static capture webview as a WebDriver handle too.
  if (initial.length !== 2) throw new Error(`expected main + hidden capture, got ${initial.length}`);
  const alpha = await browser.getWindowHandle();

  forwarded.push(spawn(APP, [B], { env: { ...env, TINE_GRAPH: "" }, stdio: "ignore" }));
  await browser.waitUntil(async () => (await browser.getWindowHandles()).length === 3, {
    timeout: WAIT_TIMEOUT,
    timeoutMsg: "forwarded beta launch did not create a second window",
  });

  const handles = await browser.getWindowHandles();
  const beta = handles.find((handle) => !initial.includes(handle));
  if (!beta) throw new Error("could not identify beta window");
  await browser.switchToWindow(beta);
  await browser.$(".graph-switch-btn").waitForExist({ timeout: WAIT_TIMEOUT });
  await browser.waitUntil(async () => (await browser.$("body").getText()).includes("BETA_SENTINEL"), {
    timeout: WAIT_TIMEOUT,
    timeoutMsg: "beta graph never painted",
  });
  const betaName = await browser.$(".graph-switch-name").getAttribute("textContent");

  // Opening the same canonical graph again focuses the existing beta window;
  // it must not create a third writer/window.
  forwarded.push(spawn(APP, [B], { env: { ...env, TINE_GRAPH: "" }, stdio: "ignore" }));
  await sleep(1500);
  const afterDuplicate = await browser.getWindowHandles();
  if (afterDuplicate.length !== 3) {
    throw new Error(`duplicate beta launch created ${afterDuplicate.length} windows`);
  }

  // The last-focused route used by quick capture follows the active graph.
  await browser.$("body").click();
  await browser.waitUntil(async () =>
    (await browser.execute(async () => globalThis.__TAURI_INTERNALS__.invoke("capture_target"))) === "graph-1", {
    timeout: WAIT_TIMEOUT,
    timeoutMsg: "capture target did not follow beta focus",
  });
  const betaTarget = await browser.execute(async () =>
    globalThis.__TAURI_INTERNALS__.invoke("capture_target")
  );

  // Forwarding alpha is an explicit graph activation. Capture routing updates
  // synchronously in the backend instead of depending on a later OS focus
  // event, so observe that global state from the still-valid beta webview
  // before switching WebDriver handles.
  forwarded.push(spawn(APP, [A], { env: { ...env, TINE_GRAPH: "" }, stdio: "ignore" }));
  await browser.waitUntil(async () =>
    (await browser.execute(async () => globalThis.__TAURI_INTERNALS__.invoke("capture_target"))) === "main", {
    timeout: WAIT_TIMEOUT,
    timeoutMsg: "capture target did not follow explicit alpha activation",
  });
  const alphaTarget = await browser.execute(async () =>
    globalThis.__TAURI_INTERNALS__.invoke("capture_target")
  );
  if (alphaTarget !== "main") throw new Error(`capture target did not follow alpha focus: ${alphaTarget}`);

  // A capture carries the native binding generation of the graph it was
  // shown for (OG-K3); the receiver refuses a stale one. Read alpha's current
  // binding from alpha's own window: loading the graph it already holds
  // returns that binding without changing it.
  const betaHandle = await browser.getWindowHandle();
  await browser.switchToWindow(alpha);
  const alphaBinding = await browser.execute(async (root) =>
    globalThis.__TAURI_INTERNALS__.invoke("load_graph", { path: root }), A
  );
  await browser.switchToWindow(betaHandle);
  if (alphaBinding?.kind !== "already_current" || !Number.isSafeInteger(alphaBinding.binding_generation)) {
    throw new Error(`alpha did not report its current binding: ${JSON.stringify(alphaBinding)}`);
  }
  await browser.execute(async (target, bindingGeneration) =>
    globalThis.__TAURI_INTERNALS__.invoke("plugin:event|emit_to", {
      target: { kind: "AnyLabel", label: target },
      event: "quick-capture",
      payload: {
        id: "e2e-multigraph-capture",
        target,
        bindingGeneration,
        text: "- E2E_CAPTURE_ONLY_ALPHA",
        title: "",
      },
    }), alphaTarget, alphaBinding.binding_generation
  );
  await browser.waitUntil(() =>
    fs.readFileSync(journalPath(A), "utf8").includes("E2E_CAPTURE_ONLY_ALPHA"), {
    timeout: 10000,
    timeoutMsg: "quick capture did not reach alpha",
  });
  if (fs.readFileSync(journalPath(B), "utf8").includes("E2E_CAPTURE_ONLY_ALPHA")) {
    throw new Error("quick capture leaked into beta");
  }

  // External changes in alpha are dispatched only to alpha.
  await browser.switchToWindow(alpha);
  fs.writeFileSync(journalPath(A), "- ALPHA_SENTINEL\n- ALPHA_WATCHER_UPDATE\n");
  await browser.waitUntil(async () => (await browser.$("body").getText()).includes("ALPHA_WATCHER_UPDATE"), {
    timeout: 10000,
    timeoutMsg: "alpha watcher event was not delivered",
  });
  await browser.switchToWindow(beta);
  const betaBody = await browser.$("body").getText();
  if (betaBody.includes("ALPHA_WATCHER_UPDATE")) throw new Error("alpha watcher event leaked into beta");

  // Closing beta must leave alpha and the process alive.
  await browser.closeWindow();
  await browser.waitUntil(async () => (await browser.getWindowHandles()).length === 2, {
    timeout: 10000,
    timeoutMsg: "closing beta did not leave exactly one graph window",
  });
  await browser.switchToWindow(alpha);
  const alphaBody = await browser.$("body").getText();
  if (!alphaBody.includes("ALPHA_SENTINEL")) throw new Error("alpha died when beta closed");

  if (productFailures.length) throw new Error(`product failures: ${JSON.stringify(productFailures)}`);
  console.log(JSON.stringify({
    lateEditLandedInOwnGraph: true,
    graphWindows: handles.length - 1,
    betaName,
    betaTarget,
    alphaTarget,
    explicitActivationObserved: true,
    duplicateGraphWindowCount: afterDuplicate.length - 1,
    watcherIsolated: true,
    quickCaptureIsolated: true,
    peerCloseKeptAlpha: true,
  }, null, 2));
} catch (error) {
  try {
    console.error("state:", JSON.stringify(await browser.execute(() => ({
      graph: document.querySelector(".graph-switch-name")?.textContent?.trim() ?? null,
      title: document.querySelector("h1.page-title")?.textContent ?? null,
      rows: [...document.querySelectorAll(".graph-switch-row")].map((row) => row.getAttribute("title")),
      editing: document.querySelector("textarea.block-editor")?.value ?? null,
      toasts: [...document.querySelectorAll(".toast")].map((node) => node.textContent),
      body: document.body.innerText.slice(0, 600),
    }))));
  } catch {}
  for (const root of [A, B]) {
    try {
      for (const file of graphFiles(root)) console.error(`${file}:`, JSON.stringify(fs.readFileSync(file, "utf8")));
    } catch {}
  }
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  for (const child of forwarded) child.kill("SIGKILL");
  try { process.kill(-td.pid, "SIGKILL"); } catch {}
  fs.closeSync(tdLog);
}
