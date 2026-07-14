// Native Quick Capture activation regression. The second `tine --capture`
// process exercises the same single-instance handoff as a desktop global
// shortcut. xdotool sends real X11 keyboard input without WebDriver ever
// touching the auxiliary window, then the saved journal proves that input
// landed in the capture editor.
import { execFileSync, spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const XDOTOOL = process.env.E2E_XDOTOOL || "xdotool";
const TMP = "/tmp/tine-capture-e2e";
const GRAPH = `${TMP}/graph`;
const PROOF = "capture-focus-proof";
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
  throw new Error(`${wanted} never received native focus; state=${JSON.stringify(state)}`);
};

const appLog = fs.openSync(`${ARTIFACT_DIR}/tine.log`, "w");
const app = spawn(APP, [], { env, stdio: ["ignore", appLog, appLog], detached: true });
try {
  await waitForWindow("Tine", 20_000);

  const second = spawn(APP, ["--capture"], { env, stdio: ["ignore", appLog, appLog], detached: true });
  second.unref();
  await waitForWindow("Quick Capture", 10_000);

  // Model the short interval between seeing the newly painted window and a
  // human's first keystroke, while still proving that focus remains native.
  await sleep(300);
  const quickCapture = await waitForWindow("Quick Capture", 100);
  if (!matchesWindowName(quickCapture.focus.name, "Quick Capture")) {
    throw new Error(`Quick Capture lacked X input focus; state=${JSON.stringify(quickCapture)}`);
  }
  if (quickCapture.active.name && quickCapture.active.id !== quickCapture.focus.id) {
    throw new Error(`Quick Capture was EWMH-active but lacked X input focus; state=${JSON.stringify(quickCapture)}`);
  }
  // Native activation and X input focus have now been proven without a click.
  // Click only after that assertion to exercise the rest of the capture/save
  // path; WebKitGTK rejects untrusted synthetic text focus under Xvfb even when
  // its real native window has focus, unlike physical keyboard input.
  xdo("mousemove", "--window", quickCapture.focus.id, "90", "66", "click", "1");
  // Let WebKit finish the synthetic pointer-focus transition before injecting
  // text; otherwise Xvfb can consume the first character as the editor mounts.
  await sleep(150);
  xdo("type", "--clearmodifiers", PROOF);
  await sleep(150);
  xdo("key", "--clearmodifiers", "ctrl+shift+Return");

  const deadline = Date.now() + 10_000;
  while (Date.now() < deadline && !fs.readFileSync(journalPath, "utf8").includes(PROOF)) await sleep(100);
  if (!fs.readFileSync(journalPath, "utf8").includes(PROOF)) {
    throw new Error(`native typing did not reach the graph; journal=${JSON.stringify(fs.readFileSync(journalPath, "utf8"))}`);
  }
  console.log("PASS: Quick Capture received native X input focus without a click and saved injected input");
} finally {
  try { process.kill(-app.pid, "SIGKILL"); } catch {}
  try { if (wm) process.kill(-wm.pid, "SIGKILL"); } catch {}
  fs.closeSync(appLog);
  fs.closeSync(wmLog);
}
