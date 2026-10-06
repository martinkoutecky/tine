// Linux real-app journey for workspace windows (OG-MULTIWINDOW): open a page in
// a new window from the page actions menu, type and press Enter there, see the
// text in main, undo from the new window, type again and close the window from
// its native title bar at once, find that text on disk, then relaunch and see
// the window restored. The popup is a real engine-linked native window driven
// by main's JavaScript; keyboard input reaches it through the OS (xdotool), and
// its native close through the window manager's own close button.
import { execFileSync, spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { APP_ID, IDENTITY } from "./lib/app-identity.mjs";
import { x11Tools } from "./lib/e2e-x11.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";

if (process.platform !== "linux") throw new Error("the multi-window journey drives X11 and is Linux-only");

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const TD = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin", "tauri-driver") : "tauri-driver");
const WD = process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver";
const DRIVER_BASE = Number(process.env.E2E_DRIVER_PORT || 4512);
const NATIVE_BASE = Number(process.env.E2E_NATIVE_PORT || 4513);
const TMP = "/tmp/tine-multiwindow-e2e";
const GRAPH = `${TMP}/graph`;
const PAGE = `${GRAPH}/pages/Alpha.md`;
const ARTIFACTS = process.env.E2E_ARTIFACT_DIR || `${TMP}/artifacts`;

fs.rmSync(TMP, { recursive: true, force: true });
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(`${GRAPH}/${dir}`, { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(`${TMP}/xdg/${dir}`, { recursive: true });
fs.mkdirSync(ARTIFACTS, { recursive: true });
fs.writeFileSync(`${GRAPH}/logseq/config.edn`, "{}\n");
fs.writeFileSync(PAGE, "- alpha one\n- alpha two\n");
fs.writeFileSync(`${GRAPH}/pages/Beta.md`, "- beta one\n");
const now = new Date();
const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(`${GRAPH}/journals/${journal}.md`, "- open [[Alpha]]\n");
const APP_DATA = `${TMP}/xdg/data/${APP_ID}`;

const env = {
  ...process.env,
  TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: `${TMP}/xdg/data`, XDG_CONFIG_HOME: `${TMP}/xdg/config`, XDG_CACHE_HOME: `${TMP}/xdg/cache`,
  XDG_CONFIG_DIRS: process.env.XDG_CONFIG_DIRS || "/etc/xdg",
  XDG_DATA_DIRS: process.env.XDG_DATA_DIRS || "/usr/local/share:/usr/share",
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11",
};
const { xdo, geometry, frameExtents } = x11Tools(env);

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

/** X11 ids of visible windows titled exactly `title`. */
function windowsTitled(title) {
  try {
    return xdo("search", "--onlyvisible", "--name", `^${title.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}$`).split(/\s+/).filter(Boolean);
  } catch {
    return [];
  }
}

const POPUP_TITLE = "Alpha — Tine";

const wmLog = fs.openSync(path.join(ARTIFACTS, "window-manager.log"), "w");
const wm = spawn(process.env.E2E_WINDOW_MANAGER || "openbox", ["--sm-disable"], { env, stdio: ["ignore", wmLog, wmLog], detached: true });
await sleep(600);
if (wm.exitCode != null) throw new Error("window manager exited early");

async function withApp(index, fn) {
  const driverPort = DRIVER_BASE + index * 2;
  const log = fs.openSync(path.join(ARTIFACTS, `tauri-driver-${index}.log`), "w");
  const td = spawn(TD, ["--port", String(driverPort), "--native-port", String(NATIVE_BASE + index * 2), "--native-driver", WD], {
    env, stdio: ["ignore", log, log], detached: true,
  });
  await sleep(2500);
  let browser;
  try {
    browser = await remote({
      hostname: "127.0.0.1", port: driverPort, path: "/", logLevel: "error", connectionRetryCount: 1, connectionRetryTimeout: 60_000,
      capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } },
    });
    await ensureMainWindow(browser);
    await browser.$(".ls-block, .page-title").waitForExist({ timeout: 30_000 });
    await fn(browser);
  } finally {
    try { await browser?.deleteSession(); } catch {}
    try { process.kill(-td.pid, "SIGKILL"); } catch {}
    fs.closeSync(log);
  }
}

/** Block texts as main renders them for page Alpha. */
const mainTexts = (browser) => browser.execute(() =>
  [...document.querySelectorAll(".ls-block[data-block-id]")].map((row) =>
    (row.querySelector("textarea.block-editor")?.value ?? row.querySelector(".block-content-wrapper")?.textContent ?? "").trim()));

/** What the current window shows, for failure messages. */
const windowState = (browser) => browser.execute(() => ({
  title: document.title,
  texts: [...document.querySelectorAll(".ls-block[data-block-id]")].map((row) =>
    (row.querySelector("textarea.block-editor")?.value ?? row.querySelector(".block-content-wrapper")?.textContent ?? "").trim()),
  active: document.activeElement ? `${document.activeElement.tagName}.${document.activeElement.className}` : null,
  hasFocus: document.hasFocus(),
}));

/** Switch the session to the window whose document is titled `title`. */
async function switchToTitled(browser, title, timeoutMs = 15_000) {
  return until(async () => {
    for (const handle of await browser.getWindowHandles()) {
      await browser.switchToWindow(handle);
      if ((await browser.getTitle()) === title) return handle;
    }
    return null;
  }, timeoutMs, `no WebDriver window titled ${title}`);
}

/** Enter the editor on the block whose text is `text`, caret at its end, with
 * a real pointer click at its rendered position. (Element references cannot
 * be serialized out of a popup document by WebKitWebDriver, so the journey
 * locates by script and clicks by viewport coordinates.) */
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

async function openActionsNewWindow(browser) {
  await browser.execute(() => document.querySelector("[data-page-actions-trigger]")?.click());
  await until(() => browser.execute(() => {
    const item = [...document.querySelectorAll(".ctx-page-item")].find((el) => el.textContent?.trim() === "Open in new window");
    if (!item) return false;
    item.click();
    return true;
  }), 5_000, "the page actions menu offered no \"Open in new window\"");
}

/** Click the window manager's own close button on X11 window `id`. */
function clickNativeClose(id) {
  const g = geometry(id);
  const extents = frameExtents(id);
  const x = g.X + g.WIDTH - Math.max(10, Math.floor(extents.right / 2));
  const y = g.Y - Math.max(1, Math.floor(extents.top / 2));
  xdo("mousemove", "--sync", String(x), String(y));
  xdo("click", "1");
  return { x, y, g, extents };
}

function sessionWithWindows() {
  const found = [];
  const walk = (dir) => {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      const file = path.join(dir, entry.name);
      if (entry.isDirectory()) walk(file);
      else if (path.basename(dir) === "sessions" && entry.name.endsWith(".json")) {
        try {
          const parsed = JSON.parse(fs.readFileSync(file, "utf8"));
          if (Array.isArray(parsed.windows) && parsed.windows.length) found.push({ file, windows: parsed.windows });
        } catch { /* partially written or another format */ }
      }
    }
  };
  if (fs.existsSync(APP_DATA)) walk(APP_DATA);
  return found[0] ?? null;
}

