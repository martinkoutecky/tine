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
const appData = `${TMP}/xdg/data/page.tine.Tine`;
fs.mkdirSync(appData, { recursive: true });
const settingsPath = `${appData}/tine-settings.json`;
// Capture is an independent WebView: begin with a persisted non-default mode
// so its cold start cannot accidentally inherit the main WebView's default.
fs.writeFileSync(settingsPath, '{"link_autocomplete_policy":"existing","space_after_ref_completion":true}\n');
fs.writeFileSync(`${GRAPH}/logseq/config.edn`, "{}\n");
fs.writeFileSync(`${GRAPH}/pages/Home.md`, "- capture focus fixture\n");
fs.writeFileSync(`${GRAPH}/pages/Fuzzy Existing.md`, "- completion target\n");
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
const waitForForwarderExit = (child, timeoutMs) => new Promise((resolve, reject) => {
  const timeout = setTimeout(() => reject(new Error(`secondary --capture process did not exit within ${timeoutMs}ms`)), timeoutMs);
  child.once("error", (error) => {
    clearTimeout(timeout);
    reject(error);
  });
  child.once("exit", (code, signal) => {
    clearTimeout(timeout);
    if (code === 0) resolve();
    else reject(new Error(`secondary --capture process exited with code=${code} signal=${signal}`));
  });
});

