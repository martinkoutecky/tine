// Linux real-app journey for the system tray (OG-TRAY, GH #625).
//
// Scenario 1 -- a desktop WITH a tray host. Xvfb has none, so the journey owns
// org.kde.StatusNotifierWatcher on its private session bus (lib/e2e-tray-host)
// and presses the real tray menu over com.canonical.dbusmenu. With the tray,
// "Minimize to tray" and "Start minimized to tray" all on:
//   - the app registers a status item and main stays hidden at launch, yet its
//     graph has loaded into the hidden webview;
//   - the tray's "Open Tine" shows main, and a block edited there reaches disk;
//   - minimizing main hides it (no taskbar entry), and "Open Tine" restores it;
//   - a second launch shows and focuses a hidden main.
//   - the tray host leaving while main is hidden shows main again and turns
//     minimize-to-tray off (never an invisible app).
// Scenario 2 -- a desktop WITHOUT a tray host, same settings: after the short
// autostart wait the window is shown, Settings is told why, and minimizing is
// an ordinary minimize.
// Scenario 3 -- the quit race: a hidden, graph-less main plus a graph window
// holding an unsaved edit; tray Quit must leave that edit on disk (the first
// window to finish may not exit the process under the other's flush).
// Scenario 4 -- a tray host that appears a few seconds AFTER launch (autostart
// before the panel): main stays hidden and the icon registers once it is up.
// The tray's left click is not delivered by AppIndicator on Linux; it is
// covered by the decision-function unit tests and the hosted Windows/macOS
// compilation, not here.
import { execFileSync, spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { APP_ID } from "./lib/app-identity.mjs";
import { x11Tools } from "./lib/e2e-x11.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { ensurePrivateSessionBus } from "./lib/e2e-session-bus.mjs";
import { startFakeTrayHost, clickTrayMenu, readTrayMenu } from "./lib/e2e-tray-host.mjs";

if (process.platform !== "linux") throw new Error("the tray journey drives X11 and D-Bus and is Linux-only");
ensurePrivateSessionBus();

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const TD = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin", "tauri-driver") : "tauri-driver");
const WD = process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver";
const DRIVER_BASE = Number(process.env.E2E_DRIVER_PORT || 4532);
const NATIVE_BASE = Number(process.env.E2E_NATIVE_PORT || 4533);
const TMP = "/tmp/tine-tray-e2e";
const GRAPH = `${TMP}/graph`;
const ARTIFACTS = process.env.E2E_ARTIFACT_DIR || `${TMP}/artifacts`;
const APP_DATA = `${TMP}/xdg/data/${APP_ID}`;
const now = new Date();
const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
const JOURNAL = `${GRAPH}/journals/${journal}.md`;

const env = {
  ...process.env,
  TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: `${TMP}/xdg/data`, XDG_CONFIG_HOME: `${TMP}/xdg/config`, XDG_CACHE_HOME: `${TMP}/xdg/cache`,
  XDG_CONFIG_DIRS: process.env.XDG_CONFIG_DIRS || "/etc/xdg",
  XDG_DATA_DIRS: process.env.XDG_DATA_DIRS || "/usr/local/share:/usr/share",
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11",
  LANG: process.env.LANG || "C.UTF-8",
};
const { xdo } = x11Tools(env);

async function until(predicate, timeoutMs, message) {
  const deadline = Date.now() + timeoutMs;
  let last;
  while (Date.now() < deadline) {
    try { last = await predicate(); if (last) return last; } catch (error) { last = error; }
    await sleep(100);
  }
  throw new Error(`${message}${last instanceof Error ? ` (${last.message})` : ""}`);
}
const shot = (name) => {
  try { execFileSync("import", ["-window", "root", path.join(ARTIFACTS, `${name}.png`)], { env }); } catch { /* evidence only */ }
};

/** Main's X11 window ids by title, mapped or not. Main is titled the product
 * name until a graph opens, then "Tine — <graph>"; Quick Capture and the
 * About window have their own titles. */
const MAIN_TITLE = "^Tine( — .*)?$";
const mainIds = ({ visibleOnly }) => {
  try {
    // stderr is dropped: a window destroyed mid-search prints a harmless BadWindow.
    return execFileSync(process.env.E2E_XDOTOOL || "xdotool", ["search", ...(visibleOnly ? ["--onlyvisible"] : []), "--name", MAIN_TITLE],
      { encoding: "utf8", env, stdio: ["ignore", "pipe", "ignore"] }).split(/\s+/).filter(Boolean);
  } catch {
    return [];
  }
};
const visibleMain = () => mainIds({ visibleOnly: true })[0];
/** WM_STATE of an X11 window: "Normal", "Iconic" or "Withdrawn". */
const wmState = (id) => /window state:\s*(\w+)/.exec(
  execFileSync("xprop", ["-id", id, "WM_STATE"], { encoding: "utf8", env }))?.[1] ?? "unknown";

const steps = [];
const step = (what) => { steps.push(what); console.log(`ok: ${what}`); };
const disk = () => fs.readFileSync(JOURNAL, "utf8");

function seed(settings) {
  fs.rmSync(TMP, { recursive: true, force: true });
  for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(`${GRAPH}/${dir}`, { recursive: true });
  for (const dir of ["data", "config", "cache"]) fs.mkdirSync(`${TMP}/xdg/${dir}`, { recursive: true });
  fs.mkdirSync(APP_DATA, { recursive: true });
  fs.mkdirSync(ARTIFACTS, { recursive: true });
  fs.writeFileSync(`${GRAPH}/logseq/config.edn`, "{}\n");
  fs.writeFileSync(JOURNAL, "- tray one\n- tray two\n");
  fs.writeFileSync(`${APP_DATA}/tine-settings.json`, `${JSON.stringify(settings)}\n`);
}

const wmLogFd = (() => { fs.mkdirSync(ARTIFACTS, { recursive: true }); return fs.openSync(path.join(ARTIFACTS, "window-manager.log"), "w"); })();
const wm = spawn(process.env.E2E_WINDOW_MANAGER || "openbox", ["--sm-disable"], { env, stdio: ["ignore", wmLogFd, wmLogFd], detached: true });
await sleep(600);
if (wm.exitCode != null) throw new Error("window manager exited early");

async function withApp(index, fn, { appEnv = env, whileLaunching } = {}) {
  const driverPort = DRIVER_BASE + index * 2;
  const log = fs.openSync(path.join(ARTIFACTS, `tauri-driver-${index}.log`), "w");
  const td = spawn(TD, ["--port", String(driverPort), "--native-port", String(NATIVE_BASE + index * 2), "--native-driver", WD], {
    env: appEnv, stdio: ["ignore", log, log], detached: true,
  });
  await sleep(2500);
  let browser;
  try {
    void whileLaunching?.();
    browser = await remote({
      hostname: "127.0.0.1", port: driverPort, path: "/", logLevel: "error", connectionRetryCount: 1, connectionRetryTimeout: 60_000,
      capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } },
    });
    await ensureMainWindow(browser);
    await fn(browser);
  } finally {
    try { await browser?.deleteSession(); } catch {}
    try { process.kill(-td.pid, "SIGKILL"); } catch {}
    fs.closeSync(log);
  }
}

