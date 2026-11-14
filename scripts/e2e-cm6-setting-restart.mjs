// CodeMirror editor setting across an app restart, in the real app (development build only).
// A fresh profile uses CM6. The user changes engines in Settings, quits, relaunches, and the first block
// edit must mount the CodeMirror editor, with no off/on in Settings first. Judged by
// which editor element the DOM mounts, on the routed page and the right sidebar.
import { spawn, execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { remote } from "webdriverio";
import { APP_ID } from "./lib/app-identity.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";
import { ensurePrivateSessionBus } from "./lib/e2e-session-bus.mjs";

ensurePrivateSessionBus();

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const DRIVER = process.env.TAURI_DRIVER || path.join(process.env.CARGO_HOME ?? "", "bin/tauri-driver");
const ARTIFACT = process.env.E2E_ARTIFACT_DIR || path.join(ROOT, "artifacts/og-cm6-setting-restart/native");
fs.mkdirSync(ARTIFACT, { recursive: true });
const TMP = fs.mkdtempSync(path.join(process.env.E2E_TMP_DIR || ARTIFACT, "journey-"));
const GRAPH = path.join(TMP, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
for (const dir of ["data", "config", "cache", "home", "state"]) fs.mkdirSync(path.join(TMP, dir));
fs.writeFileSync(path.join(GRAPH, "logseq/config.edn"), "{}\n");
fs.writeFileSync(path.join(GRAPH, "pages", "Restart Demo.md"), "- alpha\n- beta\n- see [[Side Demo]]\n");
fs.writeFileSync(path.join(GRAPH, "pages", "Side Demo.md"), "- side one\n- side two\n");
// A second graph, shown in a second window later.
const PEER = path.join(TMP, "peer");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(PEER, dir), { recursive: true });
fs.writeFileSync(path.join(PEER, "logseq/config.edn"), "{}\n");
fs.writeFileSync(path.join(PEER, "pages", "Peer Demo.md"), "- peer one\n- peer two\n");

const env = { ...process.env, HOME: path.join(TMP, "home"), TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: path.join(TMP, "data"), XDG_CONFIG_HOME: path.join(TMP, "config"), XDG_CACHE_HOME: path.join(TMP, "cache"),
  XDG_STATE_HOME: path.join(TMP, "state"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4897);
const log = fs.openSync(path.join(TMP, "driver.log"), "w");
const startDriver = () => spawn(DRIVER, ["--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log], detached: true });
let driver = startDriver();
/** The app processes of this journey only: our binary, started with our scratch profile. */
function myAppPids() {
  const own = [];
  for (const pid of fs.readdirSync("/proc").filter((name) => /^\d+$/.test(name))) {
    try { if (fs.readlinkSync(`/proc/${pid}/exe`) === fs.realpathSync(process.env.TINE_APP_BINARY || APP) && fs.readFileSync(`/proc/${pid}/environ`, "utf8").includes(TMP)) own.push(Number(pid)); } catch { /* gone or not ours */ }
  }
  return own;
}
let browser;
const proof = { commit: execFileSync("git", ["rev-parse", "HEAD"], { cwd: ROOT, encoding: "utf8" }).trim(), binarySha256: createHash("sha256").update(fs.readFileSync(APP)).digest("hex"), journey: "CodeMirror editor setting survives an app restart", expected: "after relaunch the first block edit mounts the CodeMirror editor", steps: [] };

async function launch() {
  const session = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 2, connectionRetryTimeout: 30_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } } });
  await session.$(".app-container").waitForExist({ timeout: 30_000 });
  await session.execute(() => { for (const close of document.querySelectorAll(".toast-sticky .toast-close")) close.click(); });
  return session;
}

async function restart() {
  for (const pid of myAppPids()) process.kill(pid, "SIGKILL");
  try { await browser.deleteSession(); } catch { /* the app is gone */ }
  const exited = driver.exitCode === null && driver.signalCode === null
    ? new Promise(resolve => driver.once("exit", resolve)) : Promise.resolve();
  try { process.kill(-driver.pid, "SIGTERM"); } catch { /* already gone */ }
  await exited;
  driver = startDriver();
  browser = await launch();
}

