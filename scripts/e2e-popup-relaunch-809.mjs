// GH #809: "Open in a new window" on a profile that has opened one before.
// Launch 1 opens a workspace popup and quits cleanly, so the window-state
// plugin writes its file. That file must name no workspace popup (`ws-*`).
// A synthetic saved geometry for the first popup labels is then added, the
// state a reporter's profile can hold, and launch 2 on the same app data opens
// popups twice: session restore reopens launch 1's popup at startup, then
// Ctrl+K opens another. Each must render, its native build must return, and
// main must stay responsive. Before the fix, on Windows, the plugin restored
// that geometry from inside Tauri's window-created hook and the main thread
// deadlocked in `build()`. GH809_ROOT_CAUSE=1 lets a pre-fix build continue
// past the saved-state check to that launch.
// Fixture data only. A Linux pass does not establish the Windows result
// (GTK delivers the restore's move later, so Linux never deadlocked).
import { execFileSync, spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { setTimeout as sleep } from "node:timers/promises";
import { remote } from "webdriverio";
import { accountHomeFixture, assertAppLease } from "./lib/e2e-account-home.mjs";
import { APP_ID } from "./lib/app-identity.mjs";
import { ensureMainWindow, isAuxiliaryWindow } from "./lib/e2e-main-window.mjs";
import { x11Tools } from "./lib/e2e-x11.mjs";
import { startWebdriverApplication, stopWebdriverApplication, tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";

if (!["linux", "win32"].includes(process.platform)) throw new Error("This journey supports Linux and Windows");
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const out = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(root, "target/artifacts/popup-relaunch-809", `native-${process.pid}`));
const run = path.join(out, `fixture-${process.pid}`);
const graph = path.join(run, "synthetic-graph");
const app = path.resolve(process.env.TINE_APP || path.join(root, "target/debug", process.platform === "win32" ? "tine.exe" : "tine"));
const port = Number(process.env.E2E_DRIVER_PORT || 4692);
const nativePort = Number(process.env.E2E_NATIVE_PORT || 4693);
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["home", "data", "config", "cache", "state", "appdata", "localappdata", "webview"]) fs.mkdirSync(path.join(run, dir), { recursive: true });
const pageName = "Popup probe";
const pageBody = "GH809 relaunch popup body";
fs.writeFileSync(path.join(graph, `pages/${pageName}.md`), `- ${pageBody}\n`);
const fixture = accountHomeFixture(run, out);
process.env.E2E_WEBVIEW_USER_DATA_ROOT = path.join(run, "webview");
const env = { ...fixture.env, TINE_GRAPH: graph, HOME: path.join(run, "home"),
  XDG_DATA_HOME: path.join(run, "data"), XDG_CONFIG_HOME: path.join(run, "config"),
  XDG_CACHE_HOME: path.join(run, "cache"), XDG_STATE_HOME: path.join(run, "state"),
  APPDATA: path.join(run, "appdata"), LOCALAPPDATA: path.join(run, "localappdata"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
// tauri-plugin-window-state keeps its file in the app config directory. On Windows that directory comes from the
// Known Folder API (dirs-sys known_folder_roaming_app_data), which ignores the APPDATA override above, so the file is
// in the real profile. That is only acceptable on a throwaway hosted runner, never on someone's own machine.
if (process.platform === "win32" && process.env.GITHUB_ACTIONS !== "true") {
  throw new Error("On Windows this journey writes the real profile's window state: run it on a hosted runner only");
}
const stateFile = process.platform === "win32"
  ? path.join(process.env.APPDATA, APP_ID, ".window-state.json")
  : path.join(run, "config", APP_ID, ".window-state.json");
const report = { testedCommit: execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim(),
  journey: "scripts/e2e-popup-relaunch-809.mjs", platform: process.platform, app, stage: "launch-1", steps: [], status: "running",
  expected: "A clean quit saves no workspace-popup geometry; on relaunch with a saved popup geometry, Open in new window renders the page and main stays responsive" };
const write = () => fs.writeFileSync(path.join(out, "result.json"), JSON.stringify(report, null, 2) + "\n");
const step = (stage, detail) => { report.stage = stage; report.steps.push({ stage, at: Date.now(), detail }); write(); console.log(`ok: ${stage}`); };
write();

let target, driver, browser, log, wm, mainHandle;
const logs = [];
function stopSession() {
  if (driver?.pid) {
    if (process.platform === "win32") spawnSync("taskkill", ["/PID", String(driver.pid), "/T", "/F"], { stdio: "ignore" });
    else { try { process.kill(-driver.pid, "SIGKILL"); } catch { /* already exited */ } }
  }
  stopWebdriverApplication(target);
  driver = undefined; browser = undefined; target = undefined;
}
async function launch(name) {
  target = await startWebdriverApplication(app, env, nativePort, `gh809-${name}`);
  log = fs.openSync(path.join(out, `${name}-driver.log`), "w");
  logs.push(log);
  driver = spawn(process.env.TAURI_DRIVER || "tauri-driver",
    webdriverServerArgs(port, nativePort, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"),
    { env: target.env, stdio: ["ignore", log, log], detached: process.platform !== "win32" });
  await sleep(2000);
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error",
    connectionRetryCount: 0, connectionRetryTimeout: 15_000,
    capabilities: tauriCapabilities(app, `gh809-${name}`, process.platform, target.debuggerAddress),
  });
  await ensureMainWindow(browser);
  await browser.$(".ls-block").waitForExist({ timeout: 30_000 });
  await assertAppLease(fixture);
  mainHandle = await browser.getWindowHandle();
}
async function diagnostics() {
  const result = await browser.executeAsync(done => {
    window.__TAURI_INTERNALS__.invoke("diagnostic_report", { buildCommit: "", buildTime: "", renderer: null })
      .then(value => done({ text: value.text }), error => done({ error: String(error) }));
  });
  if (result.error) throw new Error(`Diagnostic capture failed: ${result.error}`);
  return JSON.parse(result.text);
}
// Ctrl+K > page result > right-click > Open in new window (the reporter's path).
async function openPopup(name) {
  const before = new Set(await browser.getWindowHandles());
  await browser.keys(["Control", "k"]);
  await browser.$(".switcher-input").waitForExist({ timeout: 5000 });
  await browser.$(".switcher-input").setValue(pageName);
  await browser.waitUntil(() => browser.execute((page) => {
    if (document.querySelector('#switcher-results [role="status"]')) return false;
    const row = [...document.querySelectorAll(".switcher-row:not(.block-result)")]
      .find(el => el.querySelector(".switcher-name")?.textContent?.trim() === page);
    if (!row) return false;
    row.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, button: 2, clientX: 400, clientY: 250 }));
    return true;
  }, pageName), { timeout: 10_000, timeoutMsg: "No matching Ctrl+K page result" });
  step(`${name}-open-requested`);
  await browser.execute(() => {
    const item = [...document.querySelectorAll(".ctx-page-item")].find(el => el.textContent?.trim() === "Open in new window");
    if (!item) throw new Error("Open in new window context-menu item absent");
    item.click();
  });
  const label = await popupRendered(before);
  step(`${name}-popup-rendered`, { label });
  return label;
}
// A window not in `before` shows the page; main then still answers a native
// round trip, every popup build returned, and no native stall is open.
async function popupRendered(before) {
  let popup;
  await browser.waitUntil(async () => {
    popup = (await browser.getWindowHandles()).find(handle => !before.has(handle));
    return !!popup;
  }, { timeout: 20_000, timeoutMsg: "No separate workspace window appeared" });
  // Hosted msedgedriver can lose a fresh popup's handle ("web view not found") although the popup booted (11/07,
  // 11/14 runs 43591083141, 43593140677). Retry the switch; if the driver never attaches, fall back to main's flight
  // recorder: the newest open must reach frontend.bootstrap-done (weaker: the page text is then not read).
  let label;
  for (let attempt = 1; attempt <= 3 && label === undefined; attempt++) {
    try {
      await browser.switchToWindow(popup);
      await browser.waitUntil(() => browser.execute((body) => document.body.textContent.includes(body), pageBody),
        { timeout: 20_000, timeoutMsg: "Popup stayed blank instead of showing the page" });
      label = await browser.execute(() => window.name);
    } catch (error) {
      if (!/web view not found|no such window/i.test(String(error?.message ?? error))) {
        // Record what the attached window actually shows, every handle's identity, and main's open stages.
        const seen = await browser.execute(() => ({ url: location.href, name: window.name, title: document.title,
          text: document.body.textContent.slice(0, 400) })).catch(e => String(e));
        const handles = [];
        for (const handle of await browser.getWindowHandles().catch(() => [])) {
          try {
            await browser.switchToWindow(handle);
            handles.push({ handle, url: await browser.getUrl(), name: await browser.execute(() => window.name) });
          } catch (e) { handles.push({ handle, error: String(e).slice(0, 120) }); }
        }
        await browser.switchToWindow(mainHandle).catch(() => {});
        const opens = await diagnostics().then(d => d.sessions.current.filter(e => e.event === "workspace.open"))
          .catch(e => String(e));
        step("popup-blank-evidence", { popup, before: [...before], seen, handles, opens });
        // The runner replaces result.json with its own summary; keep the evidence in stdout too.
        console.log(`EVIDENCE ${JSON.stringify({ popup, before: [...before], seen, handles, opens })}`);
        throw error;
      }
      step("popup-driver-lost-handle", { attempt });
      await browser.switchToWindow(mainHandle);
      await browser.pause(1_000);
    }
  }
  await browser.switchToWindow(mainHandle);
  if (label === undefined) {
    await browser.waitUntil(async () => {
      const opens = (await diagnostics()).sessions.current.filter(e => e.event === "workspace.open");
      const newest = Math.max(...opens.map(e => e.sequence));
      return opens.some(e => e.sequence === newest && e.stage === "frontend.bootstrap-done");
    }, { timeout: 20_000, timeoutMsg: "Popup never finished booting (main's flight recorder)" });
    label = "(driver lost the popup; booted per main's flight recorder)";
  }
  const events = (await diagnostics()).sessions.current;
  const entered = events.filter(e => e.stage === "native.builder-entry").map(e => e.sequence);
  const returned = new Set(events.filter(e => e.stage === "native.build-returned").map(e => e.sequence));
  if (!entered.length || entered.some(sequence => !returned.has(sequence))) {
    throw new Error(`A popup build did not return: entered ${entered}, returned ${[...returned]}`);
  }
  const stalled = events.filter(e => e.event === "runtime.main_thread_stall" && !e.recovered);
  if (stalled.length) throw new Error(`Unrecovered native main-thread stall: ${JSON.stringify(stalled)}`);
  return label;
}
// Keyboard input goes to the focused X window; a restored popup may hold it.
function activateMain() {
  if (process.platform !== "linux") return;
  const { xdo, geometry } = x11Tools(env);
  const ids = xdo("search", "--onlyvisible", "--name", ".").split(/\s+/).filter(Boolean);
  const windows = ids.map(id => {
    try { return { id, name: xdo("getwindowname", id), ...geometry(id) }; } catch { return { id }; }
  });
  // Popups take the page's title; main is the largest other titled window.
  const main = windows.filter(w => w.name && !w.name.includes(pageName) && w.WIDTH > 0)
    .sort((a, b) => b.WIDTH * b.HEIGHT - a.WIDTH * a.HEIGHT)[0];
  if (main) xdo("windowactivate", "--sync", main.id);
  return { windows, activated: main?.id ?? null };
}
function readState() {
  return JSON.parse(fs.readFileSync(stateFile, "utf8"));
}
async function quitCleanly(name) {
  const previous = fs.existsSync(stateFile) ? fs.statSync(stateFile).mtimeMs : 0;
  // The tray/keyboard Quit path: close requests, frontend flush, orderly exit.
  // 0.9 has no request_app_quit command; there the tray's Quit is a close request to main (tray::quit).
  await browser.execute(() => {
    const invoke = window.__TAURI_INTERNALS__.invoke;
    void invoke("request_app_quit").catch(() => invoke("plugin:window|close", { label: "main" }));
  }).catch(() => {});
  const deadline = Date.now() + 30_000;
  while (Date.now() < deadline) {
    if (fs.existsSync(stateFile) && fs.statSync(stateFile).mtimeMs > previous) break;
    await sleep(250);
  }
  if (!fs.existsSync(stateFile) || fs.statSync(stateFile).mtimeMs <= previous) {
    report.classification = "harness";
    throw new Error("The window-state file was not written by a clean quit");
  }
  await sleep(1500); // let the process finish exiting before the next launch
  stopSession();
  step(`${name}-quit-cleanly`, { stateFile });
}

