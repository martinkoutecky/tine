// OG-MULTIWINDOW hosted evidence: open / type / save / main-minimized / close on
// Linux, Windows and macOS. macOS has no WebDriver and the Linux one cannot drive
// a popup's keyboard (see scripts/e2e-multiwindow.mjs), so this driver uses only
// OS input (xdotool, SendKeys via PowerShell, System Events) and observes the
// page file on disk. It never reads a private graph or app data: the app runs
// against a fixture graph copied under test-results/.
//
// Each check lands in result.json as pass/fail with its detail; the run exits
// non-zero when any check fails, so a hosted job is red exactly when a platform
// does not deliver the journey.
import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const out = path.resolve(process.env.MW_OUT || path.join(root, "test-results/hosted-multiwindow"));
const graph = path.join(out, "graph");
fs.rmSync(out, { recursive: true, force: true });
fs.mkdirSync(path.join(graph, "pages"), { recursive: true });
fs.mkdirSync(path.join(graph, "journals"), { recursive: true });
const PAGE = path.join(graph, "pages", "Alpha.md");
fs.writeFileSync(PAGE, "- alpha one\n- alpha two\n- alpha three\n");
const env = { ...process.env, TINE_GRAPH: graph, TINE_GPU: "0" };
if (process.platform === "linux") {
  for (const dir of ["data", "config", "cache", "state"]) fs.mkdirSync(path.join(out, dir), { recursive: true });
  Object.assign(env, { XDG_DATA_HOME: path.join(out, "data"), XDG_CONFIG_HOME: path.join(out, "config"), XDG_CACHE_HOME: path.join(out, "cache"), XDG_STATE_HOME: path.join(out, "state") });
}
const app = path.resolve(process.env.TINE_APP || path.join(root, "target/debug", process.platform === "win32" ? "tine.exe" : "tine"));
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const results = {};
const log = (line) => { console.log(line); fs.appendFileSync(path.join(out, "steps.log"), `${line}\n`); };

function run(command, args, extraEnv = {}) {
  const res = spawnSync(command, args, { env: { ...env, ...extraEnv }, encoding: "utf8", timeout: 30_000 });
  if (res.status !== 0) throw new Error(`${command} ${args.slice(0, 2).join(" ")} failed: ${res.error ?? ""}${res.stderr ?? ""}${res.stdout ?? ""}`.trim());
  return res.stdout.trim();
}

async function until(fn, ms, message) {
  const end = Date.now() + ms;
  let last;
  while (Date.now() < end) {
    try { const value = await fn(); if (value) return value; } catch (error) { last = error; }
    await sleep(200);
  }
  throw new Error(`${message}${last ? ` (${last.message ?? last})` : ""}`);
}

const child = spawn(app, [], { cwd: out, env, stdio: ["ignore", fs.openSync(path.join(out, "app.stdout"), "w"), fs.openSync(path.join(out, "app.stderr"), "w")] });
let exited = null;
child.on("exit", (code, signal) => { exited = { code, signal }; });
const alive = () => exited === null;

