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
import WebSocket from "ws";

// Optional Chromium trace of the whole WebView2 (browser + renderer) during one run (PROBE_TRACE=1, Windows).
async function startTrace(debuggerAddress) {
  const v = await (await fetch(`http://${debuggerAddress}/json/version`)).json();
  const ws = new WebSocket(v.webSocketDebuggerUrl, { perMessageDeflate: false, maxPayload: 2 ** 30 });
  await new Promise((res, rej) => { ws.once("open", res); ws.once("error", rej); });
  let id = 0; const pending = new Map(); const events = []; let done;
  const complete = new Promise((res) => { done = res; });
  ws.on("message", (d) => { const m = JSON.parse(d); if (m.id && pending.has(m.id)) { pending.get(m.id)(m); pending.delete(m.id); }
    else if (m.method === "Tracing.dataCollected") events.push(...m.params.value); else if (m.method === "Tracing.tracingComplete") done(); });
  const send = (method, params = {}) => new Promise((res) => { const i = ++id; pending.set(i, res); ws.send(JSON.stringify({ id: i, method, params })); });
  const r = await send("Tracing.start", { transferMode: "ReportEvents", traceConfig: { recordMode: "recordAsMuchAsPossible",
    includedCategories: ["toplevel", "devtools.timeline", "v8.execute", "blink.user_timing", "input", "latencyInfo", "disabled-by-default-devtools.timeline", "blink", "loading"] } });
  return { started: !r.error, error: r.error, stop: async (file) => { await send("Tracing.end"); await Promise.race([complete, sleep(30000)]);
    fs.writeFileSync(file, JSON.stringify({ traceEvents: events })); ws.close(); return events.length; } };
}

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const WIN = process.platform === "win32";
const APP = process.env.TINE_APP || path.join(ROOT, WIN ? "target/release/tine.exe" : "target/release/tine");
const TD = process.env.TAURI_DRIVER || "tauri-driver";
const DRIVER = Number(process.env.E2E_DRIVER_PORT || 4612);
const NATIVE = Number(process.env.E2E_NATIVE_PORT || 4613);
const LABEL = process.env.PROBE_LABEL || "run";
const SRC = process.env.PROBE_GRAPH;
const TARGET_MD = process.env.PROBE_TARGET_MD || path.join(ROOT, "tests/fixtures/gh746/GH746 typing.md");
const MODES = (process.env.PROBE_MODES || (WIN ? "os,osrepeat,webdriver" : "os,webdriver")).split(",");
const KEYS = Number(process.env.PROBE_KEYS || 330);
const HEAVY = process.env.PROBE_HEAVY === "1";
// Follow-up variants: A = beta.8 as is, B = SEND_MAX_WAIT_MS 3000, C = in-flight refresh guard
// (both PROBE-ONLY runtime switches in the probe build), D = stable (just a label).
const VARIANT = process.env.PROBE_VARIANT || "A";
const KEYLIST = (process.env.PROBE_KEYLIST || "1,Backspace").split(",");
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
  const LINKED = Number(process.env.PROBE_LINKED || 40), UNLINKED = Number(process.env.PROBE_UNLINKED || 156);
  while (picked.size < Math.min(LINKED + UNLINKED, pages.length)) picked.add(pages[Math.floor(rnd() * pages.length)]);
  [...picked].forEach((f, i) => {
    const file = path.join(GRAPH, "pages", f);
    const text = fs.readFileSync(file, "utf8");
    const line = i < LINKED ? `- mention ${i} of [[GH746 typing]] here\n` : `- plain mention ${i} of GH746 typing here\n`;
    fs.writeFileSync(file, text + (text.endsWith("\n") || text === "" ? "" : "\n") + line);
  });
  // PROBE_RESOLVE_REFS=1: give every ((uuid)) the page references but does not define a target
  // block elsewhere (as in the reporter's real graph), each with a small subtree.
  if (process.env.PROBE_RESOLVE_REFS === "1" || HEAVY) {
    const md = fs.readFileSync(TARGET_MD, "utf8");
    const defined = new Set([...md.matchAll(/id:: ([0-9a-f-]{36})/g)].map((m) => m[1]));
    const wanted = [...new Set([...md.matchAll(/\(\(([0-9a-f-]{36})\)\)/g)].map((m) => m[1]))].filter((u) => !defined.has(u));
    let out = "";
    wanted.forEach((u, i) => {
      out += `- target block ${i} with a [[link ${i}]] and #tag${i % 5} text text text\n  id:: ${u}\n`;
      for (let c = 0; c < 12; c++) out += `\t- child ${c} of target ${i}, see [[GH746 typing]] and **bold** \`code\`\n\t\t- grandchild ${c}\n`;
    });
    fs.writeFileSync(path.join(GRAPH, "pages", "GH746 targets.md"), out);
    console.log(`resolve refs: ${wanted.length} targets`);
    // PROBE_HEAVY=1 (follow-up run): the page also embeds five of those subtrees and runs two queries.
    if (HEAVY) {
      let extra = "- heavy section: embeds\n";
      for (const u of wanted.slice(0, 5)) extra += `\t- {{embed ((${u}))}}\n`;
      extra += "- {{query [[link 1]]}}\n- {{query (and [[GH746 typing]] \"mention 1\")}}\n";
      const page = path.join(GRAPH, "pages", "GH746 typing.md");
      const text = fs.readFileSync(page, "utf8");
      fs.writeFileSync(page, text + (text.endsWith("\n") ? "" : "\n") + extra);
    }
  }
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
  // Refresh rounds (follow-up run): overlap and queueing of the whole-graph refresh reads.
  const REFRESH = new Set(["get_backlinks", "get_unlinked_refs", "page_icons", "resolve_blocks", "query_run", "query_parse", "get_page"]);
  const rows = raw.ipc.filter((r) => REFRESH.has(r[0]) && r[1] >= t0 - 50);
  const tEnd = raw.now ?? (inputs.at(-1) ?? t0);
  const inflight = (t) => rows.filter((r) => r[1] <= t && (r[2] < 0 || r[2] > t)).length;
  const perSecondMax = [];
  for (let s = 0; t0 + s * 1000 < tEnd; s++) { let m = 0; for (let t = t0 + s * 1000; t < t0 + (s + 1) * 1000 && t < tEnd; t += 50) m = Math.max(m, inflight(t)); perSecondMax.push(m); }
  const roundCmd = rows.some((r) => r[0] === "get_backlinks") ? "get_backlinks" : "resolve_blocks";
  const roundRows = rows.filter((r) => r[0] === roundCmd).sort((a, b) => a[1] - b[1]);
  const overlapped = roundRows.filter((r) => roundRows.some((q) => q !== r && q[1] < r[1] && (q[2] < 0 || q[2] > r[1]))).length;
  const refresh = { roundCmd, rounds: roundRows.length, overlappedRounds: overlapped,
    roundLatency: roundRows.map((r) => `${(r[1] - t0).toFixed(0)}+${r[2] > 0 ? (r[2] - r[1]).toFixed(0) : "open"}`),
    inflightMax: perSecondMax.length ? Math.max(...perSecondMax) : 0, inflightPerSecondMax: perSecondMax,
    unfinishedAtEnd: rows.filter((r) => r[2] < 0).length, reads: rows.length };
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
    refresh, stalls, bigFrames, ipc: ipcBy, mails: raw.mail.length, mailBytes: raw.mail.reduce((s, m) => s + m[1], 0),
    uiThread: win ? { probes: win.length, over50: win.filter((w) => w.ms > 50).length, max: win.reduce((m, w) => Math.max(m, w.ms), 0), slow: win.filter((w) => w.ms > 50).map((w) => `${(w.at - raw.epoch0 - t0).toFixed(0)}+${w.ms}`) } : undefined,
  };
}