const EDITOR = '[data-block-editor="cm6"], textarea.block-editor';
async function kindAfterClick(scope, text) {
  const id = await browser.execute((s, t) => {
    const row = [...document.querySelectorAll(`${s} [data-block-id]`)].find((el) => el.querySelector(".block-content-wrapper")?.textContent.trim() === t);
    if (!row) return null;
    row.scrollIntoView({ block: "center" });
    return row.getAttribute("data-block-id");
  }, scope, text);
  if (!id) throw new Error(`no block reading ${JSON.stringify(text)} in ${scope}`);
  await browser.$(`${scope} [data-block-id="${id}"] .block-content-wrapper`).click();
  const editor = await browser.$(`${scope} ${EDITOR}`);
  await editor.waitForExist({ timeout: 10_000 });
  const kind = await browser.execute((s) => document.querySelector(`${s} [data-block-editor="cm6"]`) ? "cm6" : "textarea", scope);
  await browser.keys("Escape");
  await browser.waitUntil(() => browser.execute((s) => !document.querySelector(`${s} ${'[data-block-editor="cm6"], textarea.block-editor'}`), scope), { timeout: 5000 });
  return kind;
}

try {
  browser = await launch();
  await openPageByName(browser, "Restart Demo");
  if (await kindAfterClick(".main-content", "alpha") !== "cm6") throw new Error("fresh profile did not use CodeMirror");
  proof.steps.push("fresh profile: first routed-page edit uses CM6");
  // 1. Exercise the classic opt-out, then store an explicit CM6 choice before restart.
  await browser.$('button[title^="Settings"]').click();
  await browser.$(".settings-modal").waitForDisplayed({ timeout: 10_000 });
  await browser.$(".settings-search-input").setValue("CodeMirror");
  const result = await browser.$(".settings-search-result");
  await result.waitForDisplayed({ timeout: 10_000 });
  await result.click();
  const toggle = await browser.$('[data-setting-id="settings-cm6-label"] [role="switch"]');
  await toggle.waitForDisplayed({ timeout: 10_000 });
  if (await toggle.getAttribute("aria-checked") !== "false") throw new Error("classic opt-out was on in a fresh profile");
  await toggle.click();
  await browser.waitUntil(async () => (await toggle.getAttribute("aria-checked")) === "true", { timeout: 5000 });
  await toggle.click();
  await browser.waitUntil(async () => (await toggle.getAttribute("aria-checked")) === "false", { timeout: 5000 });
  proof.steps.push("Settings: classic on, then off to store CM6");
  await browser.keys(["Escape"]);
  // Persisted before the restart: the device preference file holds the choice.
  const settingsFile = path.join(TMP, "data", APP_ID, "tine-settings.json");
  await waitForFileText(settingsFile, text => JSON.parse(text).experimental_cm6_editor === true);
  proof.steps.push("tine-settings.json holds experimental_cm6_editor=true");

  // 2. The app dies uncleanly (kill -9, as after a crash) and is relaunched with the same profile;
  //    Tine then reports "did not close cleanly".
  await restart();
  proof.steps.push("killed with SIGKILL and relaunched");

  // Journal view straight after launch: whatever editor is open on its own, or opened by the first click.
  const early = await browser.execute(() => ({ cm6: !!document.querySelector('[data-block-editor="cm6"]'), textarea: !!document.querySelector("textarea.block-editor"), blocks: document.querySelectorAll("[data-block-id]").length, url: location.hash }));
  proof.steps.push(`journal right after launch: ${JSON.stringify(early)}`);
  await new Promise((r) => setTimeout(r, 1500));
  const later = await browser.execute(() => ({ cm6: !!document.querySelector('[data-block-editor="cm6"]'), textarea: !!document.querySelector("textarea.block-editor"), blocks: document.querySelectorAll("[data-block-id]").length }));
  proof.steps.push(`journal 1.5 s later: ${JSON.stringify(later)}`);
  const firstJournal = await browser.$(".main-content [data-block-id] .block-content-wrapper");
  if (await firstJournal.isExisting()) {
    await firstJournal.click();
    await browser.pause(500);
    proof.steps.push(`journal after a click: ${JSON.stringify(await browser.execute(() => ({ cm6: !!document.querySelector('[data-block-editor="cm6"]'), textarea: !!document.querySelector("textarea.block-editor") })))}`);
    await browser.keys("Escape");
  }
  // Quick Capture is a second, hidden WebView created at startup.
  await new Promise((r) => setTimeout(r, 1500));
  const captureKinds = [];
  for (const handle of await browser.getWindowHandles()) {
    await browser.switchToWindow(handle);
    const title = await browser.getTitle();
    if (!/Quick Capture/.test(title)) { proof.mainHandle = handle; continue; }
    captureKinds.push(await browser.execute(() => ({ cm6: !!document.querySelector('.capture-shell [data-block-editor="cm6"]'), textarea: !!document.querySelector(".capture-shell textarea") })));
  }
  proof.steps.push(`hidden Quick Capture editor after launch: ${JSON.stringify(captureKinds)}`);
  // The global-shortcut path: a second `--capture` process hands off to the running app.
  const second = spawn(APP, ["--capture"], { env, stdio: "ignore", detached: true });
  await new Promise((resolve, reject) => { second.once("exit", (code) => code === 0 ? resolve() : reject(new Error(`--capture forwarder exited ${code}`))); setTimeout(() => reject(new Error("--capture forwarder timed out")), 10_000); });
  let shown = null;
  for (let i = 0; i < 100 && !(shown?.cm6 || shown?.textarea) ; i += 1) {
    for (const handle of await browser.getWindowHandles()) {
      await browser.switchToWindow(handle);
      if (!/Quick Capture/.test(await browser.getTitle())) continue;
      shown = await browser.execute(() => ({ cm6: !!document.querySelector('.capture-shell [data-block-editor="cm6"]'), textarea: !!document.querySelector(".capture-shell textarea"), focusedInEditor: !!document.activeElement?.closest?.("[data-cm6-editor], textarea") }));
    }
    await new Promise((r) => setTimeout(r, 100));
  }
  await new Promise((r) => setTimeout(r, 1500));
  for (const handle of await browser.getWindowHandles()) {
    await browser.switchToWindow(handle);
    if (!/Quick Capture/.test(await browser.getTitle())) continue;
    shown = await browser.execute(() => ({ cm6: !!document.querySelector('.capture-shell [data-block-editor="cm6"]'), textarea: !!document.querySelector(".capture-shell textarea"), focusedInEditor: !!document.activeElement?.closest?.("[data-cm6-editor], textarea") }));
  }
  proof.steps.push(`Quick Capture editor after the shortcut: ${JSON.stringify(shown)}`);
  proof.capture = shown;
  await browser.switchToWindow(proof.mainHandle);
  // 3. The first edits after launch, on each path, with no Settings visit.
  await openPageByName(browser, "Restart Demo");
  const routed = await kindAfterClick(".main-content", "alpha");
  proof.steps.push(`routed page first edit: ${routed}`);
  proof.routed = routed;
  const link = await browser.$('.main-content a[data-ref="Side Demo"], .main-content .page-ref');
  await link.waitForExist({ timeout: 5000 });
  await browser.keys(["Shift"]); // release any state
  await browser.execute(() => {
    const ref = document.querySelector(".main-content .page-ref, .main-content a[data-ref]");
    ref.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, shiftKey: true }));
  });
  await browser.$(".right-sidebar [data-block-id]").waitForExist({ timeout: 10_000 });
  const sidebar = await kindAfterClick(".right-sidebar", "side one");
  proof.steps.push(`right sidebar first edit: ${sidebar}`);
  proof.sidebar = sidebar;
  // 4. A second window on another graph: a choice made in one window reaches the other,
  //    without a restart, in both directions.
  const known = await browser.getWindowHandles();
  spawn(APP, [PEER], { env: { ...env, TINE_GRAPH: "" }, stdio: "ignore", detached: true }).unref();
  await browser.waitUntil(async () => (await browser.getWindowHandles()).length === known.length + 1, { timeout: 30_000, timeoutMsg: "the second graph window did not open" });
  const peer = (await browser.getWindowHandles()).find((handle) => !known.includes(handle));
  const first = proof.mainHandle;
  await browser.switchToWindow(peer);
  await browser.$(".app-container").waitForExist({ timeout: 30_000 });
  await openPageByName(browser, "Peer Demo");
  const peerKind = await kindAfterClick(".main-content", "peer one");
  proof.steps.push(`second window first edit: ${peerKind}`);
  const settingSwitch = async () => {
    await browser.$('button[title^="Settings"]').click();
    await browser.$(".settings-modal").waitForDisplayed({ timeout: 10_000 });
    const search = await browser.$(".settings-search-input");
    await search.setValue("CodeMirror");
    const hit = await browser.$(".settings-search-result");
    await hit.waitForDisplayed({ timeout: 10_000 });
    await hit.click();
    const control = await browser.$('[data-setting-id="settings-cm6-label"] [role="switch"]');
    await control.waitForDisplayed({ timeout: 10_000 });
    return control;
  };
  const flip = async (want) => {
    const control = await settingSwitch();
    if ((await control.getAttribute("aria-checked")) !== String(want)) throw new Error(`Settings did not show classic ${want ? "on" : "off"} before the change`);
    await control.click();
    await browser.waitUntil(async () => (await control.getAttribute("aria-checked")) === String(!want), { timeout: 5000 });
    await browser.keys(["Escape"]);
  };
  const settleKind = async (scope, text, want, where) => {
    let seen = "";
    try {
      await browser.waitUntil(async () => (seen = await kindAfterClick(scope, text)) === want, { timeout: 10_000, interval: 300 });
    } catch { throw new Error(`${where}: first edit mounts ${seen}, wanted ${want}, after the choice was changed in the other window`); }
    proof.steps.push(`${where}: ${want}`);
  };
  await flip(false); // in the second window
  await browser.switchToWindow(first);
  await settleKind(".main-content", "beta", "textarea", "first window after the second window switched CodeMirror off");
  await flip(true); // in the first window
  await browser.switchToWindow(peer);
  await settleKind(".main-content", "peer one", "cm6", "second window after the first window switched CodeMirror on");
  // 5. A slow read. The second window's reply to the stored-choice read is held back (the backend is busy
  //    on a big graph); the first block opened meanwhile must still get the engine the setting names.
  const hook = await browser.execute(() => {
    // The internals object is frozen, so hold back the IPC request itself.
    window.__slowHits = 0;
    const realFetch = window.fetch.bind(window);
    window.fetch = (input, init) => {
      const url = String(input?.url ?? input);
      if (!/get_app_bool/.test(url)) return realFetch(input, init);
      window.__slowHits++;
      return new Promise((resolve, reject) => setTimeout(() => realFetch(input, init).then(resolve, reject), 3000));
    };
    return "fetch wrapped";
  });
  proof.steps.push(`slow-read hook: ${hook}`);
  await browser.switchToWindow(first);
  await flip(false);
  await browser.switchToWindow(peer);
  const slow = await kindAfterClick(".main-content", "peer one");
  proof.steps.push(`second window first edit while the read is still in flight: ${slow} (delayed reads: ${await browser.execute(() => window.__slowHits)})`);
  if (slow !== "textarea") throw new Error(`a block opened while the stored choice was still being read got ${slow}, wanted textarea`);
  if (routed !== "cm6" || sidebar !== "cm6" || !proof.capture?.cm6) throw new Error(`stored CodeMirror choice ignored after restart: routed=${routed} sidebar=${sidebar} capture=${JSON.stringify(proof.capture)}`);
  if (!proof.capture.focusedInEditor) throw new Error(`Quick Capture opened with CodeMirror but the caret is not in it: ${JSON.stringify(proof.capture)}`);
  await waitForFileText(settingsFile, text => JSON.parse(text).experimental_cm6_editor === false);
  await restart();
  await openPageByName(browser, "Restart Demo");
  if (await kindAfterClick(".main-content", "alpha") !== "textarea") throw new Error("stored classic choice was lost after restart");
  proof.steps.push("explicit classic choice survives another process restart");
  console.log("PASS", JSON.stringify(proof));
} catch (error) {
  proof.observed = String(error);
  proof.classification = "product if the editor kind above is textarea; otherwise inspect DOM and driver log";
  if (browser) {
    try { await browser.saveScreenshot(path.join(TMP, "failure.png")); } catch {}
    fs.writeFileSync(path.join(TMP, "failure-dom.html"), await browser.execute(() => document.body.innerHTML).catch(() => ""));
  }
  console.error("FAIL", JSON.stringify(proof));
  process.exitCode = 1;
} finally {
  fs.writeFileSync(path.join(TMP, "proof.json"), `${JSON.stringify(proof, null, 2)}\n`);
  try { await browser?.deleteSession(); } catch {}
  for (const pid of myAppPids()) { try { process.kill(pid, "SIGKILL"); } catch { /* gone */ } }
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  fs.closeSync(log);
}
