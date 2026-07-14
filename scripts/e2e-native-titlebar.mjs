// Linux native-titlebar regression: start the real app with the persisted native
// frame preference, prove the window manager added a decorated frame, then click
// its actual close button. The window must traverse Tine's safe persistence
// handler and disappear. Preference-vs-active behavior is covered separately by
// nativeChrome.test.ts because Tao cannot redecorate an existing GTK window.
import { execFileSync, spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

if (process.platform !== "linux") throw new Error("native titlebar regression is Linux-only");

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const TD = process.env.TAURI_DRIVER || "tauri-driver";
const WD = process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver";
const XDOTOOL = process.env.E2E_XDOTOOL || "xdotool";
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4464);
const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4465);
const TMP = "/tmp/tine-native-titlebar-e2e";
const GRAPH = `${TMP}/graph`;
const ARTIFACTS = process.env.E2E_ARTIFACT_DIR || TMP;

fs.rmSync(TMP, { recursive: true, force: true });
for (const dir of ["pages", "journals", "logseq", "assets"]) fs.mkdirSync(`${GRAPH}/${dir}`, { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(`${TMP}/xdg/${dir}`, { recursive: true });
fs.mkdirSync(ARTIFACTS, { recursive: true });
const appData = `${TMP}/xdg/data/page.tine.Tine`;
fs.mkdirSync(appData, { recursive: true });
fs.writeFileSync(`${appData}/tine-settings.json`, '{"native_window_frame":true}\n');
fs.writeFileSync(`${GRAPH}/logseq/config.edn`, "{}\n");
fs.writeFileSync(`${GRAPH}/pages/Home.md`, "- native titlebar fixture\n");
const now = new Date();
const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(`${GRAPH}/journals/${journal}.md`, "- open [[Home]]\n");

const env = {
  ...process.env,
  TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: `${TMP}/xdg/data`,
  XDG_CONFIG_HOME: `${TMP}/xdg/config`,
  XDG_CACHE_HOME: `${TMP}/xdg/cache`,
  XDG_CONFIG_DIRS: process.env.XDG_CONFIG_DIRS || "/etc/xdg",
  XDG_DATA_DIRS: process.env.XDG_DATA_DIRS || "/usr/local/share:/usr/share",
  WEBKIT_DISABLE_DMABUF_RENDERER: "1",
  WEBKIT_DISABLE_COMPOSITING_MODE: "1",
  LIBGL_ALWAYS_SOFTWARE: "1",
  GDK_BACKEND: "x11",
};
const xdoEnv = process.env.E2E_XDOTOOL_LIB
  ? { ...env, LD_LIBRARY_PATH: process.env.E2E_XDOTOOL_LIB }
  : env;
const xdo = (...args) => execFileSync(XDOTOOL, args, { encoding: "utf8", env: xdoEnv }).trim();
const windowIds = () => {
  try {
    // xdotool uses POSIX extended regular expressions (no `(?:...)`).
    return xdo("search", "--onlyvisible", "--name", "^Tine( — .*)?$")
      .split(/\s+/)
      .filter(Boolean)
      // Tauri/Openbox can also expose a tiny same-title helper surface. The
      // graph window is the largest visible match and owns the real frame.
      .sort((a, b) => {
        try {
          const ga = geometry(a);
          const gb = geometry(b);
          return gb.WIDTH * gb.HEIGHT - ga.WIDTH * ga.HEIGHT;
        } catch {
          return 0;
        }
      });
  } catch {
    return [];
  }
};
const waitFor = async (predicate, timeoutMs, message) => {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const value = predicate();
    if (value) return value;
    await sleep(50);
  }
  throw new Error(message);
};
const geometry = (id) => Object.fromEntries(
  xdo("getwindowgeometry", "--shell", id)
    .split("\n")
    .filter((line) => line.includes("="))
    .map((line) => line.split("=", 2))
    .map(([key, value]) => [key, Number(value)]),
);
const frameExtents = (id) => {
  const raw = execFileSync("xprop", ["-id", id, "_NET_FRAME_EXTENTS", "_GTK_FRAME_EXTENTS"], { encoding: "utf8", env });
  const values = raw.match(/=\s*(\d+),\s*(\d+),\s*(\d+),\s*(\d+)/)?.slice(1).map(Number);
  // An undecorated window commonly has no property at all; that is equivalent
  // to zero extents and is the expected pre-toggle state.
  if (!values && /not found/i.test(raw)) return { left: 0, right: 0, top: 0, bottom: 0 };
  if (!values) throw new Error(`window manager exposed malformed frame extents: ${raw.trim()}`);
  const [left, right, top, bottom] = values;
  return { left, right, top, bottom };
};

const wmLog = fs.openSync(path.join(ARTIFACTS, "window-manager.log"), "w");
const wm = spawn(process.env.E2E_WINDOW_MANAGER || "openbox", ["--sm-disable"], {
  env, stdio: ["ignore", wmLog, wmLog], detached: true,
});
await sleep(600);
if (wm.exitCode != null) throw new Error(`window manager exited early: ${fs.readFileSync(path.join(ARTIFACTS, "window-manager.log"), "utf8")}`);

const driverLog = fs.openSync(path.join(ARTIFACTS, "tauri-driver.log"), "w");
const td = spawn(TD, ["--port", String(DRIVER_PORT), "--native-port", String(NATIVE_PORT), "--native-driver", WD], {
  env, stdio: ["ignore", driverLog, driverLog], detached: true,
});
await sleep(2500);

let browser;
try {
  browser = await remote({
    hostname: "127.0.0.1", port: DRIVER_PORT, path: "/", logLevel: "error",
    connectionRetryCount: 1, connectionRetryTimeout: 60_000,
    capabilities: {
      browserName: "wry",
      "wdio:enforceWebDriverClassic": true,
      "tauri:options": { application: APP },
    },
  });
  await browser.$(".ls-block, .page-title").waitForExist({ timeout: 20_000 });
  const decoratedId = await waitFor(() => windowIds()[0], 10_000, "Tine window did not appear");
  if (!decoratedId) throw new Error("Tine window disappeared before the native close test");
  const decorated = { id: decoratedId, extents: frameExtents(decoratedId) };
  if (decorated.extents.top <= 4) {
    throw new Error(`persisted native frame was not applied at window construction: ${JSON.stringify(decorated.extents)}`);
  }

  await browser.$('button[title^="Settings"]').click();
  const field = await browser.$('[data-setting-label="System title bar & window controls"]');
  await field.waitForExist({ timeout: 5_000 });
  const toggle = await field.$('button[role="switch"]');
  if ((await toggle.getAttribute("aria-checked")) !== "true") {
    throw new Error("Settings did not reflect the native frame applied at startup");
  }

  // Allow the close-request handler to be installed before driving the actual
  // window-manager widget rather than synthesizing WM_DELETE_WINDOW directly.
  await sleep(500);
  const g = geometry(decorated.id);
  // X/Y describe the client-area origin. The native close button lives in the
  // top-right of the window-manager frame, above that client area.
  const titlebarHeight = decorated.extents.top > 4 ? decorated.extents.top : 24;
  const closeX = g.X + g.WIDTH - Math.max(10, Math.floor(decorated.extents.right / 2));
  const closeY = g.Y - Math.max(4, Math.floor(titlebarHeight / 2));
  execFileSync("import", ["-window", "root", path.join(ARTIFACTS, "native-titlebar-before-close.png")], { env });
  xdo("mousemove", "--sync", String(closeX), String(closeY));
  xdo("click", "1");

  await sleep(800);
  let clickState = { closed: windowIds().length === 0 };
  if (!clickState.closed) {
    clickState = await browser.execute(() => ({
      closed: false,
      transitionShield: Boolean(document.querySelector(".graph-transition-shield")),
      activeTag: document.activeElement?.tagName ?? "",
      activeClass: document.activeElement?.getAttribute("class") ?? "",
    }));
    // Diagnostic control: WM_DELETE_WINDOW bypasses the decoration's pointer
    // widget but enters the same application close-request path. If this works,
    // the defect is the runtime-created frame rather than Tine's safe-close
    // handler. The test still fails because the actual close button did not.
    xdo("windowclose", decorated.id);
    await sleep(800);
    const semanticState = windowIds().length === 0
      ? { closed: true }
      : await browser.execute(() => ({
          closed: false,
          transitionShield: Boolean(document.querySelector(".graph-transition-shield")),
        }));
    throw new Error(`native close button was inert; clickState=${JSON.stringify(clickState)} semanticClose=${JSON.stringify(semanticState)} geometry=${JSON.stringify(g)} extents=${JSON.stringify(decorated.extents)} click=${closeX},${closeY}`);
  }
  await waitFor(() => windowIds().length === 0, 12_000,
    `native close control did not close Tine; geometry=${JSON.stringify(g)} extents=${JSON.stringify(decorated.extents)} click=${closeX},${closeY} state=${JSON.stringify(clickState)}`);
  console.log(`PASS: Linux native close control closed Tine safely; extents=${JSON.stringify(decorated.extents)} click=${closeX},${closeY}`);
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-td.pid, "SIGKILL"); } catch {}
  try { process.kill(-wm.pid, "SIGKILL"); } catch {}
  fs.closeSync(driverLog);
  fs.closeSync(wmLog);
}