const blockTexts = (browser) => browser.execute(() =>
  [...document.querySelectorAll(".ls-block[data-block-id]")].map((row) =>
    (row.querySelector("textarea.block-editor")?.value ?? row.querySelector(".block-content-wrapper")?.textContent ?? "").trim()));

/** Enter the editor on the block whose text is `text`, caret at its end. */
async function editBlock(browser, text) {
  const point = await until(() => browser.execute((wanted) => {
    const row = [...document.querySelectorAll(".ls-block[data-block-id] .block-content-wrapper")]
      .find((el) => el.textContent?.trim() === wanted);
    if (!row) return null;
    const r = row.getBoundingClientRect();
    return { x: Math.round(r.left + Math.min(r.width - 4, 40)), y: Math.round(r.top + r.height / 2) };
  }, text), 10_000, `no rendered block ${JSON.stringify(text)}`);
  await browser.performActions([{ type: "pointer", id: "mouse", parameters: { pointerType: "mouse" }, actions: [
    { type: "pointerMove", duration: 0, x: point.x, y: point.y, origin: "viewport" },
    { type: "pointerDown", button: 0 }, { type: "pointerUp", button: 0 },
  ] }]);
  await browser.releaseActions();
  await until(() => browser.execute(() => !!document.querySelector("textarea.block-editor")), 5_000, `clicking ${JSON.stringify(text)} did not open its editor`);
  await browser.keys(["End"]);
}

const trayStatus = (browser) => browser.executeAsync((done) => {
  window.__TAURI_INTERNALS__.invoke("tray_apply").then(done, (error) => done({ error: String(error) }));
});

