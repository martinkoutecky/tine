// Native proof for UI-OG-L9-18-IME-BLUR-ABANDONED, in the real webview (WebKitGTK).
// Run E2E_EDITOR=cm6 or E2E_EDITOR=textarea (the Settings choice between the two editors).
//
// A real IME cannot be driven on Xvfb. This journey is synthetic: it sends the
// events an IME sends (compositionstart, a composing beforeinput, the preedit
// text in the editing surface, compositionupdate, a composing input) through
// WebDriver, then moves focus away for real and never sends compositionend,
// which is the reported shape (an IME the platform cancels on blur). What the
// webview then does with focus, DOM reconciliation and the editor's teardown is
// real; the IME is not.
//
// Two ways to lose focus, each on its own routed page and each followed by
// ordinary typing with WebDriver key actions:
//   click  - a real pointer click on a part of the window that is not a block.
//   window - another application's window takes native X11 focus (openbox
//            activates an xmessage window), so the editor's document loses focus
//            while the editor stays mounted in edit mode; the editor's own window
//            then gets focus back and the journey clicks into the editor.
// The assertions are on the saved file: the composing text lands once, and the
// text typed afterwards lands after it.
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import net from "node:net";
import crypto from "node:crypto";
import { spawn, execFileSync } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { remote } from "webdriverio";
import { chooseEditor } from "./lib/e2e-editor-choice.mjs";
import { accountHomeFixture, assertAppLease } from "./lib/e2e-account-home.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { ensurePrivateSessionBus } from "./lib/e2e-session-bus.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";
import { startWebdriverApplication, stopWebdriverApplication, tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";

// Direct runs re-exec on a private session bus (the single-instance transport); the runner supplies one.
ensurePrivateSessionBus();

const root = path.resolve(import.meta.dirname, "..");
const mode = process.env.E2E_EDITOR || "cm6";
assert.ok(["cm6", "textarea"].includes(mode), `Unknown E2E_EDITOR ${mode}`);
assert.equal(process.platform, "linux", "This journey drives native X11 focus; Linux only");
const artifacts = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(root, "artifacts/ime-blur"));
fs.mkdirSync(artifacts, { recursive: true });
const out = fs.mkdtempSync(path.join(artifacts, `${mode}-`));
const graph = path.join(out, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["home", "data", "config", "cache", "state"]) fs.mkdirSync(path.join(out, dir));
const file = page => path.join(graph, "pages", `${page}.md`);
// The announcement toast covers the window's lower right; it is not this journey's subject.
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{:preferred-workflow :todo :tine/guide-announced? true}\n");
const PAGES = { click: "Ime Click", window: "Ime Window" };
const START = { click: "compose click", window: "compose window" };
for (const [how, page] of Object.entries(PAGES)) fs.writeFileSync(file(page), `- ${START[how]}\n- bystander ${how}\n`);
const app = process.env.TINE_APP || path.join(root, "target/release/tine");
const fixture = accountHomeFixture(out, out, { ...process.env,
  HOME: path.join(out, "home"), TINE_GRAPH: graph,
  XDG_DATA_HOME: path.join(out, "data"), XDG_CONFIG_HOME: path.join(out, "config"),
  XDG_CACHE_HOME: path.join(out, "cache"), XDG_STATE_HOME: path.join(out, "state"),
  XDG_CONFIG_DIRS: process.env.XDG_CONFIG_DIRS || "/etc/xdg",
  XDG_DATA_DIRS: process.env.XDG_DATA_DIRS || "/usr/local/share:/usr/share",
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11",
});
async function freePort() {
  const server = net.createServer();
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  const port = server.address().port;
  await new Promise(resolve => server.close(resolve));
  return port;
}
const port = Number(process.env.E2E_DRIVER_PORT) || await freePort();
const nativePort = Number(process.env.E2E_NATIVE_PORT) || await freePort();
const log = fs.openSync(path.join(out, "driver.log"), "w");
const proof = { base: execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim(),
  app, sha256: crypto.createHash("sha256").update(fs.readFileSync(app)).digest("hex"), mode,
  composition: "Synthetic compositionstart/beforeinput/update/input events with preedit text (a Telex-style preedit), no compositionend; focus loss is real; no real OS IME",
  outcomes: [] };
let browser, driver, webview, wm, other, mainWindowId, stage = "startup", expected;

// Native X11 focus, as e2e-capture drives it.
const xdo = (...args) => execFileSync(process.env.E2E_XDOTOOL || "xdotool", args, { encoding: "utf8",
  env: process.env.E2E_XDOTOOL_LIB ? { ...fixture.env, LD_LIBRARY_PATH: process.env.E2E_XDOTOOL_LIB } : fixture.env }).trim();