const driverLog = fs.openSync(`${ARTIFACT_DIR}/tauri-driver.log`, "w");
let td = spawn(
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
  // The single-instance callback runs in the primary while this short-lived
  // forwarding process still owns GTK/X11 resources. On slower hosted runners,
  // probing native focus during that teardown observes a destroyed transient
  // frame rather than the final user-visible state. Require the forwarder to
  // exit successfully, then prove that Quick Capture owns focus without clicks.
  await waitForForwarderExit(second, 5000);
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
  const activeAutocomplete = () => browser.execute(() => ({
    active: document.querySelector(".autocomplete .ac-item.active .ac-label")?.textContent?.trim() ?? "",
    labels: [...document.querySelectorAll(".autocomplete .ac-label")].map((node) => node.textContent?.trim() ?? ""),
    value: document.activeElement instanceof HTMLTextAreaElement ? document.activeElement.value : null,
  }));
  const expectActiveAutocomplete = async (expected, message) => {
    let last = null;
    const deadline = Date.now() + 10_000;
    while (Date.now() < deadline) {
      last = await activeAutocomplete();
      if (last.active === expected) return;
      await sleep(50);
    }
    // WebDriverIO interpolates timeoutMsg when waitUntil is created, so its
    // previous message always reported `last=null`. Keep the assertion equally
    // strict while exposing the actual capture popup/value for a policy failure.
    throw new Error(`${message}; last=${JSON.stringify(last)}`);
  };
  const expectAutocompleteValue = async (expected, message) => {
    let last = null;
    const deadline = Date.now() + 5_000;
    while (Date.now() < deadline) {
      last = await activeAutocomplete();
      if (last.value === expected) return;
      await sleep(50);
    }
    throw new Error(`${message}; expected=${JSON.stringify(expected)} actual=${JSON.stringify(last)}`);
  };
  const typePageQueryFromEmpty = async (query) => {
    // Keep every physical key in its own WebDriver command and observe the
    // editor between the repeated delimiters. WebKitGTK can otherwise deliver
    // a queued second `[` after the following text (`[Fz[`), which is an XTest
    // transport artefact rather than the user's literal ordering.
    await browser.keys(["["]);
    await expectAutocompleteValue("[", "Quick Capture did not receive the first page-ref delimiter");
    await browser.keys(["["]);
    await expectAutocompleteValue("[[]]", "Quick Capture did not auto-pair the second page-ref delimiter");
    for (const key of query) await browser.keys([key]);
  };
  // A fuzzy-only query distinguishes existing-first from adaptive/Create-first.
  // Exercise both page and tag acceptance in the cold, independently initialized
  // capture WebView using literal keys.
  await typePageQueryFromEmpty("Fz");
  await expectActiveAutocomplete("Fuzzy Existing", "cold Quick Capture ignored persisted existing-first page policy");
  await browser.keys(["Enter"]);
  // Existing page acceptance keeps Tine's configured GH #35 continuation space.
  // Do not type another separator before the following tag query: that would
  // conceal a doubled-space regression instead of proving the actual edit.
  await expectAutocompleteValue("[[Fuzzy Existing]] ", "cold Quick Capture did not accept the existing page with its configured continuation space");
  for (const key of "#Fz") await browser.keys([key]);
  await expectActiveAutocomplete("#Fuzzy Existing", "cold Quick Capture ignored persisted existing-first tag policy");
  await browser.keys(["Enter"]);
  await expectAutocompleteValue("[[Fuzzy Existing]] #[[Fuzzy Existing]] ", "cold Quick Capture did not accept the existing multiword tag with its configured continuation space");
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
  // Change the policy through the main Settings UI while the persistent capture
  // window is hidden. Reopening that exact WebView must refresh before Enter can
  // choose a completion; otherwise it would retain the cold existing-first signal.
  await browser.switchToWindow(mainHandle);
  await browser.$('button[title^="Settings"]').click();
  // The settings launcher opens its default tab, while this policy deliberately
  // lives under Editor → Advanced. Follow that visible flow rather than reaching
  // into the collapsed setting or mutating its persisted value directly.
  const settingsState = () => browser.execute(() => ({
    modalOpen: Boolean(document.querySelector(".settings-modal")),
    tabs: [...document.querySelectorAll(".settings-nav-item")].map((tab) => ({
      label: tab.textContent?.trim() ?? "",
      active: tab.classList.contains("active"),
    })),
    advanced: [...document.querySelectorAll(".settings-advanced-toggle")].map((toggle) => ({
      label: toggle.textContent?.trim() ?? "",
      expanded: toggle.getAttribute("aria-expanded"),
    })),
    policyVisible: Boolean(document.querySelector('select[aria-label="Link autocomplete default"]')),
  }));
  const waitForSettingsState = async (matches, message) => {
    let last = null;
    const deadline = Date.now() + 5_000;
    while (Date.now() < deadline) {
      last = await settingsState();
      if (matches(last)) return last;
      await sleep(50);
    }
    throw new Error(`${message}; settings=${JSON.stringify(last)}`);
  };
  const openedSettings = await waitForSettingsState((state) => state.modalOpen, "Settings UI did not open");
  if (!openedSettings.tabs.some((tab) => tab.label === "Editor")) {
    throw new Error(`Settings UI lacks its Editor tab; settings=${JSON.stringify(openedSettings)}`);
  }
  const editorTab = await browser.$("//button[contains(concat(' ', normalize-space(@class), ' '), ' settings-nav-item ') and normalize-space(.)='Editor']");
  await editorTab.click();
  let currentSettings = await waitForSettingsState(
    (state) => state.tabs.some((tab) => tab.label === "Editor" && tab.active),
    "Settings UI did not activate the Editor tab",
  );
  if (!currentSettings.advanced.some((section) => section.expanded === "true")) {
    if (!currentSettings.advanced.length) {
      throw new Error(`Editor Settings lacks an Advanced disclosure; settings=${JSON.stringify(currentSettings)}`);
    }
    const advanced = await browser.$(".settings-advanced-toggle");
    await advanced.click();
    currentSettings = await waitForSettingsState(
      (state) => state.advanced.some((section) => section.expanded === "true"),
      "Settings UI did not reveal Editor Advanced",
    );
  }
  await waitForSettingsState((state) => state.policyVisible, "Editor Advanced did not reveal Link autocomplete default");
  const policy = await browser.$('select[aria-label="Link autocomplete default"]');
  await policy.selectByAttribute("value", "typed");
  await browser.waitUntil(() => fs.existsSync(settingsPath) && fs.readFileSync(settingsPath, "utf8").includes('"link_autocomplete_policy": "typed"'), {
    timeout: 5_000, timeoutMsg: "Settings UI did not persist typed completion policy",
  });
  await browser.keys(["Escape"]);
  const reopen = spawn(APP, ["--capture"], { env, stdio: ["ignore", driverLog, driverLog], detached: true });
  await waitForForwarderExit(reopen, 5_000);
  reopen.unref();
  await waitForWindow("Quick Capture", 10_000);
  await browser.switchToWindow(captureHandle);
  await browser.keys(["Control", "a"]);
  await browser.keys(["Backspace"]);
  await typePageQueryFromEmpty("Fz");
  await expectActiveAutocomplete('Create "Fz"', "reopened Quick Capture retained the hidden existing-first policy");
  await browser.keys(["Escape"]);
  // Repeat from a fresh process with the same XDG home. This is distinct from
  // reopening the hidden window above: it proves persisted policy initialization
  // before a cold Capture WebView can accept a page completion.
  await browser.deleteSession();
  browser = undefined;
  try { process.kill(-td.pid, "SIGKILL"); } catch {}
  await sleep(700);
  td = spawn(
    TD,
    ["--port", String(DRIVER_PORT), "--native-port", String(NATIVE_PORT), "--native-driver", WD],
    { env, stdio: ["ignore", driverLog, driverLog], detached: true },
  );
  await sleep(2_500);
  browser = await remote({
    hostname: "127.0.0.1", port: DRIVER_PORT, path: "/",
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } },
    logLevel: "error", connectionRetryCount: 1, connectionRetryTimeout: 60_000,
  });
  const restartedHandles = await browser.getWindowHandles();
  const restartedCapture = await (async () => {
    for (const handle of restartedHandles) {
      await browser.switchToWindow(handle);
      if (matchesWindowName(await browser.getTitle(), "Quick Capture")) return handle;
    }
    return null;
  })();
  if (!restartedCapture) throw new Error("fresh process lacked the pre-created Quick Capture WebView");
  const coldReopen = spawn(APP, ["--capture"], { env, stdio: ["ignore", driverLog, driverLog], detached: true });
  await waitForForwarderExit(coldReopen, 5_000);
  coldReopen.unref();
  await waitForWindow("Quick Capture", 10_000);
  await browser.switchToWindow(restartedCapture);
  await typePageQueryFromEmpty("Fz");
  await expectActiveAutocomplete('Create "Fz"', "cold restarted Quick Capture did not initialize persisted typed policy");
  await browser.keys(["Escape"]);
  console.log(`PASS: cold Quick Capture exposed a focused bullet and saved keyboard input without a click; dom=${JSON.stringify(domFocus)} frame=${JSON.stringify({ lightShadow, darkShadow })}`);
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-td.pid, "SIGKILL"); } catch {}
  try { if (wm) process.kill(-wm.pid, "SIGKILL"); } catch {}
  fs.closeSync(driverLog);
  fs.closeSync(wmLog);
}