// Windows OS-level input + UI-thread probe (one C# helper, compiled by PowerShell).
const WIN_HELPER = String.raw`
param([int]$procId, [int]$count, [int]$delay, [int]$vk, [int]$repeat, [string]$out)
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
  public static string Run(int pid, int count, int delay, int vk, int repeat) {
    timeBeginPeriod(1);
    var proc = Process.GetProcessById(pid); IntPtr h = proc.MainWindowHandle;
    var sb = new StringBuilder();
    keybd_event(0x12, 0, 0, UIntPtr.Zero); keybd_event(0x12, 0, 2, UIntPtr.Zero); // ALT tap unlocks SetForegroundWindow
    ShowWindow(h, 9); bool fg = SetForegroundWindow(h); Thread.Sleep(300);
    sb.Append("{\"hwnd\":" + h.ToInt64() + ",\"fg\":" + (fg ? "true" : "false") + ",\"isFg\":" + (GetForegroundWindow() == h ? "true" : "false"));
    var ui = new List<string>(); bool stop = false; int probes = 0; int fails = 0; int firstErr = 0;
    var mon = new Thread(() => { while (!stop) { long a = Epoch(); var sw = Stopwatch.StartNew(); UIntPtr r; IntPtr ok = SendMessageTimeout(h, 0, UIntPtr.Zero, IntPtr.Zero, 0, 2000, out r); long ms = sw.ElapsedMilliseconds; probes++; if (ok == IntPtr.Zero) { fails++; if (firstErr == 0) firstErr = Marshal.GetLastWin32Error(); } if (ms > 8) lock (ui) ui.Add("[" + a + "," + ms + "]"); Thread.Sleep(5); } });
    mon.Start(); Thread.Sleep(500);
    var sent = new List<long>(); var clock = Stopwatch.StartNew(); long next = 0;
    for (int k = 0; k < count; k++) {
      while (clock.ElapsedMilliseconds < next) Thread.Sleep(1);
      // repeat=1 imitates OS autorepeat: key-downs only (the previous-state bit makes them repeats), one key-up at the end.
      var arr = repeat == 1 ? new INPUT[] { Key((ushort)vk, false) } : new INPUT[] { Key((ushort)vk, false), Key((ushort)vk, true) };
      SendInput((uint)arr.Length, arr, Marshal.SizeOf(typeof(INPUT))); sent.Add(Epoch()); next += delay;
    }
    if (repeat == 1) SendInput(1, new INPUT[] { Key((ushort)vk, true) }, Marshal.SizeOf(typeof(INPUT)));
    Thread.Sleep(3000); stop = true; mon.Join();
    sb.Append(",\"sentFirst\":" + sent[0] + ",\"sentLast\":" + sent[sent.Count - 1] + ",\"probes\":" + probes + ",\"fails\":" + fails + ",\"firstErr\":" + firstErr + ",\"ui\":[" + string.Join(",", ui) + "]}");
    return sb.ToString();
  }
}
"@
[P746]::Run($procId, $count, $delay, $vk, $repeat) | Out-File -FilePath $out -Encoding ascii
`;
fs.writeFileSync(path.join(TMP, "p746.ps1"), WIN_HELPER);