const activeWindow = () => { try { const id = xdo("getactivewindow"); return { id, name: xdo("getwindowname", id) }; } catch { return { id: "", name: "" }; } };
async function waitActive(match, what, timeoutMs = 10_000) {
  const deadline = Date.now() + timeoutMs;
  let state = activeWindow();
  while (Date.now() < deadline) {
    state = activeWindow();
    if (match(state)) return state;
    await sleep(50);
  }
  throw new Error(`${what}; active window was ${JSON.stringify(state)}`);
}
const isMain = state => state.name === "Tine" || state.name.startsWith("Tine —");

const editorSelector = mode === "cm6" ? '.main-content [data-block-editor="cm6"] .cm-content' : ".main-content textarea.block-editor";
const editor = () => browser.$(editorSelector);
async function screenshot(name) {
  // WebKitGTK's WebDriver screenshot can stall with compositing disabled: capture the X11 screen.
  execFileSync("import", ["-window", "root", path.join(out, name)], { env: fixture.env, timeout: 10000 });
}
async function keys(...value) { await browser.keys(value); await browser.releaseActions(); }
const value = () => browser.execute(selector => {
  const el = document.querySelector(selector);
  return el instanceof HTMLTextAreaElement ? el.value
    : el ? [...el.querySelectorAll(".cm-line")].map(line => line.textContent).join("\n") : null;
}, editorSelector);
async function type(text) {
  // Observe each delivered character before sending the next one.
  for (const char of text) {
    const before = await value();
    await browser.keys(char);
    await browser.waitUntil(async () => await value() !== before, { timeout: 5000, timeoutMsg: `Input ${JSON.stringify(char)} was lost` });
  }
}
async function saved(page, bytes) {
  expected = { page, bytes };
  assert.equal(await waitForFileText(file(page), text => text === bytes), bytes);
  proof.outcomes.push({ stage, page, bytes });
}
async function clickBlock(text) {
  const block = await browser.$(`//div[contains(concat(' ', normalize-space(@class), ' '), ' block-content ') and contains(., '${text}') and ancestor::*[contains(concat(' ', normalize-space(@class), ' '), ' main-content ')]]`);
  await block.waitForDisplayed();
  await block.click();
  await editor().waitForDisplayed();
}
async function openBlock(page, text) {
  await openPageByName(browser, page, { entry: "button" });
  await clickBlock(text);
}
/** The events an IME sends while it holds preedit text, then the preedit in the surface. No compositionend. */
async function startPreedit(preedit) {
  const inserted = await browser.execute((selector, text) => {
    const el = document.querySelector(selector);
    el.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true, data: "" }));
    el.dispatchEvent(new InputEvent("beforeinput", { bubbles: true, inputType: "insertCompositionText", data: text, isComposing: true }));
    const done = document.execCommand("insertText", false, text);
    el.dispatchEvent(new CompositionEvent("compositionupdate", { bubbles: true, data: text }));
    el.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertCompositionText", data: text, isComposing: true }));
    return done;
  }, editorSelector, preedit);
  assert.equal(inserted, true, "Browser did not support composition DOM insertion");
}
const kind = () => browser.execute(selector => {
  const el = document.querySelector(selector);
  return { editor: el instanceof HTMLTextAreaElement ? "textarea" : el ? "cm6" : "none", hasFocus: document.hasFocus(),
    activeIsEditor: !!el && (document.activeElement === el || el.contains(document.activeElement)) };
}, editorSelector);
async function clickAway() {
  // A real pointer click on a part of the window that is neither a block nor a control: the empty
  // part of the left sidebar below its page list.
  const point = await browser.execute(() => {
    const interactive = "button, a, input, textarea, select, [contenteditable], [data-block-id], .block-content, [data-tauri-drag-region]";
    const side = document.querySelector(".left-sidebar-scroll, .left-sidebar");
    if (!side) return null;
    const box = side.getBoundingClientRect();
    const x = Math.round(box.left + box.width / 2);
    for (let y = Math.round(box.top + box.height * 0.75); y > box.top; y -= 20) {
      const hit = document.elementFromPoint(x, y);
      if (hit && side.contains(hit) && !hit.closest(interactive)) return { x, y, hit: hit.className || hit.tagName };
    }
    return null;
  });
  assert.ok(point, "found no inert part of the left sidebar to click");
  await browser.performActions([{ type: "pointer", id: "mouse", parameters: { pointerType: "mouse" }, actions: [
    { type: "pointerMove", duration: 0, origin: "viewport", x: point.x, y: point.y },
    { type: "pointerDown", button: 0 }, { type: "pointerUp", button: 0 },
  ] }]);
  await browser.releaseActions();
  return point;
}

