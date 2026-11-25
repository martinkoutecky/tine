// GH #1043 hosted check: a restarted main window has the size it was saved
// with, frameless (the default) and with the native frame. It reads the window
// from outside the app (GetClientRect on Windows, xdotool on Linux), so the app
// needs no logging of its own.
//
// Per frame mode, with a scratch home and graph and a seeded saved size:
//   start the app, wait until main is shown and its size has settled, read its
//   client (inner) size; resize it by a few pixels and back, as a user drags
//   an edge, so the app records its size; close it natively; read the saved
//   size. Three launches.
// It fails when a shown size differs from the size saved before that launch,
// or when a launch changes the saved size although the window ends where it
// started, or when the app does not exit after the native close (then nothing
// is saved). Each check lands in result.json; the run exits non-zero when any
// fails.
//
// Windows runs the exe portably (a `tine-data` folder beside a copy of it), so
// its scratch home is that folder. Linux uses XDG directories (run it under an
// X11 window manager, as the probe did with openbox).
import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const out = path.resolve(process.env.WS_OUT || path.join(root, "test-results/hosted-window-size"));
if (fs.existsSync(out) && fs.readdirSync(out).length) throw new Error(`${out} is not empty; pass a new WS_OUT`);
fs.mkdirSync(out, { recursive: true });
const LAUNCHES = Number(process.env.WS_LAUNCHES || 3);
const app = path.resolve(process.env.TINE_APP || path.join(root, "target/debug", process.platform === "win32" ? "tine.exe" : "tine"));
const identifier = JSON.parse(fs.readFileSync(path.join(root, "src-tauri/tauri.conf.json"), "utf8")).identifier;
// Above the 390x480 window minimum and inside a 1024x768 hosted screen with
// room for a few title bars of growth, so the work area never clamps it.
const SEED = { width: 900, height: 520, x: 60, y: 40 };
const NUDGE = 6;
// Main's title before and after its graph opens; the single-instance helper
// window and Quick Capture never match.
const MAIN_TITLE = /^Tine( Beta)?( — .*)?$/;
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const log = (line) => { console.log(line); fs.appendFileSync(path.join(out, "steps.log"), `${line}\n`); };

function run(command, args) {
  const res = spawnSync(command, args, { encoding: "utf8", timeout: 30_000 });
  if (res.status !== 0) throw new Error(`${command} ${args.slice(0, 2).join(" ")} failed: ${res.error ?? ""}${res.stderr ?? ""}${res.stdout ?? ""}`.trim());
  return res.stdout.trim();
}

async function until(fn, ms) {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    try { const value = await fn(); if (value) return value; } catch { /* retry */ }
    await sleep(250);
  }
  return null;
}