async function driveOs(key, browser, repeat = false) {
  if (WIN) {
    const out = path.join(TMP, `os-${key}.json`);
    const pid = webviewTarget.applicationProcess.pid;
    const r = spawnSync("powershell", ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", path.join(TMP, "p746.ps1"),
      "-procId", String(pid), "-count", String(KEYS), "-delay", String(DELAY), "-vk", String(key === "1" ? 0x31 : 0x08), "-repeat", repeat ? "1" : "0", "-out", out], { encoding: "utf8", timeout: 120000 });
    if (r.status !== 0) throw new Error(`SendInput helper failed: ${r.stdout} ${r.stderr}`);
    const res = JSON.parse(fs.readFileSync(out, "utf8").trim());
    return { meta: { hwnd: res.hwnd, fg: res.fg, isFg: res.isFg, probes: res.probes, fails: res.fails, firstErr: res.firstErr, sentSpan: res.sentLast - res.sentFirst }, win: res.ui.map(([at, ms]) => ({ at, ms })) };
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
  report.variant = VARIANT;
  await browser.execute((v) => { if (v === "B") globalThis.__probe746MaxWait = 3000; if (v === "A") globalThis.__probe746NoGuard = true; }, VARIANT);
  await openPageByName(browser, "GH746 typing", { timeout: 180000 });
  // Settle: at least SETTLE ms after launch, then until no IPC started for 5 s (max 240 s).
  const settle = async () => {
    const settleStart = Date.now();
    for (;;) {
      const quiet = await browser.execute(() => { const P = window.__p746; const now = performance.now();
        return !P.ipc.some((r) => r[2] < 0 && now - r[1] < 60000) && !P.ipc.some((r) => now - r[1] < 5000); });
      if (quiet || Date.now() - settleStart > 240000) break;
      await sleep(1000);
    }
  };
  while (Date.now() - launchedAt < SETTLE) await sleep(1000);
  await settle();
  if (HEAVY) {
    // Expand the Linked and Unlinked References sections (both default to collapsed here).
    report.expanded = await browser.execute(() => {
      let n = 0;
      for (const h of document.querySelectorAll(".references-header")) {
        const c = h.querySelector(".ref-collapse");
        if ((c && c.classList.contains("collapsed")) || (!c && h.textContent.trim().startsWith("\u25b8"))) { h.click(); n++; }
      }
      return n;
    });
    await sleep(3000);
    await settle();
    report.rendered = await browser.execute(() => ({
      linked: document.querySelectorAll(".linked-references .ls-block").length,
      unlinked: document.querySelectorAll(".unlinked-references .ls-block").length,
      blocks: document.querySelectorAll(".ls-block").length,
      headers: [...document.querySelectorAll(".references-header")].map((h) => h.textContent.trim().slice(0, 40)),
    }));
    console.log(JSON.stringify({ expanded: report.expanded, rendered: report.rendered }));
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
    for (const key of KEYLIST) {
      if (!(await browser.execute(() => document.activeElement?.tagName === "TEXTAREA"))) await enter();
      await sleep(1500);
      await browser.execute(drain);
      const trace = WIN && process.env.PROBE_TRACE === "1" && mode === "os" && key === "1" ? await startTrace(webviewTarget.debuggerAddress).catch((e) => ({ started: false, error: String(e) })) : null;
      await browser.execute(() => performance.mark("p746-drive-start"));
      const drive = mode === "os" ? await driveOs(key, browser) : mode === "osrepeat" ? await driveOs(key, browser, true) : await driveWebdriver(key, browser);
      if (trace?.started) report.traceEvents = await trace.stop(path.join(OUT, `trace-${mode}-${key}.json`));
      else if (trace) report.traceError = trace.error;
      const raw = JSON.parse(await browser.execute(drain));
      const value = await browser.execute(() => document.activeElement?.tagName === "TEXTAREA" ? document.activeElement.value : null);
      const a = analyse(raw, drive.win);
      const run = { mode, key, drive: drive.meta, valueLength: value?.length ?? null, ...a };
      report.runs.push(run);
      fs.writeFileSync(path.join(OUT, `raw-${mode}-${key}.json`), JSON.stringify({ raw, win: drive.win }));
      console.log(JSON.stringify({ label: LABEL, variant: VARIANT, mode, key, rounds: a.refresh.rounds, overlapped: a.refresh.overlappedRounds, inflightMax: a.refresh.inflightMax, trusted: a.trusted, inputs: a.inputs, gap: a.inputGapMs, over: a.gapsOver, frames: a.frameGapMs, ui: a.uiThread && { max: a.uiThread.max, over50: a.uiThread.over50 }, intervals: a.stallIntervalsMs }));
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