// ---------------------------------------------------------------- scenario 1
seed({ tray_show: true, tray_minimize: true, tray_start_minimized: true });
const host = await startFakeTrayHost({ hostRegistered: true });
try {
  await withApp(0, async (browser) => {
    const item = await until(() => host.items[0], 15_000, "the app registered no status item with the tray host");
    step(`the app registered a status item (${item.service})`);
    const { entries } = readTrayMenu(item, env);
    const labels = entries.map((entry) => entry.label.replace(/_/g, ""));
    for (const wanted of ["Open Tine", "Quick Capture", "Quit"]) {
      if (!labels.includes(wanted)) throw new Error(`tray menu lacks "${wanted}": ${JSON.stringify(labels)}`);
    }
    step(`the tray menu offers ${labels.join(", ")}`);

    // Start minimized: the window is never mapped, yet the app works (the
    // blocks below render into the hidden webview).
    if (visibleMain()) throw new Error("main is visible although start-minimized-to-tray is on");
    await until(async () => (await blockTexts(browser)).includes("tray one"), 30_000, "the graph did not load into the hidden main window");
    step("main stayed hidden at launch while its graph loaded");
    // A hidden window must stay hidden until asked: not even the startup
    // reveal fallback (3 s) may show it.
    await sleep(4_500);
    if (visibleMain()) throw new Error("main appeared on its own after the startup reveal fallback");
    step("main stayed hidden past the startup reveal fallback");

    // Tray > Open Tine.
    clickTrayMenu(item, "Open Tine", env);
    const shown = await until(() => visibleMain(), 15_000, "tray Open Tine did not show main");
    shot("01-opened-from-tray");
    await until(async () => (await blockTexts(browser)).includes("tray one"), 15_000, "main rendered no blocks after the tray opened it");
    step("tray Open Tine showed main with its page rendered");

    // The revealed window is fully working: edit with real keys, see the disk.
    xdo("windowactivate", "--sync", shown);
    await editBlock(browser, "tray one");
    xdo("type", "--delay", "40", " edited after tray open");
    xdo("key", "--delay", "60", "Escape");
    await until(() => disk().includes("- tray one edited after tray open"), 15_000,
      `the block edited after tray Open Tine never reached disk: ${JSON.stringify(disk())}`);
    step("a block edited in the window shown from the tray reached disk");

    // Minimize to tray.
    xdo("windowminimize", shown);
    await until(() => !visibleMain(), 10_000, "minimizing main with minimize-to-tray on left it visible");
    const state = await until(() => { const s = mainIds({ visibleOnly: false }).map(wmState); return s.length ? s : null; }, 5_000, "main vanished instead of hiding");
    // A hidden (withdrawn) window carries no WM_STATE; an iconified one is
    // "Iconic" and would keep its taskbar entry (scenario 2 shows that case).
    if (state.some((s) => s !== "unknown" && s !== "Withdrawn")) throw new Error(`main is only minimized (${state}), so it keeps a taskbar entry`);
    shot("02-minimized-to-tray");
    step(`minimizing hid main from the window list (WM_STATE ${state.join(",")})`);
    clickTrayMenu(item, "Open Tine", env);
    await until(() => visibleMain(), 15_000, "tray Open Tine did not restore the minimized main");
    step("tray Open Tine restored the hidden main");

    // A second launch shows and focuses a hidden main.
    xdo("windowminimize", visibleMain());
    await until(() => !visibleMain(), 10_000, "main did not hide for the second-launch check");
    const second = spawn(APP, [], { env, stdio: "ignore", detached: true });
    second.unref();
    await until(() => visibleMain(), 20_000, "a second launch did not show the hidden main");
    step("a second launch showed the hidden main");

    // The host leaves while main is hidden in the tray: nothing could bring
    // the window back, so it must be shown, and minimizing is ordinary again.
    xdo("windowminimize", visibleMain());
    await until(() => !visibleMain(), 10_000, "main did not hide before the tray host left");
    host.close();
    const rescued = await until(() => visibleMain(), 15_000, "the tray host left while main was hidden and main stayed hidden: an invisible app");
    shot("04-host-left");
    step("the tray host leaving showed the hidden main");
    xdo("windowminimize", rescued);
    const iconic = await until(() => { const s = mainIds({ visibleOnly: false }).map(wmState); return s.includes("Iconic") ? s : null; }, 10_000,
      "after the host left, minimize was not an ordinary minimize");
    step(`after the host left minimize is ordinary (WM_STATE ${iconic.join(",")})`);
    xdo("windowmap", "--sync", rescued);
    await until(() => visibleMain(), 10_000, "the window did not come back from the ordinary minimize");

    // Close is unchanged: it quits (the flush handler, then the process).
    await browser.execute(() => document.querySelector(".win-close")?.click());
    await until(() => mainIds({ visibleOnly: false }).length === 0, 20_000, "closing main did not quit the app");
    step("closing main still quits the app");
  });
} finally {
  host.close();
}