const ps1 = path.join(out, "ws.ps1");
fs.writeFileSync(ps1, String.raw`param([string]$Action, [string]$Arg, [string]$Arg2)
Add-Type @"
using System; using System.Text; using System.Runtime.InteropServices; using System.Collections.Generic;
public class WsWin {
  public delegate bool Proc(IntPtr h, IntPtr l);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
  [DllImport("user32.dll")] public static extern bool EnumWindows(Proc p, IntPtr l);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint p);
  [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  public static List<string> List(uint pid) {
    var r = new List<string>();
    EnumWindows((h, l) => { uint p; GetWindowThreadProcessId(h, out p);
      if (p == pid && IsWindowVisible(h)) { var s = new StringBuilder(512); GetWindowText(h, s, 512);
        if (s.Length > 0) r.Add(h.ToInt64() + "\t" + s); }
      return true; }, IntPtr.Zero);
    return r;
  }
  public static string Client(IntPtr h) { RECT c; GetClientRect(h, out c); return (c.Right - c.Left) + " " + (c.Bottom - c.Top); }
  // Grow or shrink the window's outer height by dy, as dragging its bottom edge does.
  public static void Nudge(IntPtr h, int dy) {
    RECT w; GetWindowRect(h, out w);
    SetWindowPos(h, IntPtr.Zero, 0, 0, w.Right - w.Left, w.Bottom - w.Top + dy, 0x0002 | 0x0004 | 0x0010);
  }
}
"@
Add-Type -AssemblyName System.Windows.Forms
switch ($Action) {
  "list" { [WsWin]::List([uint32]$Arg) | ForEach-Object { $_ } }
  "client" { [WsWin]::Client([IntPtr][long]$Arg) }
  "nudge" { [WsWin]::Nudge([IntPtr][long]$Arg, [int]$Arg2) }
  "close" { [void][WsWin]::PostMessage([IntPtr][long]$Arg, 0x10, [IntPtr]::Zero, [IntPtr]::Zero) }
  "info" {
    $s = [System.Windows.Forms.Screen]::PrimaryScreen
    "os=" + [Environment]::OSVersion.VersionString
    "build=" + (Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion').CurrentBuild
    "screen=" + $s.Bounds.Width + "x" + $s.Bounds.Height + " workarea=" + $s.WorkingArea.Width + "x" + $s.WorkingArea.Height
  }
}
`);
const powershell = (...args) => run("powershell", ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", ps1, ...args.map(String)]);

/** Visible titled windows of `pid`: [{ id, title }]. */
function windows(pid) {
  if (process.platform === "win32") {
    return powershell("list", pid).split(/\r?\n/).filter(Boolean).map((line) => {
      const [id, ...title] = line.split("\t");
      return { id, title: title.join("\t") };
    });
  }
  let ids = [];
  try { ids = run("xdotool", ["search", "--onlyvisible", "--pid", String(pid), "--name", "."]).split(/\s+/).filter(Boolean); } catch { ids = []; }
  return ids.map((id) => ({ id, title: run("xdotool", ["getwindowname", id]) }));
}

/** The window's client (inner) size: { width, height }. */
function clientSize(win) {
  if (process.platform === "win32") {
    const [width, height] = powershell("client", win.id).split(/\s+/).map(Number);
    return { width, height };
  }
  const shell = Object.fromEntries(run("xdotool", ["getwindowgeometry", "--shell", win.id]).split(/\r?\n/).map((l) => l.split("=")));
  return { width: Number(shell.WIDTH), height: Number(shell.HEIGHT) };
}

function nudge(win, dy) {
  if (process.platform === "win32") { powershell("nudge", win.id, dy); return; }
  const { width, height } = clientSize(win);
  run("xdotool", ["windowsize", win.id, String(width), String(height + dy)]);
}

function closeNatively(win) {
  if (process.platform === "win32") powershell("close", win.id);
  else { run("xdotool", ["windowactivate", "--sync", win.id]); run("xdotool", ["key", "--clearmodifiers", "alt+F4"]); }
}

/** The size once two reads a second apart agree. */
async function settledSize(win) {
  let last = null;
  return until(async () => {
    const size = clientSize(win);
    const same = last && last.width === size.width && last.height === size.height;
    last = size;
    if (!same) await sleep(750);
    return same ? size : null;
  }, 20_000);
}

function hostInfo() {
  const info = { platform: process.platform, release: os.release(), app, identifier, seed: SEED };
  try {
    if (process.platform === "win32") for (const line of powershell("info").split(/\r?\n/)) { const [k, ...v] = line.split("="); info[k] = v.join("="); }
    else info.screen = run("xdotool", ["getdisplaygeometry"]);
  } catch (error) { info.infoError = String(error.message ?? error); }
  return info;
}

/** The scratch home of one mode: where the app keeps settings and window state. */
function prepareHome(dir) {
  const graph = path.join(dir, "graph");
  fs.mkdirSync(path.join(graph, "pages"), { recursive: true });
  fs.mkdirSync(path.join(graph, "journals"), { recursive: true });
  fs.writeFileSync(path.join(graph, "pages", "Alpha.md"), "- alpha one\n- alpha two\n");
  const env = { ...process.env, TINE_GRAPH: graph, TINE_GPU: "0" };
  let exe = app;
  let config, data;
  if (process.platform === "win32") {
    // Portable: a copy of the exe with `tine-data` beside it (portable.rs).
    const appDir = path.join(dir, "app");
    fs.mkdirSync(path.join(appDir, "tine-data"), { recursive: true });
    exe = path.join(appDir, path.basename(app));
    fs.copyFileSync(app, exe);
    config = data = path.join(appDir, "tine-data", identifier, "roaming");
  } else {
    for (const sub of ["data", "config", "cache", "state"]) fs.mkdirSync(path.join(dir, sub), { recursive: true });
    Object.assign(env, { XDG_DATA_HOME: path.join(dir, "data"), XDG_CONFIG_HOME: path.join(dir, "config"), XDG_CACHE_HOME: path.join(dir, "cache"), XDG_STATE_HOME: path.join(dir, "state") });
    config = path.join(dir, "config", identifier);
    data = path.join(dir, "data", identifier);
  }
  fs.mkdirSync(config, { recursive: true });
  fs.mkdirSync(data, { recursive: true });
  return { env, exe, statePath: path.join(config, ".window-state.json"), settingsPath: path.join(data, "tine-settings.json") };
}

const readJson = (file) => { try { return JSON.parse(fs.readFileSync(file, "utf8")); } catch { return null; } };
const savedSize = (state) => (state?.main ? { width: state.main.width, height: state.main.height } : null);
const fmt = (size) => (size ? `${size.width}x${size.height}` : "none");
const same = (a, b) => Boolean(a && b && a.width === b.width && a.height === b.height);

const checks = [];
function check(name, pass, detail) {
  checks.push({ name, pass, detail });
  log(`${pass ? "PASS" : "FAIL"} ${name}: ${detail}`);
}

async function launch(home, dir, mode, n) {
  const name = `${mode} launch ${n}`;
  const child = spawn(home.exe, [], { cwd: dir, env: home.env, stdio: ["ignore", fs.openSync(path.join(dir, `app-${n}.stdout`), "w"), fs.openSync(path.join(dir, `app-${n}.stderr`), "w")] });
  let exited = null;
  child.on("exit", (code, signal) => { exited = { code, signal }; });
  const record = { launch: n, savedBefore: savedSize(readJson(home.statePath)) };
  const main = await until(() => windows(child.pid).find((w) => MAIN_TITLE.test(w.title)), 90_000);
  if (!main) {
    check(`${name}: main window`, false, "the main window never appeared");
  } else {
    record.title = main.title;
    record.shown = await settledSize(main);
    check(`${name}: shown size is the saved size`, same(record.shown, record.savedBefore), `saved ${fmt(record.savedBefore)}, shown ${fmt(record.shown)}`);
    // A user's resize, undone: the app records the size it then has.
    nudge(main, NUDGE);
    await sleep(1000);
    record.nudged = clientSize(main);
    nudge(main, -NUDGE);
    await sleep(1000);
    record.afterNudge = clientSize(main);
    record.nudgeMoved = !same(record.nudged, record.shown) && same(record.afterNudge, record.shown);
    log(`${name}: resized to ${fmt(record.nudged)} and back to ${fmt(record.afterNudge)}`);
    closeNatively(main);
  }
  record.exitedNatively = Boolean(await until(() => exited, 60_000));
  if (!record.exitedNatively) {
    child.kill();
    await until(() => exited, 10_000);
  }
  record.exit = exited;
  check(`${name}: exits after the native close`, record.exitedNatively, record.exitedNatively ? JSON.stringify(exited) : "still running after 60 s, killed (nothing saved)");
  const state = readJson(home.statePath);
  if (state) fs.copyFileSync(home.statePath, path.join(dir, `window-state-${n}.json`));
  record.savedAfter = savedSize(state);
  if (record.nudgeMoved) {
    check(`${name}: saved size unchanged by a resize and back`, same(record.savedAfter, record.savedBefore), `saved before ${fmt(record.savedBefore)}, after ${fmt(record.savedAfter)}`);
  } else if (main) {
    check(`${name}: resize and back`, false, `the window did not resize and return (shown ${fmt(record.shown)}, resized ${fmt(record.nudged)}, back ${fmt(record.afterNudge)})`);
  }
  return record;
}

const host = hostInfo();
log(`host: ${JSON.stringify(host)}`);
const modes = [];
for (const [mode, nativeFrame] of [["frameless", false], ["native-frame", true]]) {
  const dir = path.join(out, mode);
  const home = prepareHome(dir);
  if (nativeFrame) fs.writeFileSync(home.settingsPath, JSON.stringify({ native_window_frame: true }) + "\n");
  fs.writeFileSync(home.statePath, JSON.stringify({ main: { ...SEED, prev_x: SEED.x, prev_y: SEED.y, maximized: false, visible: true, decorated: true, fullscreen: false } }, null, 2));
  const launches = [];
  for (let n = 1; n <= LAUNCHES; n++) launches.push(await launch(home, dir, mode, n));
  modes.push({ mode, nativeFrame, launches });
}

const failed = checks.filter((c) => !c.pass);
const summary = [
  `### GH #1043 window size across restarts (${host.platform} ${host.build ?? host.release}, screen ${host.screen ?? "?"})`,
  "",
  `Seeded saved size ${fmt(SEED)}; ${LAUNCHES} launches per mode, each resized by ${NUDGE} px and back, then closed natively.`,
  "",
  ...modes.map((m) => `- ${m.mode}: saved before each launch ${m.launches.map((l) => fmt(l.savedBefore)).join(", ")}; shown ${m.launches.map((l) => fmt(l.shown)).join(", ")}; saved after the last ${fmt(m.launches.at(-1)?.savedAfter)}`),
  "",
  failed.length ? `**${failed.length} check(s) failed:**\n${failed.map((c) => `- ${c.name}: ${c.detail}`).join("\n")}` : `All ${checks.length} checks passed.`,
  "",
].join("\n");
fs.writeFileSync(path.join(out, "summary.md"), summary);
fs.writeFileSync(path.join(out, "result.json"), JSON.stringify({ host, checks, modes }, null, 2));
if (process.env.GITHUB_STEP_SUMMARY) fs.appendFileSync(process.env.GITHUB_STEP_SUMMARY, summary);
console.log(summary);
process.exit(failed.length ? 1 : 0);