// ---- one OS-input layer per platform: windows are named by their titles ----
const ps1 = path.join(out, "mw.ps1");
fs.writeFileSync(ps1, String.raw`param([string]$Action, [string]$Arg)
Add-Type @"
using System; using System.Text; using System.Runtime.InteropServices; using System.Collections.Generic;
public class MwWin {
  public delegate bool Proc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(Proc p, IntPtr l);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint p);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int c);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  public static List<string> List(uint pid) {
    var r = new List<string>();
    EnumWindows((h, l) => { uint p; GetWindowThreadProcessId(h, out p);
      if (p == pid && IsWindowVisible(h)) { var s = new StringBuilder(512); GetWindowText(h, s, 512);
        if (s.Length > 0) r.Add(h.ToInt64() + "\t" + (IsIconic(h) ? "1" : "0") + "\t" + s); }
      return true; }, IntPtr.Zero);
    return r;
  }
}
"@
Add-Type -AssemblyName System.Windows.Forms
switch ($Action) {
  "list" { [MwWin]::List([uint32]$Arg) | ForEach-Object { $_ } }
  "activate" { $w = New-Object -ComObject WScript.Shell; if (-not $w.AppActivate($env:MW_TITLE)) { throw "activate failed" }; Start-Sleep -Milliseconds 400 }
  "minimize" { [void][MwWin]::ShowWindow([IntPtr][long]$Arg, 6) }
  "close" { [void][MwWin]::PostMessage([IntPtr][long]$Arg, 0x10, [IntPtr]::Zero, [IntPtr]::Zero) }
  "keys" { [System.Windows.Forms.SendKeys]::SendWait($env:MW_KEYS) }
}
`);
const powershell = (action, arg = "", extra = {}) => run("powershell", ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", ps1, action, String(arg)], extra);
const osa = (script, extra = {}) => run("osascript", ["-e", script], extra);
const MAC_PROC = `(first process whose unix id is ${child.pid})`;

/** Visible titled windows of the app: [{ id, title, minimized }]. */
function windows() {
  if (process.platform === "linux") {
    let ids = [];
    try { ids = run("xdotool", ["search", "--onlyvisible", "--pid", String(child.pid), "--name", "."]).split(/\s+/).filter(Boolean); } catch { ids = []; }
    return ids.map((id) => ({ id, title: run("xdotool", ["getwindowname", id]), minimized: false }));
  }
  if (process.platform === "win32") {
    return powershell("list", child.pid).split(/\r?\n/).filter(Boolean).map((line) => {
      const [id, iconic, ...title] = line.split("\t");
      return { id, title: title.join("\t"), minimized: iconic === "1" };
    });
  }
  const names = osa(`tell application "System Events" to tell ${MAC_PROC} to get name of every window`);
  if (!names) return [];
  return names.split(", ").filter((title) => title && title !== "Quick Capture").map((title) => ({ id: title, title, minimized: false }));
}

/** Whether `win` still exists, minimized or not (xdotool's visible-only search
 * skips a minimized X11 window; Win32 and System Events list it). */
function exists(win) {
  if (process.platform === "linux") {
    try { run("xdotool", ["getwindowname", win.id]); return true; } catch { return false; }
  }
  if (process.platform === "darwin") return Boolean(current(win, false));
  return windows().some((w) => w.id === win.id);
}

/** System Events names a window only by its title, and both titles move after
 * launch (main: "Tine" -> "Tine — <graph>"; the new window takes its page's
 * title), so on macOS a window is re-found by its role before every action. */
const MAIN_TITLE = /^Tine( Beta)?( — .*)?$/;
function current(win, required = true) {
  if (process.platform !== "darwin" || !win.role) return win;
  const found = windows().find((w) => (win.role === "main" ? MAIN_TITLE.test(w.title) : !MAIN_TITLE.test(w.title)));
  if (!found && required) throw new Error(`no ${win.role} window among ${JSON.stringify(windows().map((w) => w.title))}`);
  if (found) { win.id = found.id; win.title = found.title; }
  return found;
}

function activate(win) {
  current(win);
  if (process.platform === "linux") run("xdotool", ["windowactivate", "--sync", win.id]);
  else if (process.platform === "win32") powershell("activate", "", { MW_TITLE: win.title });
  else osa(`tell application "System Events" to tell ${MAC_PROC}
  set frontmost to true
  perform action "AXRaise" of window (system attribute "MW_TITLE")
end tell`, { MW_TITLE: win.title });
}

/** A chord over mod (Ctrl, or Cmd on macOS) [+ shift] and one letter. */
function chord(letter, shift = false) {
  if (process.platform === "linux") run("xdotool", ["key", "--clearmodifiers", `ctrl+${shift ? "shift+" : ""}${letter}`]);
  else if (process.platform === "win32") powershell("keys", "", { MW_KEYS: `^${shift ? "+" : ""}${letter}` });
  else osa(`tell application "System Events" to keystroke "${letter}" using {command down${shift ? ", shift down" : ""}}`);
}

function typeText(text) {
  if (process.platform === "linux") run("xdotool", ["type", "--clearmodifiers", "--delay", "60", text]);
  else if (process.platform === "win32") powershell("keys", "", { MW_KEYS: text });
  else osa(`tell application "System Events" to keystroke (system attribute "MW_KEYS")`, { MW_KEYS: text });
}

function press(key) {
  const linux = { enter: "Return", escape: "Escape", down: "Down" }[key];
  const win = { enter: "{ENTER}", escape: "{ESC}", down: "{DOWN}" }[key];
  const mac = { enter: 36, escape: 53, down: 125 }[key];
  if (process.platform === "linux") run("xdotool", ["key", "--clearmodifiers", linux]);
  else if (process.platform === "win32") powershell("keys", "", { MW_KEYS: win });
  else osa(`tell application "System Events" to key code ${mac}`);
}

function minimize(win) {
  current(win);
  if (process.platform === "linux") run("xdotool", ["windowminimize", "--sync", win.id]);
  else if (process.platform === "win32") powershell("minimize", win.id);
  else osa(`tell application "System Events" to tell ${MAC_PROC} to set value of attribute "AXMinimized" of window (system attribute "MW_TITLE") to true`, { MW_TITLE: win.title });
}

/** The window manager's own close (title-bar button / WM_CLOSE / AXCloseButton). */
function closeNatively(win) {
  current(win);
  if (process.platform === "linux") { run("xdotool", ["windowactivate", "--sync", win.id]); run("xdotool", ["key", "--clearmodifiers", "alt+F4"]); }
  else if (process.platform === "win32") powershell("close", win.id);
  else osa(`tell application "System Events" to tell ${MAC_PROC} to click (first button of window (system attribute "MW_TITLE") whose subrole is "AXCloseButton")`, { MW_TITLE: win.title });
}

function screenshot(name) {
  try {
    if (process.platform === "linux") run("import", ["-window", "root", path.join(out, `${name}.png`)]);
    else if (process.platform === "darwin") run("screencapture", ["-x", path.join(out, `${name}.png`)]);
    else run("powershell", ["-NoProfile", "-Command", `Add-Type -AssemblyName System.Windows.Forms; Add-Type -AssemblyName System.Drawing; $r=[System.Windows.Forms.SystemInformation]::VirtualScreen; $b=New-Object System.Drawing.Bitmap($r.Width,$r.Height); $g=[System.Drawing.Graphics]::FromImage($b); $g.CopyFromScreen($r.Left,$r.Top,0,0,$r.Size); $b.Save('${path.join(out, `${name}.png`).replaceAll("'", "''")}'); $g.Dispose(); $b.Dispose()`]);
  } catch (error) { log(`screenshot ${name} failed: ${error.message}`); }
}

const disk = () => fs.readFileSync(PAGE, "utf8");

/** Open the first block's editor from the keyboard in the active window:
 * Escape enters pane select, Enter returns to the pane selecting its first
 * visible block, and Enter on a selected block edits it (caret at the end). */
async function editFirstBlock() {
  press("escape");
  await sleep(500);
  debugShot("pane-select");
  press("enter");
  await sleep(500);
  debugShot("first-selected");
  press("enter");
  await sleep(500);
  debugShot("editing");
}

/** After Escape left the edited block selected, edit the next block down. */
async function editNextBlock() {
  press("escape");
  await sleep(400);
  debugShot("selected");
  press("down");
  await sleep(400);
  press("enter");
  await sleep(500);
  debugShot("editing-next");
}

/** Back in the new window after its edit ended elsewhere: nothing is being
 * edited, so the same keyboard path as the first edit applies, one block on. */
async function editBlockAfterReturn() {
  press("escape");
  await sleep(500);
  press("enter");
  await sleep(500);
  debugShot("return-selected");
  press("down");
  await sleep(400);
  press("enter");
  await sleep(500);
  debugShot("return-editing");
}

let debugCount = 0;
const debugShot = (name) => { if (process.env.MW_DEBUG) screenshot(`dbg-${String(++debugCount).padStart(2, "0")}-${name}`); };

async function check(id, fn) {
  try {
    const detail = await fn();
    results[id] = { status: "pass", detail };
    log(`pass ${id}: ${detail}`);
  } catch (error) {
    results[id] = { status: "fail", detail: String(error.message ?? error) };
    log(`FAIL ${id}: ${error.message ?? error}`);
    screenshot(`fail-${id}`);
  }
}

let main;
let popup;
await check("launch", async () => {
  const wins = await until(() => { const list = windows(); return list.length ? list : null; }, 90_000, "the main window never appeared");
  main = await until(() => windows().find((w) => /Tine/.test(w.title)), 30_000, `no main window titled Tine (${JSON.stringify(wins)})`);
  main.role = "main";
  await sleep(4000); // the graph's first paint; nothing below depends on it beyond keyboard routing
  return `main window "${main.title}"`;
});

await check("open", async () => {
  if (!main) throw new Error("no main window");
  const before = new Set(windows().map((w) => w.id));
  activate(main);
  chord("k");
  await sleep(700);
  typeText("Alpha");
  await sleep(900);
  press("enter");
  await sleep(1500);
  chord("p", true);
  await sleep(700);
  typeText("Open current page in new window");
  await sleep(900);
  press("enter");
  const isNew = (w) => (process.platform === "darwin" ? !MAIN_TITLE.test(w.title) : !before.has(w.id));
  popup = await until(() => windows().find(isNew), 20_000, `no new window appeared (${JSON.stringify(windows())})`);
  popup.role = "popup";
  await sleep(1500);
  popup = Object.assign(popup, windows().find((w) => w.id === popup.id) ?? {});
  current(popup);
  current(main);
  screenshot("01-open");
  return `new window "${popup.title}"${popup.title.startsWith("Alpha") ? " (titled after its page)" : " (page title not mirrored natively)"}`;
});

await check("type-save", async () => {
  if (!popup) throw new Error("no new window");
  activate(popup);
  await editFirstBlock();
  typeText(" hostedone");
  await until(() => disk().includes("hostedone"), 15_000, `text typed in the new window never reached disk: ${JSON.stringify(disk())}`);
  return "text typed in the new window was saved to the page file";
});

await check("main-minimized", async () => {
  if (!popup || !main) throw new Error("no windows");
  // The user's path: go to main (focusing another Tine window ends the new
  // window's edit by design), minimize it, come back and edit the next block.
  activate(main);
  await sleep(800);
  debugShot("main-active");
  minimize(main);
  await sleep(1500);
  log(`after minimizing main: ${JSON.stringify(windows())}`);
  activate(popup);
  await sleep(500);
  debugShot("popup-back");
  await editBlockAfterReturn();
  typeText(" minimizedtwo");
  await until(() => disk().includes("minimizedtwo"), 15_000, `with main minimized, text typed in the new window never reached disk: ${JSON.stringify(disk())}`);
  const state = windows().find((w) => w.id === main.id);
  return `saved while main was minimized${state?.minimized ? " (main reported iconic)" : ""}`;
});

await check("close", async () => {
  if (!popup) throw new Error("no new window");
  activate(popup);
  await editNextBlock();
  typeText(" closingthree");
  closeNatively(popup);
  await until(() => !exists(popup), 15_000, "the new window did not close");
  await until(() => disk().includes("closingthree"), 15_000, `text typed just before the native close never reached disk: ${JSON.stringify(disk())}`);
  await sleep(1500);
  if (!alive()) throw new Error(`the app exited when the new window closed (${JSON.stringify(exited)})`);
  if (!exists(main)) throw new Error("main window vanished with the new window");
  screenshot("02-after-close");
  return "text typed just before the native close was saved; the window closed and main stayed open";
});

if (alive()) child.kill();
await sleep(500);
fs.writeFileSync(path.join(out, "page-final.md"), disk());
fs.writeFileSync(path.join(out, "result.json"), JSON.stringify({ platform: process.platform, results }, null, 2));
console.log(JSON.stringify(results, null, 2));
if (Object.values(results).some((r) => r.status !== "pass")) process.exitCode = 1;
