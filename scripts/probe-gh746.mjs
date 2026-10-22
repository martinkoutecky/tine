// PROBE (throwaway branches probe/gh746-*, never merge). GH #746: periodic typing freezes on a
// large graph. Opens a copy of a generated graph with the GH #248 reporter page installed as
// "GH746 typing" (40 linked + 156 unlinked mentions elsewhere), holds a key at ~30/s for ~11 s and
// then holds Backspace, and records in the page: every keydown/input (arrival, handler time),
// every animation frame, long tasks (Chromium), every IPC call and page-mail event. On Windows
// it also drives OS-level input (SendInput) and probes the host UI thread with WM_NULL.
// Env: PROBE_GRAPH (source graph dir), PROBE_LABEL, PROBE_TARGET_MD, PROBE_MODES (os,webdriver),
// PROBE_KEYS (330), PROBE_DELAY_MS (33), PROBE_SETTLE_MS (30000), PROBE_OUT.
import { spawn, spawnSync } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { startWebdriverApplication, stopWebdriverApplication, tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const WIN = process.platform === "win32";
const APP = process.env.TINE_APP || path.join(ROOT, WIN ? "target/release/tine.exe" : "target/release/tine");
const TD = process.env.TAURI_DRIVER || "tauri-driver";
const DRIVER = Number(process.env.E2E_DRIVER_PORT || 4612);
const NATIVE = Number(process.env.E2E_NATIVE_PORT || 4613);
const LABEL = process.env.PROBE_LABEL || "run";
const SRC = process.env.PROBE_GRAPH;
const TARGET_MD = process.env.PROBE_TARGET_MD || path.join(ROOT, "tests/fixtures/gh746/GH746 typing.md");
const MODES = (process.env.PROBE_MODES || (WIN ? "os,webdriver" : "os,webdriver")).split(",");
const KEYS = Number(process.env.PROBE_KEYS || 330);
const DELAY = Number(process.env.PROBE_DELAY_MS || 33);
const SETTLE = Number(process.env.PROBE_SETTLE_MS || 30000);
const OUT = path.resolve(process.env.PROBE_OUT || path.join(ROOT, "test-results/probe-gh746"), LABEL);
const TMP = path.join(process.env.PROBE_TMP || os.tmpdir(), `tine-gh746-${LABEL}-${process.pid}`);
const GRAPH = path.join(TMP, "graph");
fs.mkdirSync(OUT, { recursive: true });
fs.rmSync(TMP, { recursive: true, force: true });
if (!SRC) throw new Error("PROBE_GRAPH is required");
fs.cpSync(SRC, GRAPH, { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(`${TMP}/xdg/${dir}`, { recursive: true });

// The target page and its mentions (GH #248 comment 3: 40 linked, 156 unlinked references).
{
  fs.writeFileSync(path.join(GRAPH, "pages", "GH746 typing.md"), fs.readFileSync(TARGET_MD));
  const pages = fs.readdirSync(path.join(GRAPH, "pages")).filter((f) => f.endsWith(".md") && f !== "GH746 typing.md").sort();
  let seed = 746;
  const rnd = () => ((seed = (seed * 1103515245 + 12345) % 2147483648) / 2147483648);
  const picked = new Set();
  while (picked.size < Math.min(196, pages.length)) picked.add(pages[Math.floor(rnd() * pages.length)]);
  [...picked].forEach((f, i) => {
    const file = path.join(GRAPH, "pages", f);
    const text = fs.readFileSync(file, "utf8");
    const line = i < 40 ? `- mention ${i} of [[GH746 typing]] here\n` : `- plain mention ${i} of GH746 typing here\n`;
    fs.writeFileSync(file, text + (text.endsWith("\n") || text === "" ? "" : "\n") + line);
  });
}

const env = { ...process.env, TINE_GRAPH: GRAPH, XDG_DATA_HOME: `${TMP}/xdg/data`, XDG_CONFIG_HOME: `${TMP}/xdg/config`, XDG_CACHE_HOME: `${TMP}/xdg/cache`,
  APPDATA: path.join(TMP, "appdata"), LOCALAPPDATA: path.join(TMP, "localappdata"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
fs.mkdirSync(env.APPDATA, { recursive: true }); fs.mkdirSync(env.LOCALAPPDATA, { recursive: true });
if (WIN && process.env.CI === "true") spawnSync("taskkill", ["/IM", path.basename(APP), "/T", "/F"], { stdio: "ignore" });
const launchedAt = Date.now();
const webviewTarget = await startWebdriverApplication(APP, env, NATIVE);
const log = fs.openSync(path.join(OUT, "tauri-driver.log"), "w");
const driver = spawn(TD, webdriverServerArgs(DRIVER, NATIVE, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"),
  { env: webviewTarget.env, stdio: ["ignore", log, log], detached: !WIN });
await sleep(2500);

// In-page instrumentation. Arrays of numbers only, so the log itself costs little.
const INSTRUMENT = () => {
  if (window.__p746) return "already";
  const P = window.__p746 = { keys: [], inputs: [], frames: [], lt: [], ipc: [], mail: [], mut: [], epoch0: Date.now() - performance.now(), on: true };
  window.addEventListener("keydown", (e) => { if (P.on) P.keys.push([performance.now(), e.timeStamp, e.key.length === 1 ? e.key.charCodeAt(0) : e.key === "Backspace" ? 8 : -1, e.isTrusted ? 1 : 0]); }, true);
  let current = null;
  window.addEventListener("input", (e) => { if (!P.on) return; current = [performance.now(), e.timeStamp, 0, 0]; P.inputs.push(current);
    const rec = current; requestAnimationFrame(() => { rec[3] = performance.now(); }); }, true);
  window.addEventListener("input", () => { if (current) { current[2] = performance.now(); current = null; } }, false);
  const frame = (t) => { P.frames.push(performance.now()); requestAnimationFrame(frame); };
  requestAnimationFrame(frame);
  try { new PerformanceObserver((list) => { for (const e of list.getEntries()) P.lt.push([e.startTime, e.duration]); }).observe({ type: "longtask", buffered: true }); } catch { P.lt = null; }
  const I = window.__TAURI_INTERNALS__;
  const orig = I.invoke.bind(I);
  // `invoke` is a non-writable property; Tauri's IPC goes through window.fetch at call time.
  const origFetch = window.fetch.bind(window);
  window.fetch = function (input, init) {
    const url = typeof input === "string" ? input : input?.url ?? "";
    if (!/^(ipc:|https?:\/\/ipc\.localhost)/.test(url)) return origFetch(input, init);
    const cmd = decodeURIComponent(url.split("/").pop() || "?");
    const rec = [cmd, performance.now(), -1, 0];
    P.ipc.push(rec);
    const p = origFetch(input, init);
    p.then(() => { rec[2] = performance.now(); }, () => { rec[2] = performance.now(); rec[3] = 1; });
    return p;
  };
  const h = I.transformCallback((ev) => {
    let size = 0; try { size = JSON.stringify(ev?.payload ?? null).length; } catch {}
    P.mail.push([performance.now(), size]);
  });
  orig("plugin:event|listen", { event: "page-mail", target: { kind: "Any" }, handler: h }).catch(() => {});
  new MutationObserver((records) => { let n = 0; for (const r of records) n += r.addedNodes.length + r.removedNodes.length; P.mut.push([performance.now(), n]); })
    .observe(document.body, { childList: true, subtree: true });
  return "installed";
};

const drain = () => {
  const P = window.__p746;
  const out = { keys: P.keys, inputs: P.inputs, frames: P.frames, lt: P.lt, ipc: P.ipc, mail: P.mail, mut: P.mut, epoch0: P.epoch0, now: performance.now() };
  P.keys = []; P.inputs = []; P.frames = []; P.lt = P.lt ? [] : null; P.ipc = P.ipc.filter((r) => r[2] < 0); P.mail = []; P.mut = [];
  return JSON.stringify(out);
};

function quant(xs, q) { if (!xs.length) return null; const s = [...xs].sort((a, b) => a - b); return +s[Math.min(s.length - 1, Math.ceil(s.length * q) - 1)].toFixed(1); }

function analyse(raw, win) {
  const t0 = raw.keys.length ? raw.keys[0][0] : raw.inputs[0]?.[0] ?? 0;
  const inputs = raw.inputs.map((r) => r[0]);
  const gaps = inputs.slice(1).map((t, i) => t - inputs[i]);
  const stalls = [];
  for (let i = 1; i < inputs.length; i++) if (inputs[i] - inputs[i - 1] > 150) stalls.push({ at: +(inputs[i - 1] - t0).toFixed(0), gap: +(inputs[i] - inputs[i - 1]).toFixed(0) });
  const frames = raw.frames;
  const fgaps = frames.slice(1).map((t, i) => t - frames[i]);
  const bigFrames = [];
  for (let i = 1; i < frames.length; i++) if (frames[i] - frames[i - 1] > 100) bigFrames.push({ at: +(frames[i - 1] - t0).toFixed(0), gap: +(frames[i] - frames[i - 1]).toFixed(0) });
  const handler = raw.inputs.filter((r) => r[2]).map((r) => r[2] - r[0]);
  const toFrame = raw.inputs.filter((r) => r[3]).map((r) => r[3] - r[0]);
  const queue = raw.inputs.map((r) => r[0] - r[1]).filter((x) => x >= 0 && x < 60000);
  const ipcBy = {};
  for (const [cmd, s, e] of raw.ipc) { if (s < t0 - 50) continue; const b = (ipcBy[cmd] ??= { n: 0, ms: [] }); b.n++; if (e > 0) b.ms.push(e - s); }
  for (const b of Object.values(ipcBy)) { b.p50 = quant(b.ms, 0.5); b.max = b.ms.length ? +Math.max(...b.ms).toFixed(1) : null; delete b.ms; }
  // What happened around each stall: IPC calls started in the 1.5 s before it ended, mail, mutations.
  for (const st of stalls) {
    const a = t0 + st.at - 1500, b = t0 + st.at + st.gap;
    st.ipc = raw.ipc.filter((r) => r[1] >= a && r[1] <= b).map((r) => `${r[0]}@${(r[1] - t0).toFixed(0)}+${r[2] > 0 ? (r[2] - r[1]).toFixed(0) : "?"}`);
    st.mail = raw.mail.filter((m) => m[0] >= a && m[0] <= b).map((m) => `${(m[0] - t0).toFixed(0)}:${m[1]}`);
    st.mut = raw.mut.filter((m) => m[0] >= a && m[0] <= b).reduce((s, m) => s + m[1], 0);
    st.lt = (raw.lt ?? []).filter((l) => l[0] >= a && l[0] <= b).map((l) => `${(l[0] - t0).toFixed(0)}+${l[1].toFixed(0)}`);
    if (win) st.ui = win.filter((w) => w.at >= raw.epoch0 + a && w.at <= raw.epoch0 + b).map((w) => `${(w.at - raw.epoch0 - t0).toFixed(0)}+${w.ms}`);
  }
  const stallStarts = stalls.map((s) => s.at);
  return {
    keydowns: raw.keys.length, trusted: raw.keys.filter((k) => k[3]).length, inputs: inputs.length,
    spanMs: inputs.length ? +(inputs.at(-1) - inputs[0]).toFixed(0) : 0,
    inputGapMs: { p50: quant(gaps, 0.5), p95: quant(gaps, 0.95), p99: quant(gaps, 0.99), max: quant(gaps, 1) },
    gapsOver: { 100: gaps.filter((g) => g > 100).length, 200: gaps.filter((g) => g > 200).length, 500: gaps.filter((g) => g > 500).length },
    frameGapMs: { p50: quant(fgaps, 0.5), p99: quant(fgaps, 0.99), max: quant(fgaps, 1), over50: fgaps.filter((g) => g > 50).length, over100: fgaps.filter((g) => g > 100).length },
    handlerMs: { p50: quant(handler, 0.5), p95: quant(handler, 0.95), max: quant(handler, 1) },
    inputToFrameMs: { p50: quant(toFrame, 0.5), p95: quant(toFrame, 0.95), max: quant(toFrame, 1) },
    eventQueueMs: { p50: quant(queue, 0.5), p95: quant(queue, 0.95), max: quant(queue, 1) },
    longTasks: raw.lt ? raw.lt.filter((l) => l[0] >= t0).map((l) => `${(l[0] - t0).toFixed(0)}+${l[1].toFixed(0)}`) : "unsupported",
    stallIntervalsMs: stallStarts.slice(1).map((t, i) => t - stallStarts[i]),
    stalls, bigFrames, ipc: ipcBy, mails: raw.mail.length, mailBytes: raw.mail.reduce((s, m) => s + m[1], 0),
    uiThread: win ? { probes: win.length, over50: win.filter((w) => w.ms > 50).length, max: win.reduce((m, w) => Math.max(m, w.ms), 0), slow: win.filter((w) => w.ms > 50).map((w) => `${(w.at - raw.epoch0 - t0).toFixed(0)}+${w.ms}`) } : undefined,
  };
}

// Windows OS-level input + UI-thread probe (one C# helper, compiled by PowerShell).
const WIN_HELPER = String.raw`
param([int]$procId, [int]$count, [int]$delay, [int]$vk, [string]$out)
Add-Type -TypeDefinition @"
using System; using System.Diagnostics; using System.Runtime.InteropServices; using System.Threading; using System.Text; using System.Collections.Generic;
public static class P746 {
  [StructLayout(LayoutKind.Sequential)] struct KEYBDINPUT { public ushort wVk; public ushort wScan; public uint dwFlags; public uint time; public IntPtr dwExtraInfo; }
  [StructLayout(LayoutKind.Sequential)] struct MOUSEINPUT { public int dx; public int dy; public uint mouseData; public uint dwFlags; public uint time; public IntPtr dwExtraInfo; }
  [StructLayout(LayoutKind.Explicit)] struct UNION { [FieldOffset(0)] public MOUSEINPUT mi; [FieldOffset(0)] public KEYBDINPUT ki; }
  [StructLayout(LayoutKind.Sequential)] struct INPUT { public uint type; public UNION u; }
  [DllImport("user32.dll", SetLastError=true)] static extern uint SendInput(uint n, INPUT[] i, int size);
  [DllImport("user32.dll")] static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] static extern bool ShowWindow(IntPtr h, int c);
  [DllImport("user32.dll")] static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
  [DllImport("user32.dll", SetLastError=true)] static extern IntPtr SendMessageTimeout(IntPtr h, uint msg, UIntPtr w, IntPtr l, uint flags, uint timeout, out UIntPtr result);
  [DllImport("winmm.dll")] static extern uint timeBeginPeriod(uint p);
  static long Epoch() { return (long)(DateTime.UtcNow - new DateTime(1970,1,1,0,0,0,DateTimeKind.Utc)).TotalMilliseconds; }
  static INPUT Key(ushort vk, bool up) { var i = new INPUT(); i.type = 1; i.u.ki.wVk = vk; i.u.ki.dwFlags = up ? 2u : 0u; return i; }
  public static string Run(int pid, int count, int delay, int vk) {
    timeBeginPeriod(1);
    var proc = Process.GetProcessById(pid); IntPtr h = proc.MainWindowHandle;
    var sb = new StringBuilder();
    keybd_event(0x12, 0, 0, UIntPtr.Zero); keybd_event(0x12, 0, 2, UIntPtr.Zero); // ALT tap unlocks SetForegroundWindow
    ShowWindow(h, 9); bool fg = SetForegroundWindow(h); Thread.Sleep(300);
    sb.Append("{\"hwnd\":" + h.ToInt64() + ",\"fg\":" + (fg ? "true" : "false") + ",\"isFg\":" + (GetForegroundWindow() == h ? "true" : "false"));
    var ui = new List<string>(); bool stop = false; int probes = 0;
    var mon = new Thread(() => { while (!stop) { long a = Epoch(); var sw = Stopwatch.StartNew(); UIntPtr r; SendMessageTimeout(h, 0, UIntPtr.Zero, IntPtr.Zero, 0, 10000, out r); long ms = sw.ElapsedMilliseconds; probes++; if (ms > 8) lock (ui) ui.Add("[" + a + "," + ms + "]"); Thread.Sleep(5); } });
    mon.Start(); Thread.Sleep(500);
    var sent = new List<long>(); var clock = Stopwatch.StartNew(); long next = 0;
    for (int k = 0; k < count; k++) {
      while (clock.ElapsedMilliseconds < next) Thread.Sleep(1);
      var arr = new INPUT[] { Key((ushort)vk, false), Key((ushort)vk, true) };
      SendInput(2, arr, Marshal.SizeOf(typeof(INPUT))); sent.Add(Epoch()); next += delay;
    }
    Thread.Sleep(3000); stop = true; mon.Join();
    sb.Append(",\"sentFirst\":" + sent[0] + ",\"sentLast\":" + sent[sent.Count - 1] + ",\"probes\":" + probes + ",\"ui\":[" + string.Join(",", ui) + "]}");
    return sb.ToString();
  }
}
"@
[P746]::Run($procId, $count, $delay, $vk) | Out-File -FilePath $out -Encoding ascii
`;
fs.writeFileSync(path.join(TMP, "p746.ps1"), WIN_HELPER);

async function driveOs(key, browser) {
  if (WIN) {
    const out = path.join(TMP, `os-${key}.json`);
    const pid = webviewTarget.applicationProcess.pid;
    const r = spawnSync("powershell", ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", path.join(TMP, "p746.ps1"),
      "-procId", String(pid), "-count", String(KEYS), "-delay", String(DELAY), "-vk", String(key === "1" ? 0x31 : 0x08), "-out", out], { encoding: "utf8", timeout: 120000 });
    if (r.status !== 0) throw new Error(`SendInput helper failed: ${r.stdout} ${r.stderr}`);
    const res = JSON.parse(fs.readFileSync(out, "utf8").trim());
    return { meta: { hwnd: res.hwnd, fg: res.fg, isFg: res.isFg, probes: res.probes, sentSpan: res.sentLast - res.sentFirst }, win: res.ui.map(([at, ms]) => ({ at, ms })) };
  }
  const t = Date.now();
  const r = spawnSync("xdotool", ["key", "--delay", String(DELAY), ...Array(KEYS).fill(key === "1" ? "1" : "BackSpace")], { encoding: "utf8", timeout: 120000 });
  if (r.status !== 0) throw new Error(`xdotool failed: ${r.stderr}`);
  await sleep(3000);
  return { meta: { sentSpan: Date.now() - t - 3000 }, win: null };
}

async function driveWebdriver(key, browser) {
  const value = key === "1" ? "1" : "";
  const actions = [];
  for (let k = 0; k < KEYS; k++) actions.push({ type: "keyDown", value }, { type: "keyUp", value }, { type: "pause", duration: DELAY });
  const t = Date.now();
  await browser.performActions([{ type: "key", id: "kb746", actions }]);
  await browser.releaseActions().catch(() => {});
  const span = Date.now() - t;
  await sleep(3000);
  return { meta: { sentSpan: span }, win: null };
}

let browser;
const report = { label: LABEL, platform: process.platform, app: APP, keys: KEYS, delayMs: DELAY, runs: [] };
try {
  browser = await remote({ hostname: "127.0.0.1", port: DRIVER, path: "/", logLevel: "error", connectionRetryCount: 1, connectionRetryTimeout: 120_000,
    capabilities: tauriCapabilities(APP, "default", process.platform, webviewTarget.debuggerAddress) });
  await ensureMainWindow(browser);
  try { await browser.setWindowRect(0, 0, 1600, 1000); } catch (e) { console.log(`setWindowRect: ${e.message}`); }
  report.env = await browser.execute(() => ({ ua: navigator.userAgent, dpr: devicePixelRatio, w: innerWidth, h: innerHeight }));
  console.log(JSON.stringify(report.env));
  await browser.$(".page-title").waitForExist({ timeout: 180000 });
  report.firstTitleMs = Date.now() - launchedAt;
  console.log(await browser.execute(INSTRUMENT));
  await openPageByName(browser, "GH746 typing", { timeout: 180000 });
  // Settle: at least SETTLE ms after launch, then until no IPC started for 5 s (max 240 s).
  const settleStart = Date.now();
  while (Date.now() - launchedAt < SETTLE) await sleep(1000);
  for (;;) {
    const quiet = await browser.execute(() => { const P = window.__p746; const now = performance.now();
      return !P.ipc.some((r) => r[2] < 0 && now - r[1] < 60000) && !P.ipc.some((r) => now - r[1] < 5000); });
    if (quiet || Date.now() - settleStart > 240000) break;
    await sleep(1000);
  }
  report.settledMs = Date.now() - launchedAt;
  // Edit the third top-level block ("文文", with one child) at its end.
  const enter = async () => {
    await browser.waitUntil(() => browser.execute(() => {
      const roots = [...document.querySelectorAll(".page .ls-block")].filter((b) => !b.parentElement.closest(".ls-block"));
      const target = roots.find((b) => (b.querySelector(".block-content-wrapper")?.textContent ?? "").trim() === "文文") ?? roots[2];
      const content = target?.querySelector(".block-content-wrapper");
      if (!(content instanceof HTMLElement)) return false;
      content.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0 }));
      content.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, cancelable: true, button: 0 }));
      content.click();
      return document.activeElement instanceof HTMLTextAreaElement;
    }), { timeout: 15000, interval: 300, timeoutMsg: "editor did not open" });
    await browser.execute(() => { const ta = document.activeElement; ta.selectionStart = ta.selectionEnd = ta.value.length; });
  };
  await enter();
  report.initialValue = await browser.execute(() => document.activeElement.value);
  for (const mode of MODES) {
    for (const key of ["1", "Backspace"]) {
      if (!(await browser.execute(() => document.activeElement?.tagName === "TEXTAREA"))) await enter();
      await sleep(1500);
      await browser.execute(drain);
      const drive = mode === "os" ? await driveOs(key, browser) : await driveWebdriver(key, browser);
      const raw = JSON.parse(await browser.execute(drain));
      const value = await browser.execute(() => document.activeElement?.tagName === "TEXTAREA" ? document.activeElement.value : null);
      const a = analyse(raw, drive.win);
      const run = { mode, key, drive: drive.meta, valueLength: value?.length ?? null, ...a };
      report.runs.push(run);
      fs.writeFileSync(path.join(OUT, `raw-${mode}-${key}.json`), JSON.stringify({ raw, win: drive.win }));
      console.log(JSON.stringify({ label: LABEL, mode, key, trusted: a.trusted, inputs: a.inputs, gap: a.inputGapMs, over: a.gapsOver, frames: a.frameGapMs, ui: a.uiThread && { max: a.uiThread.max, over50: a.uiThread.over50 }, intervals: a.stallIntervalsMs }));
      await sleep(2000);
    }
  }
  await browser.saveScreenshot(path.join(OUT, "end.png")).catch(() => {});
} catch (error) {
  report.error = String(error?.stack || error);
  console.log(report.error);
  if (browser) await browser.saveScreenshot(path.join(OUT, "failure.png")).catch(() => {});
} finally {
  fs.writeFileSync(path.join(OUT, "report.json"), JSON.stringify(report, null, 1));
  try { await browser?.deleteSession(); } catch {}
  try { WIN ? spawnSync("taskkill", ["/PID", String(driver.pid), "/T", "/F"], { stdio: "ignore" }) : process.kill(-driver.pid, "SIGKILL"); } catch {}
  stopWebdriverApplication(webviewTarget);
  fs.closeSync(log);
  if (!WIN) spawnSync("pkill", ["-f", GRAPH], { stdio: "ignore" });
}
process.exit(0);