/** Real OS keyboard input into X11 window `id` (WebKitWebDriver's synthetic
 * key sequence drops repeated characters and spaces in this environment, in
 * main and popup alike). */
async function osType(id, text) {
  xdo("windowactivate", "--sync", id);
  await sleep(150);
  xdo("type", "--delay", "40", text);
}
async function osKey(id, ...keys) {
  xdo("windowactivate", "--sync", id);
  await sleep(150);
  xdo("key", "--delay", "60", ...keys);
}
/** Main's X11 window: titled the product name until a graph opens, then
 * "Tine — <graph>" (graph.rs); workspace windows are titled "<page> — Tine". */
const mainX = () => {
  for (const pattern of [`^Tine — `, `^${IDENTITY.productName}$`]) {
    try {
      const ids = xdo("search", "--onlyvisible", "--name", pattern).split(/\s+/).filter(Boolean);
      if (ids[0]) return ids[0];
    } catch { /* none titled so */ }
  }
  throw new Error("main window not found by title");
};

const disk = () => fs.readFileSync(PAGE, "utf8");
const steps = [];
const step = (what) => { steps.push(what); console.log(`ok: ${what}`); };

await withApp(0, async (browser) => {
  const main = await browser.getWindowHandle();
  await openPageByName(browser, "Alpha");
  await until(async () => (await mainTexts(browser)).includes("alpha two"), 15_000, "Alpha did not render in main");
  await openActionsNewWindow(browser);
  const popupX = await until(() => windowsTitled(POPUP_TITLE)[0], 15_000, `no native window titled ${POPUP_TITLE} appeared`);
  const popup = await switchToTitled(browser, POPUP_TITLE);
  await until(async () => (await mainTexts(browser)).includes("alpha two"), 15_000, "Alpha did not render in the new window");
  shot("01-popup-open");
  step("Open in new window opened a decorated native window rendering the page");

  // Type and press Enter in the new window.
  xdo("windowactivate", "--sync", popupX);
  await editBlock(browser, "alpha two");
  await osKey(popupX, "Return");
  await osType(popupX, "typed in popup");
  await until(async () => (await mainTexts(browser)).includes("typed in popup"), 5_000, "the new window did not show its own typing")
    .catch(async (error) => { throw new Error(`${error.message}; window state ${JSON.stringify(await windowState(browser))}`); });
  await browser.switchToWindow(main);
  await until(async () => (await mainTexts(browser)).includes("typed in popup"), 10_000, "main never showed the text typed in the new window");
  step("text typed after Enter in the new window appeared in main");

  // Undo from the new window.
  await browser.switchToWindow(popup);
  await osKey(popupX, "Escape", "ctrl+z");
  await browser.switchToWindow(main);
  await until(async () => !(await mainTexts(browser)).includes("typed in popup"), 10_000, "undo in the new window did not reach main");
  step("undo pressed in the new window reverted the edit in main");

  // Type, then close the window natively at once: the text must reach disk.
  await browser.switchToWindow(popup);
  xdo("windowactivate", "--sync", popupX);
  await editBlock(browser, "alpha one");
  await osType(popupX, " closed at once");
  const click = clickNativeClose(popupX);
  await until(() => windowsTitled(POPUP_TITLE).length === 0, 10_000, `the native close button did not close the window (${JSON.stringify(click)})`);
  await until(async () => (await browser.getWindowHandles()).length === 2, 10_000, "a closed window stayed registered with the driver");
  await browser.switchToWindow(main);
  await until(() => disk().includes("- alpha one closed at once"), 10_000, `text typed just before the native close never reached disk: ${JSON.stringify(disk())}`);
  await until(async () => (await mainTexts(browser)).includes("alpha one closed at once"), 5_000, "main lost the text typed before the close");
  shot("02-after-native-close");
  step("text typed just before a native close reached disk and the window unregistered");

  // Main keeps working after the window closed.
  xdo("windowactivate", "--sync", mainX());
  await editBlock(browser, "alpha two");
  await osType(mainX(), " main still saves");
  await osKey(mainX(), "Escape");
  await until(() => disk().includes("- alpha two main still saves"), 10_000, "main stopped saving after the window closed");
  step("main still edits and saves after the window closed");

  // Leave a window open for the relaunch.
  await openActionsNewWindow(browser);
  await until(() => windowsTitled(POPUP_TITLE)[0], 15_000, "the second new window did not appear");
  const saved = await until(() => sessionWithWindows(), 10_000, "the session never listed the open window");
  if (saved.windows.length !== 1) throw new Error(`session lists ${saved.windows.length} windows, expected 1`);
  step(`the session lists the open window (${path.basename(saved.file)})`);

  // Quit from main's own close button: the window closes with the app and the
  // quit-time session still lists it.
  await browser.switchToWindow(main);
  await browser.execute(() => document.querySelector(".win-close")?.click());
  await until(() => {
    if (windowsTitled(POPUP_TITLE).length) return false;
    try { mainX(); return false; } catch { return true; }
  }, 20_000, "quitting from main's close button did not close the app and its window");
  const afterQuit = sessionWithWindows();
  if (afterQuit?.windows.length !== 1) throw new Error(`after quit the session lists ${afterQuit?.windows.length ?? 0} windows, expected 1`);
  step("quitting from main closed the window and kept it in the session");
});

await withApp(1, async (browser) => {
  await until(() => windowsTitled(POPUP_TITLE)[0], 30_000, "the window open at quit was not restored on relaunch");
  await switchToTitled(browser, POPUP_TITLE);
  await until(async () => (await mainTexts(browser)).includes("alpha one closed at once"), 15_000, "the restored window did not render its page");
  shot("03-restored");
  step("relaunch restored the window with its page");
});
console.log(`PASS: multi-window journey (${steps.length} steps)`);
try { process.kill(-wm.pid, "SIGKILL"); } catch {}
fs.closeSync(wmLog);
