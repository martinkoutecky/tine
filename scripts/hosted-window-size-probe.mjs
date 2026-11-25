// PROBE (tine#1043, never merged): does the main window's saved size drift
// across launches? Starts the built app N times with a scratch home and a
// scratch graph, waits until main is shown, closes it natively (WM_CLOSE on
// Windows, Alt+F4 under the X11 window manager on Linux), waits for the
// process to exit, and copies the saved `.window-state.json`. Then the same
// with the native-frame setting seeded on. When the app was built from the
// probe branch, TINE_PROBE_1043 makes it log its own geometry per launch.
//
// Windows runs the exe portably (a `tine-data` folder beside a copy of it), so
// the scratch home is the portable folder, as for the reporter's portable build.
// The script reports and exits 0 whether or not the height drifts.
import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const out = path.resolve(process.env.WSP_OUT || path.join(root, "test-results/hosted-window-size-probe"));
if (fs.existsSync(out) && fs.readdirSync(out).length) throw new Error(`${out} is not empty; pass a new WSP_OUT`);
fs.mkdirSync(out, { recursive: true });
const LAUNCHES = Number(process.env.WSP_LAUNCHES || 5);
const app = path.resolve(process.env.TINE_APP || path.join(root, "target/debug", process.platform === "win32" ? "tine.exe" : "tine"));
const identifier = JSON.parse(fs.readFileSync(path.join(root, "src-tauri/tauri.conf.json"), "utf8")).identifier;
// A seeded start size above the 390x480 window minimum and inside a 1024x768
// screen, so five 31 px steps cannot reach the work-area clamp. WSP_SEED=none starts from the app default.
const SEED = process.env.WSP_SEED === "none" ? null : { width: 900, height: 520, x: 60, y: 40 };
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const log = (line) => { console.log(line); fs.appendFileSync(path.join(out, "steps.log"), `${line}\n`); };

function run(command, args, extraEnv = {}) {
  const res = spawnSync(command, args, { env: { ...process.env, ...extraEnv }, encoding: "utf8", timeout: 30_000 });
  if (res.status !== 0) throw new Error(`${command} ${args.slice(0, 2).join(" ")} failed: ${res.error ?? ""}${res.stderr ?? ""}${res.stdout ?? ""}`.trim());
  return res.stdout.trim();
}

async function until(fn, ms) {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    try { const value = await fn(); if (value) return value; } catch { /* retry */ }
    await sleep(200);
  }
  return null;
}