// Latin script with precomposed letters, as a Vietnamese Telex IME composes it: the host has no CJK font,
// so a CJK preedit would be unreadable in the screenshots.
const PREEDIT = "việt";
async function journey(how) {
  const page = PAGES[how], start = START[how];
  stage = `${mode}/${how}: start a composition on ${page}`;
  await openBlock(page, start);
  await keys("Control", "End"); await type(" ");
  await startPreedit(PREEDIT);
  assert.equal(await value(), `${start} ${PREEDIT}`, "the preedit is in the editing surface");
  await screenshot(`${how}-1-preedit.png`);

  stage = `${mode}/${how}: focus leaves with no compositionend`;
  if (how === "click") {
    const point = await clickAway();
    proof.outcomes.push({ stage, clickedAt: point });
  } else {
    other = spawn("xmessage", ["-title", "ime-blur-other", "-timeout", "120", "Another application took focus"],
      { env: fixture.env, stdio: "ignore", detached: true });
    await waitActive(state => state.name === "ime-blur-other", "the other application's window never took native focus");
    await browser.waitUntil(async () => !(await kind()).hasFocus, { timeout: 5000, timeoutMsg: "the document still reports focus" });
    // The editor stays mounted and in edit mode: the user only switched applications.
    assert.equal((await kind()).editor, mode, "the editor stays mounted while the window is not focused");
  }
  await sleep(300);
  await screenshot(`${how}-2-abandoned.png`);
  // Whatever the blur did with the composing text, it is in the file once.
  stage = `${mode}/${how}: the composing text was kept`;
  await saved(page, `- ${start} ${PREEDIT}\n- bystander ${how}\n`);

  stage = `${mode}/${how}: focus comes back and ordinary typing lands`;
  if (how === "window") {
    other.kill("SIGKILL"); other = undefined;
    xdo("windowactivate", mainWindowId);
    await waitActive(isMain, "the application's window never got native focus back");
    await browser.waitUntil(async () => (await kind()).hasFocus, { timeout: 5000, timeoutMsg: "the document did not get focus back" });
    await editor().waitForDisplayed();
    await editor().click(); // a real click back into the editor
  } else {
    await clickBlock(`${start} ${PREEDIT}`);
  }
  await keys("Control", "End");
  await type(" abc");
  await screenshot(`${how}-3-typed.png`);
  await saved(page, `- ${start} ${PREEDIT} abc\n- bystander ${how}\n`);
  assert.equal((fs.readFileSync(file(page), "utf8").match(new RegExp(PREEDIT, "g")) ?? []).length, 1, "the composing text is not duplicated");
  await keys("Escape");
  await browser.waitUntil(async () => !(await editor().isExisting()) || (await kind()).editor === "cm6", { timeout: 5000 });
  await saved(page, `- ${start} ${PREEDIT} abc\n- bystander ${how}\n`);
}

try {
  if (process.env.E2E_WINDOW_MANAGER) {
    const wmLog = fs.openSync(path.join(out, "window-manager.log"), "w");
    wm = spawn(process.env.E2E_WINDOW_MANAGER, ["--sm-disable"], { env: fixture.env, stdio: ["ignore", wmLog, wmLog], detached: true });
    await sleep(600);
    assert.equal(wm.exitCode, null, "the window manager exited before the app launched");
  } else throw new Error("E2E_WINDOW_MANAGER (openbox) is required: native focus moves need a window manager");
  webview = await startWebdriverApplication(app, fixture.env, nativePort, mode);
  driver = spawn(process.env.TAURI_DRIVER || "tauri-driver",
    webdriverServerArgs(port, nativePort, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"),
    { env: webview.env, stdio: ["ignore", log, log], detached: true });
  browser = await remote({
    hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 3, connectionRetryTimeout: 30000,
    capabilities: tauriCapabilities(app, mode, process.platform, webview.debuggerAddress),
  });
  await ensureMainWindow(browser);
  await browser.$(".app-container").waitForExist({ timeout: 30000 });
  await assertAppLease(fixture);
  mainWindowId = (await waitActive(isMain, "the application's window never took native focus at launch", 20_000)).id;
  stage = "Choose editor through Settings";
  await chooseEditor(browser, mode);

  await journey("click");
  await journey("window");
  fixture.verify();
  stage = "Capture native evidence";
  await screenshot("complete.png");
  proof.outcome = "PASS";
} catch (error) {
  proof.outcome = "FAIL"; proof.stage = stage; proof.expected = `Complete ${stage}`; proof.expectedFile = expected;
  proof.observed = String(error.stack);
  proof.classification = "ambiguous; inspect native screenshot, DOM and driver log";
  try { await screenshot("failure.png"); } catch {}
  try { fs.writeFileSync(path.join(out, "failure.html"), await browser.getPageSource()); } catch {}
  process.exitCode = 1;
} finally {
  fs.writeFileSync(path.join(out, "proof.json"), JSON.stringify(proof, null, 2) + "\n");
  console.log(`${proof.outcome}: ${stage}: ${out}`);
  try { await browser?.deleteSession(); } catch {}
  if (driver?.pid) { try { process.kill(-driver.pid, "SIGKILL"); } catch {} }
  if (other?.pid) { try { other.kill("SIGKILL"); } catch {} }
  if (wm?.pid) { try { process.kill(-wm.pid, "SIGKILL"); } catch {} }
  stopWebdriverApplication(webview);
  fixture.close?.(); fs.closeSync(log);
}
