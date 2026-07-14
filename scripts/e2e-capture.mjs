// Native Quick Capture activation regression. The second `tine --capture`
// process exercises the same single-instance handoff as a desktop global
// shortcut. xdotool proves the native X11 focus without clicking; WebDriver then
// inspects the real WebKit document and sends keys to its already-active element.
// The saved journal proves that input landed in the capture editor.
import { execFileSync, spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const XDOTOOL = process.env.E2E_XDOTOOL || "xdotool";
const TD = process.env.TAURI_DRIVER || "tauri-driver";
const WD = process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver";
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4456);
const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4457);
const TMP = "/tmp/tine-capture-e2e";
const GRAPH = `${TMP}/graph`;
// Avoid adjacent duplicate characters: WebKitWebDriver coalesces identical
// synthetic key-downs on some Linux versions even though a physical keyboard
// supplies the missing key-up transition.
const PROOF = "capture-focus-ok";
const ARTIFACT_DIR = process.env.E2E_ARTIFACT_DIR || TMP;

fs.rmSync(TMP, { recursive: true, force: true });
for (const dir of ["pages", "journals", "logseq", "assets"]) fs.mkdirSync(`${GRAPH}/${dir}`, { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(`${TMP}/xdg/${dir}`, { recursive: true });
fs.writeFileSync(`${GRAPH}/logseq/config.edn`, "{}\n");
fs.writeFileSync(`${GRAPH}/pages/Home.md`, "- capture focus fixture\n");
const now = new Date();
const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
const journalPath = `${GRAPH}/journals/${journal}.md`;
fs.writeFileSync(journalPath, "- open [[Home]]\n");

const env = {
  ...process.env,
  TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: `${TMP}/xdg/data`,
  XDG_CONFIG_HOME: `${TMP}/xdg/config`,
  XDG_CACHE_HOME: `${TMP}/xdg/cache`,
  // GitHub runners may set host-specific XDG search paths. Openbox must still
  // find its system config and theme after the test isolates the user's XDG
  // homes, otherwise it starts without EWMH activation support.
  XDG_CONFIG_DIRS: process.env.XDG_CONFIG_DIRS || "/etc/xdg",
  XDG_DATA_DIRS: process.env.XDG_DATA_DIRS || "/usr/local/share:/usr/share",
  WEBKIT_DISABLE_DMABUF_RENDERER: "1",
  WEBKIT_DISABLE_COMPOSITING_MODE: "1",
  LIBGL_ALWAYS_SOFTWARE: "1",
  GDK_BACKEND: "x11",
};
fs.mkdirSync(ARTIFACT_DIR, { recursive: true });
const wmLogPath = `${ARTIFACT_DIR}/window-manager.log`;
const wmLog = fs.openSync(wmLogPath, "w");
const wm = process.env.E2E_WINDOW_MANAGER
  ? spawn(process.env.E2E_WINDOW_MANAGER, ["--sm-disable"], { env, stdio: ["ignore", wmLog, wmLog], detached: true })
  : null;
if (wm) await sleep(600);
if (wm?.exitCode != null) {
  fs.closeSync(wmLog);
  throw new Error(`window manager exited before the app launched: ${fs.readFileSync(wmLogPath, "utf8")}`);
}
const xdo = (...args) => execFileSync(XDOTOOL, args, {
  encoding: "utf8",
  env: process.env.E2E_XDOTOOL_LIB
    ? { ...env, LD_LIBRARY_PATH: process.env.E2E_XDOTOOL_LIB }
    : env,
}).trim();
const allowFocusWhenEwmhIsUnavailable = process.env.E2E_ALLOW_SYNTHETIC_FOCUS === "1";
const describeWindow = (selector) => {
  try {
    const id = xdo(selector);
    return { id, name: xdo("getwindowname", id), error: "" };
  } catch (error) {
    return {
      id: "",
      name: "",
      error: error?.stderr?.toString().trim() || error?.message || String(error),
    };
  }
};
const matchesWindowName = (actual, wanted) => actual === wanted || actual.startsWith(`${wanted} —`);
const namedWindows = (wanted) => {
  try {
    return xdo("search", "--name", wanted).split(/\s+/).filter(Boolean).map((id) => ({
      id,
      name: xdo("getwindowname", id),
    }));
  } catch (error) {
    return [{ error: error?.stderr?.toString().trim() || error?.message || String(error) }];
  }
};
const windowState = () => ({
  active: describeWindow("getactivewindow"),
  focus: describeWindow("getwindowfocus"),
});
const waitForWindow = async (wanted, timeoutMs) => {
  const deadline = Date.now() + timeoutMs;
  let state = windowState();
  while (Date.now() < deadline) {
    state = windowState();
    if (matchesWindowName(state.active.name, wanted)) return state;
    // Openbox under GitHub's nested Xvfb occasionally leaves
    // _NET_ACTIVE_WINDOW pointing at a just-destroyed frame even though the
    // real X input focus has already moved to Quick Capture. The hosted gate
    // explicitly allows that EWMH-only defect, but still requires the target
    // window to own actual keyboard focus before injecting any input.
    if (allowFocusWhenEwmhIsUnavailable && state.active.error && matchesWindowName(state.focus.name, wanted)) {
      return state;
    }
    await sleep(50);
  }
  throw new Error(`${wanted} never received native focus; state=${JSON.stringify(state)} windows=${JSON.stringify(namedWindows(wanted))}`);
};

const driverLog = fs.openSync(`${ARTIFACT_DIR}/tauri-driver.log`, "w");
const td = spawn(
  TD,
  ["--port", String(DRIVER_PORT), "--native-port", String(NATIVE_PORT), "--native-driver", WD],
  { env, stdio: ["ignore", driverLog, driverLog], detached: true },
);
await sleep(2500);

let browser;
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
    connectionRetryTimeout: 60_000,
  });
  await waitForWindow("Tine", 20_000);
  const handles = await browser.getWindowHandles();
  if (handles.length !== 2) throw new Error(`expected main + hidden capture WebViews, got ${handles.length}`);
  let mainHandle;
  let captureHandle;
  const webviews = [];
  for (const handle of handles) {
    await browser.switchToWindow(handle);
    const title = await browser.getTitle();
    webviews.push({ handle, title, url: await browser.getUrl() });
    if (matchesWindowName(title, "Quick Capture")) captureHandle = handle;
    else mainHandle = handle;
  }
  if (!mainHandle || !captureHandle) {
    throw new Error(`could not identify Tine WebViews: ${JSON.stringify({ webviews, mainHandle, captureHandle })}`);
  }
  await browser.switchToWindow(mainHandle);
  // This scenario exercises the normal global-shortcut path: hand off to an
  // app that is already running. GitHub's cold WebKit/portal startup can expose
  // a titled main window before its native surfaces have settled; launching the
  // second process during that unrelated cold-start race can leave Openbox
  // focused on its root window. Require one stable turn before the handoff.
  await sleep(1500);
  await waitForWindow("Tine", 5000);

  const second = spawn(APP, ["--capture"], { env, stdio: ["ignore", driverLog, driverLog], detached: true });
  second.unref();
  await waitForWindow("Quick Capture", 10_000);

  // Model the short interval between seeing the newly painted window and a
  // human's first keystroke, while still proving that focus remains native.
  await sleep(300);
  const quickCapture = await waitForWindow("Quick Capture", 100);
  const afterHandoffHandles = await browser.getWindowHandles();
  const captureWindows = namedWindows("Quick Capture");
  if (afterHandoffHandles.length !== 2 || captureWindows.length !== 1) {
    throw new Error(`capture handoff created duplicate windows: ${JSON.stringify({ afterHandoffHandles, captureWindows })}`);
  }
  if (!matchesWindowName(quickCapture.focus.name, "Quick Capture")) {
    throw new Error(`Quick Capture lacked X input focus; state=${JSON.stringify(quickCapture)}`);
  }
  if (quickCapture.active.name && quickCapture.active.id !== quickCapture.focus.id) {
    throw new Error(`Quick Capture was EWMH-active but lacked X input focus; state=${JSON.stringify(quickCapture)}`);
  }
  // Switching WebDriver's browsing context does not click or focus a native
  // surface. Verify that it did not disturb the X focus established above,
  // then inspect the WebKit document before sending any input. This separates
  // an actual application focus defect from Xvfb/XTest refusing synthetic text
  // in a never-clicked WebKitGTK child.
  await browser.switchToWindow(captureHandle);
  const afterContextSwitch = windowState();
  if (!matchesWindowName(afterContextSwitch.focus.name, "Quick Capture")) {
    throw new Error(`WebDriver context inspection disturbed native focus: ${JSON.stringify(afterContextSwitch)}`);
  }
  const readDomFocus = () => browser.execute(() => {
    const editor = document.querySelector(".capture-shell textarea");
    return {
      documentHasFocus: document.hasFocus(),
      editorExists: Boolean(editor),
      editorIsActive: document.activeElement === editor,
      activeTag: document.activeElement?.tagName ?? "",
      activeClass: document.activeElement?.getAttribute("class") ?? "",
      textareas: document.querySelectorAll("textarea").length,
      shellExists: Boolean(document.querySelector(".capture-shell")),
    };
  });
  const domFocus = await readDomFocus();
  if (!domFocus.documentHasFocus || !domFocus.editorExists || !domFocus.editorIsActive) {
    throw new Error(`Quick Capture bullet lacked first-show DOM focus: ${JSON.stringify({ webviews, captureWindows, domFocus })}`);
  }
  // Let WebKit finish its first tile paint before capturing visual evidence;
  // the focus assertion above intentionally happens before this cosmetic wait.
  await sleep(150);
  const settledDomFocus = await readDomFocus();
  if (!settledDomFocus.documentHasFocus || !settledDomFocus.editorIsActive) {
    throw new Error(`Quick Capture bullet did not retain focus after first paint: ${JSON.stringify(settledDomFocus)}`);
  }
  // Send keyboard input to the already-active element without clicking or
  // calling focus(). Xvfb's XTest transport is rejected by a never-clicked
  // WebKitGTK child on some runners, so WebDriver supplies the keystrokes only
  // after the independent X11 + DOM assertions above have proved application
  // focus. Saving the marker proves the focused editor is live, not decorative.
  await browser.keys([...PROOF]);
  await sleep(150);
  await browser.saveScreenshot(path.join(ARTIFACT_DIR, "quick-capture-typed.png"));
  const lightShadow = await browser.execute(() => getComputedStyle(document.body).boxShadow);
  await browser.execute(() => document.documentElement.setAttribute("data-theme", "dark"));
  await sleep(100);
  const darkShadow = await browser.execute(() => getComputedStyle(document.body).boxShadow);
  if (lightShadow === "none" || darkShadow === "none" || lightShadow === darkShadow) {
    throw new Error(`Quick Capture frame was not theme-aware: ${JSON.stringify({ lightShadow, darkShadow })}`);
  }
  await browser.saveScreenshot(path.join(ARTIFACT_DIR, "quick-capture-dark.png"));
  await browser.keys(["Control", "Shift", "Enter"]);

  const deadline = Date.now() + 10_000;
  while (Date.now() < deadline && !fs.readFileSync(journalPath, "utf8").includes(PROOF)) await sleep(100);
  if (!fs.readFileSync(journalPath, "utf8").includes(PROOF)) {
    throw new Error(`keyboard input did not reach the graph despite proven native + DOM focus; dom=${JSON.stringify(domFocus)} journal=${JSON.stringify(fs.readFileSync(journalPath, "utf8"))}`);
  }
  console.log(`PASS: cold Quick Capture exposed a focused bullet and saved keyboard input without a click; dom=${JSON.stringify(domFocus)} frame=${JSON.stringify({ lightShadow, darkShadow })}`);
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-td.pid, "SIGKILL"); } catch {}
  try { if (wm) process.kill(-wm.pid, "SIGKILL"); } catch {}
  fs.closeSync(driverLog);
  fs.closeSync(wmLog);
}