const ps1 = path.join(out, "wsp.ps1");
fs.writeFileSync(ps1, String.raw`param([string]$Action, [string]$Arg)
Add-Type @"
using System; using System.Text; using System.Runtime.InteropServices; using System.Collections.Generic;
public class WspWin {
  public delegate bool Proc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(Proc p, IntPtr l);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint p);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  public static List<string> List(uint pid) {
    var r = new List<string>();
    EnumWindows((h, l) => { uint p; GetWindowThreadProcessId(h, out p);
      if (p == pid && IsWindowVisible(h)) { var s = new StringBuilder(512); GetWindowText(h, s, 512);
        if (s.Length > 0) r.Add(h.ToInt64() + "\t" + s); }
      return true; }, IntPtr.Zero);
    return r;
  }
}
"@
Add-Type -AssemblyName System.Windows.Forms
switch ($Action) {
  "list" { [WspWin]::List([uint32]$Arg) | ForEach-Object { $_ } }
  "close" { [void][WspWin]::PostMessage([IntPtr][long]$Arg, 0x10, [IntPtr]::Zero, [IntPtr]::Zero) }
  "info" {
    $s = [System.Windows.Forms.Screen]::PrimaryScreen
    "os=" + [Environment]::OSVersion.VersionString
    "build=" + (Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion').CurrentBuild
    "screen=" + $s.Bounds.Width + "x" + $s.Bounds.Height + " workarea=" + $s.WorkingArea.Width + "x" + $s.WorkingArea.Height
  }
}
`);
const powershell = (action, arg = "") => run("powershell", ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", ps1, action, String(arg)]);

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

function closeNatively(win) {
  if (process.platform === "win32") powershell("close", win.id);
  else { run("xdotool", ["windowactivate", "--sync", win.id]); run("xdotool", ["key", "--clearmodifiers", "alt+F4"]); }
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
const readLines = (file) => {
  try { return fs.readFileSync(file, "utf8").split(/\r?\n/).filter(Boolean).map((l) => { try { return JSON.parse(l); } catch { return { raw: l }; } }); } catch { return []; }
};

async function launch(home, dir, n) {
  const probe = path.join(dir, `probe-${n}.jsonl`);
  const child = spawn(home.exe, [], { cwd: dir, env: { ...home.env, TINE_PROBE_1043: probe }, stdio: ["ignore", fs.openSync(path.join(dir, `app-${n}.stdout`), "w"), fs.openSync(path.join(dir, `app-${n}.stderr`), "w")] });
  let exited = null;
  child.on("exit", (code, signal) => { exited = { code, signal }; });
  const record = { launch: n, stateBefore: readJson(home.statePath) };
  const main = await until(() => windows(child.pid).find((w) => /Tine/.test(w.title)), 90_000);
  if (!main) {
    record.error = "the main window never appeared";
  } else {
    record.title = main.title;
    // The probe build logs `shown`; a build without it gets a fixed settle.
    await until(() => readLines(probe).some((l) => l.label === "shown"), 15_000);
    await sleep(3000);
    closeNatively(main);
    record.closedBy = "native";
  }
  if (!(await until(() => exited, 60_000))) {
    record.closedBy = `${record.closedBy ?? "none"}, then killed after 60 s`;
    child.kill();
    await until(() => exited, 10_000);
  }
  record.exit = exited;
  record.probe = readLines(probe);
  record.state = readJson(home.statePath);
  if (record.state) fs.copyFileSync(home.statePath, path.join(dir, `window-state-${n}.json`));
  return record;
}

const host = hostInfo();
log(`host: ${JSON.stringify(host)}`);
const modes = [];
for (const [name, nativeFrame] of [["frameless", false], ["native-frame", true]]) {
  const dir = path.join(out, name);
  const home = prepareHome(dir);
  if (nativeFrame) fs.writeFileSync(home.settingsPath, JSON.stringify({ native_window_frame: true }) + "\n");
  if (SEED) fs.writeFileSync(home.statePath, JSON.stringify({ main: { ...SEED, prev_x: SEED.x, prev_y: SEED.y, maximized: false, visible: true, decorated: true, fullscreen: false } }, null, 2));
  const launches = [];
  for (let n = 1; n <= LAUNCHES; n++) {
    const record = await launch(home, dir, n);
    launches.push(record);
    const shown = record.probe.find((l) => l.label === "shown");
    log(`${name} launch ${n}: saved ${record.state?.main ? `${record.state.main.width}x${record.state.main.height}` : "none"}; shown inner ${shown?.inner_size?.join("x") ?? "?"}; exit ${JSON.stringify(record.exit)}; ${record.closedBy ?? record.error}`);
  }
  modes.push({ mode: name, nativeFrame, seededSettings: nativeFrame, launches });
}

const heights = (m) => m.launches.map((l) => l.state?.main?.height ?? "none");
const shownHeights = (m) => m.launches.map((l) => l.probe.find((x) => x.label === "shown")?.inner_size?.[1] ?? "?");
const summary = [
  `### tine#1043 window-size probe (${host.platform} ${host.build ?? host.release}, screen ${host.screen ?? "?"})`,
  "",
  `Seeded start: ${SEED ? `${SEED.width}x${SEED.height}` : "none (app default)"}. ${LAUNCHES} launches per mode, each closed natively.`,
  "",
  ...modes.map((m) => `- ${m.mode}: height per launch: ${heights(m).join(", ")} (saved); shown inner height per launch: ${shownHeights(m).join(", ")}`),
  "",
].join("\n");
fs.writeFileSync(path.join(out, "summary.md"), summary);
fs.writeFileSync(path.join(out, "result.json"), JSON.stringify({ host, modes }, null, 2));
if (process.env.GITHUB_STEP_SUMMARY) fs.appendFileSync(process.env.GITHUB_STEP_SUMMARY, summary);
console.log(summary);
