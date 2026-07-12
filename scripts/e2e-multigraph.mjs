// Real-app regression for ADR 0038: one process, two graph windows. Uses only
// disposable /tmp graphs and an isolated app-data directory.
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";

const APP = process.env.TINE_APP || "/tmp/tine-multiprocess";
const TD = process.env.TAURI_DRIVER || "/aux/koutecky/logseq/.toolchain/cargo/bin/tauri-driver";
const WD = process.env.WEBKIT_DRIVER || "/tmp/tine-webdriver/usr/bin/WebKitWebDriver";
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4454);
const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4455);
const WAIT_TIMEOUT = Number(process.env.E2E_WAIT_TIMEOUT_MS || 60_000);
const ROOT = "/tmp/tine-multigraph-e2e";
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

const tdLog = fs.openSync(`${ROOT}/tauri-driver.log`, "w");
const td = spawn(
  TD,
  ["--port", String(DRIVER_PORT), "--native-port", String(NATIVE_PORT), "--native-driver", WD],
  { env, stdio: ["ignore", tdLog, tdLog], detached: true }
);
await sleep(2500);

let browser;
const forwarded = [];
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

  // Forwarding alpha focuses its already-open owner. This produces a real OS
  // focus event (switchToWindow alone only changes WebDriver's target).
  forwarded.push(spawn(APP, [A], { env: { ...env, TINE_GRAPH: "" }, stdio: "ignore" }));
  await browser.switchToWindow(alpha);
  await browser.waitUntil(async () =>
    (await browser.execute(async () => globalThis.__TAURI_INTERNALS__.invoke("capture_target"))) === "main", {
    timeout: WAIT_TIMEOUT,
    timeoutMsg: "capture target did not follow alpha focus",
  });
  const alphaTarget = await browser.execute(async () =>
    globalThis.__TAURI_INTERNALS__.invoke("capture_target")
  );
  if (alphaTarget !== "main") throw new Error(`capture target did not follow alpha focus: ${alphaTarget}`);

  await browser.execute(async (target) =>
    globalThis.__TAURI_INTERNALS__.invoke("plugin:event|emit_to", {
      target: { kind: "AnyLabel", label: target },
      event: "quick-capture",
      payload: {
        id: "e2e-multigraph-capture",
        target,
        text: "- E2E_CAPTURE_ONLY_ALPHA",
        title: "",
      },
    }), alphaTarget
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

  console.log(JSON.stringify({
    graphWindows: handles.length - 1,
    betaName,
    betaTarget,
    alphaTarget,
    duplicateGraphWindowCount: afterDuplicate.length - 1,
    watcherIsolated: true,
    quickCaptureIsolated: true,
    peerCloseKeptAlpha: true,
  }, null, 2));
} finally {
  try { await browser?.deleteSession(); } catch {}
  for (const child of forwarded) child.kill("SIGKILL");
  try { process.kill(-td.pid, "SIGKILL"); } catch {}
  fs.closeSync(tdLog);
}
