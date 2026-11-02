// CodeMirror editor setting across an app restart, in the real app (development build only).
// The user turns the experiment on in Settings, quits, relaunches, and the first block
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

const env = { ...process.env, HOME: path.join(TMP, "home"), TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: path.join(TMP, "data"), XDG_CONFIG_HOME: path.join(TMP, "config"), XDG_CACHE_HOME: path.join(TMP, "cache"),
  XDG_STATE_HOME: path.join(TMP, "state"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4897);
const log = fs.openSync(path.join(TMP, "driver.log"), "w");
const driver = spawn(DRIVER, ["--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log], detached: true });
let browser;
const proof = { commit: execFileSync("git", ["rev-parse", "HEAD"], { cwd: ROOT, encoding: "utf8" }).trim(), binarySha256: createHash("sha256").update(fs.readFileSync(APP)).digest("hex"), journey: "CodeMirror editor setting survives an app restart", expected: "after relaunch the first block edit mounts the CodeMirror editor", steps: [] };

async function launch() {
  const session = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 2, connectionRetryTimeout: 30_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } } });
  await session.$(".app-container").waitForExist({ timeout: 30_000 });
  await session.execute(() => { for (const close of document.querySelectorAll(".toast-sticky .toast-close")) close.click(); });
  return session;
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
  // 1. Turn the experiment on in Settings (search reveals it inside the collapsed Advanced section).
  await browser.$('button[title^="Settings"]').click();
  await browser.$(".settings-modal").waitForDisplayed({ timeout: 10_000 });
  await browser.$(".settings-search-input").setValue("CodeMirror");
  const result = await browser.$(".settings-search-result");
  await result.waitForDisplayed({ timeout: 10_000 });
  await result.click();
  const toggle = await browser.$('[data-setting-label="Experimental: CodeMirror editor"] [role="switch"]');
  await toggle.waitForDisplayed({ timeout: 10_000 });
  if (await toggle.getAttribute("aria-checked") !== "false") throw new Error("CodeMirror was not off on a fresh profile");
  await toggle.click();
  await browser.waitUntil(async () => (await toggle.getAttribute("aria-checked")) === "true", { timeout: 5000 });
  proof.steps.push("Settings: CodeMirror switched on");
  await browser.keys(["Escape"]);
  // Persisted before the restart: the device preference file holds the choice.
  const settingsFile = path.join(TMP, "data", APP_ID, "tine-settings.json");
  for (let i = 0; i < 50 && !(fs.existsSync(settingsFile) && /experimental_cm6_editor"\s*:\s*true/.test(fs.readFileSync(settingsFile, "utf8"))); i += 1) await new Promise((r) => setTimeout(r, 100));
  if (!/experimental_cm6_editor"\s*:\s*true/.test(fs.readFileSync(settingsFile, "utf8"))) throw new Error(`setting not persisted in ${settingsFile}`);
  proof.steps.push("tine-settings.json holds experimental_cm6_editor=true");

  // 2. Quit and relaunch with the same profile.
  await browser.deleteSession();
  browser = await launch();
  proof.steps.push("relaunched");

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
  if (routed !== "cm6" || sidebar !== "cm6" || !proof.capture?.cm6) throw new Error(`stored CodeMirror choice ignored after restart: routed=${routed} sidebar=${sidebar} capture=${JSON.stringify(proof.capture)}`);
  if (!proof.capture.focusedInEditor) throw new Error(`Quick Capture opened with CodeMirror but the caret is not in it: ${JSON.stringify(proof.capture)}`);
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
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  fs.closeSync(log);
}