const watchdog = setTimeout(() => {
  report.status = "failed";
  report.classification ??= "ambiguous";
  report.observed = `No completion within 240 seconds; last stage: ${report.stage}`;
  write(); stopSession(); wm?.kill(); fixture.close?.(); process.exit(1);
}, 240_000);
try {
  if (process.platform === "linux") {
    wm = spawn(process.env.E2E_WINDOW_MANAGER || "openbox", ["--sm-disable"], { env, stdio: "ignore" });
  }
  await launch("launch-1");
  step("launch-1-ready");
  await openPopup("launch-1");
  await quitCleanly("launch-1");
  const saved = readState();
  fs.writeFileSync(path.join(out, "launch-1-window-state.json"), JSON.stringify(saved, null, 2) + "\n");
  const popupLabels = Object.keys(saved).filter(label => label.startsWith("ws-"));
  if (!saved.main) { report.classification = "harness"; throw new Error(`No main entry in the saved window state: ${Object.keys(saved)}`); }
  // GH809_ROOT_CAUSE=1 runs a pre-fix build on to launch 2, where Windows froze.
  if (popupLabels.length && process.env.GH809_ROOT_CAUSE !== "1") {
    report.classification = "product";
    throw new Error(`The window-state plugin saved workspace popups: ${popupLabels.join(", ")}`);
  }
  step("launch-1-saved-window-state", { labels: Object.keys(saved), popupLabels });

  // The profile a reporter can hold: saved, on-screen, non-default geometry
  // for the first popup labels, offset from main so a restore must move it.
  const m = saved.main;
  for (let n = 1; n <= 4; n++) {
    saved[`ws-main-${n}`] = { ...m, x: m.x + 60 * n, y: m.y + 40 * n, prev_x: m.x + 60 * n, prev_y: m.y + 40 * n,
      width: 840, height: 640, maximized: false, visible: true, fullscreen: false };
  }
  fs.writeFileSync(stateFile, JSON.stringify(saved, null, 2));
  step("seeded-popup-geometry", { labels: Object.keys(saved) });

  report.stage = "launch-2";
  await launch("launch-2");
  step("launch-2-ready");
  // Session restore reopens launch 1's popup at startup, under a seeded label.
  const notPopups = new Set([mainHandle]);
  for (const handle of await browser.getWindowHandles()) {
    // A handle the driver cannot attach to (see popupRendered) is left as a popup candidate.
    try {
      await browser.switchToWindow(handle);
      if (isAuxiliaryWindow(await browser.getUrl())) notPopups.add(handle);
    } catch (error) {
      if (!/web view not found|no such window/i.test(String(error?.message ?? error))) throw error;
    }
  }
  await browser.switchToWindow(mainHandle);
  const restored = await popupRendered(notPopups);
  step("launch-2-restored-popup-rendered", { label: restored });
  step("launch-2-main-activated", { x11: activateMain() ?? null,
    focused: await browser.execute(() => document.hasFocus()) });
  await openPopup("launch-2");
  fs.writeFileSync(path.join(out, "launch-2-diagnostics.json"), JSON.stringify(await diagnostics(), null, 2));
  fixture.verify(); report.status = "passed"; write();
} catch (error) {
  report.status = "failed"; report.classification ??= "ambiguous"; report.observed = String(error);
  write();
  try {
    if (browser && mainHandle) {
      await browser.switchToWindow(mainHandle);
      await sleep(12_000); // let liveness probes and open stages reach the recorder
      fs.writeFileSync(path.join(out, "failure-diagnostics.json"), JSON.stringify(await diagnostics(), null, 2));
    }
  } catch (diagError) { report.diagnosticsError = String(diagError); }
  write(); process.exitCode = 1; console.error(error);
} finally {
  if (browser) await Promise.race([browser.deleteSession().catch(() => {}), sleep(1000)]);
  stopSession(); wm?.kill(); clearTimeout(watchdog); fixture.close?.();
  for (const fd of logs) { try { fs.closeSync(fd); } catch { /* closed */ } }
}