// ---------------------------------------------------------------- scenario 2
seed({ tray_show: true, tray_minimize: true, tray_start_minimized: true });
await withApp(1, async (browser) => {
  const shown = await until(() => visibleMain(), 30_000,
    "with no tray host the window stayed hidden: the user would be left with an invisible app");
  await until(async () => (await blockTexts(browser)).includes("tray one"), 30_000, "the graph did not load");
  shot("03-no-tray-host");
  step("without a tray host the window is shown despite start-minimized");
  const status = await trayStatus(browser);
  if (status.active || !status.problem) throw new Error(`Settings would not be told the tray is unavailable: ${JSON.stringify(status)}`);
  step(`Settings is told why: ${JSON.stringify(status.problem)}`);
  xdo("windowminimize", shown);
  const state = await until(() => { const s = mainIds({ visibleOnly: false }).map(wmState); return s.includes("Iconic") ? s : null; }, 10_000,
    "without a tray host minimize did not leave an ordinary minimized window");
  step(`without a tray host minimize is an ordinary minimize (WM_STATE ${state.join(",")})`);
  xdo("windowmap", "--sync", shown);
});

// ---------------------------------------------------------------- scenario 3
// No graph in main (hidden, graph-less) and a second window holding the graph.
// The edit is typed and the tray's Quit pressed within the save debounce, so
// the only thing that can save it is the window's own close flush; main has
// nothing to flush and finishes first.
seed({ tray_show: true, tray_start_minimized: true });
const quitHost = await startFakeTrayHost({ hostRegistered: true });
try {
  await withApp(2, async (browser) => {
    const item = await until(() => quitHost.items[0], 15_000, "scenario 3: no status item registered");
    if (visibleMain()) throw new Error("scenario 3: the graph-less main should start hidden");
    const mainHandle = await browser.getWindowHandle();
    const before = await browser.getWindowHandles();
    await browser.executeAsync((path, done) => {
      window.__TAURI_INTERNALS__.invoke("open_graph_window", { path }).then(done, (error) => done({ error: String(error) }));
    }, GRAPH);
    const graphHandle = await until(async () => (await browser.getWindowHandles()).find((h) => !before.includes(h)), 20_000,
      "open_graph_window created no graph window");
    await browser.switchToWindow(graphHandle);
    await until(async () => (await blockTexts(browser)).includes("tray one"), 30_000, "the graph window did not load the graph");
    const graphId = await until(() => {
      const ids = execFileSync(process.env.E2E_XDOTOOL || "xdotool", ["search", "--onlyvisible", "--name", "^Tine — "],
        { encoding: "utf8", env, stdio: ["ignore", "pipe", "ignore"] }).split(/\s+/).filter(Boolean);
      return ids[0];
    }, 15_000, "the graph window is not on screen");
    step("a hidden graph-less main and a graph window are open");
    xdo("windowactivate", "--sync", graphId);
    await editBlock(browser, "tray one");
    xdo("type", "--delay", "30", " saved-by-quit");
    clickTrayMenu(item, "Quit", env);
    await until(() => mainIds({ visibleOnly: false }).length === 0, 30_000, "tray Quit did not end the app");
    await until(() => disk().includes("- tray one saved-by-quit"), 5_000,
      `tray Quit exited before the graph window saved its edit: ${JSON.stringify(disk())}`);
    step("tray Quit left the graph window's unsaved edit on disk");
    void mainHandle;
  }, { appEnv: { ...env, TINE_GRAPH: "" } });
} finally {
  quitHost.close();
}

// ---------------------------------------------------------------- scenario 4
seed({ tray_show: true, tray_minimize: true, tray_start_minimized: true });
let lateHost;
try {
  await withApp(3, async (browser) => {
    if (visibleMain()) throw new Error("scenario 4: main was shown before any tray host existed (it should wait for the panel)");
    const item = await until(() => lateHost?.items[0], 20_000, "scenario 4: the late tray host never got a status item");
    await until(async () => (await blockTexts(browser)).includes("tray one"), 30_000, "scenario 4: the graph did not load");
    if (visibleMain()) throw new Error("scenario 4: main appeared although the tray came up (start minimized)");
    step(`a tray host that appeared after launch got the icon (${item.service}) and main stayed hidden`);
    clickTrayMenu(item, "Open Tine", env);
    const shown = await until(() => visibleMain(), 15_000, "scenario 4: tray Open Tine did not show main");
    xdo("windowminimize", shown);
    await until(() => !visibleMain(), 10_000, "scenario 4: minimize-to-tray was not active after the late host appeared");
    step("minimize-to-tray works once the late host was found");
    clickTrayMenu(item, "Open Tine", env);
    await until(() => visibleMain(), 15_000, "scenario 4: could not restore main");
  }, { whileLaunching: () => sleep(2500).then(startFakeTrayHost).then((h) => { lateHost = h; }) });
} finally {
  lateHost?.close();
}

console.log(`PASS: tray journey (${steps.length} steps)`);
try { process.kill(-wm.pid, "SIGKILL"); } catch {}
fs.closeSync(wmLogFd);
