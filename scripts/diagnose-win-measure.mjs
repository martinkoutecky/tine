#!/usr/bin/env node
// Hosted-Windows measurement probe (diagnostic branch only; not a release gate).
//
// Drives the REAL app (msedgedriver attached to WebView2 on Windows; tauri-driver
// on Linux, for developing the harness) through seven phases per app on one
// graph: cold launch, Ctrl+K steady state, page open, edit -> disk, rename,
// warm reopen, reopen after an external change. Several apps (the candidate and
// released baselines) run sequentially on the SAME runner and the same graph so
// their numbers are comparable. Every phase is caught separately: a missing
// selector in an older release is recorded as n/a and the next phase runs.
//
// Env:
//   TINE_MEASURE_GRAPH   real295 | realistic1k | realistic10k | tiny
//   TINE_MEASURE_APPS    "label=path;label=path"
//   TINE_295_PUBLIC_GRAPH  (real295 only)
//   TINE_MEASURE_OUT     artifact directory (default test-results/win-measure)
//   TINE_MEASURE_PHASES  comma list (default all)
//   TINE_MEASURE_REPS    steady-state repetitions per needle (default 10)
//   TINE_MEASURE_WARMUP  "1" = launch the first app once on a tiny graph first
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawn, spawnSync } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { remote } from "webdriverio";
import {
  freeLoopbackPort,
  selectWebdriverWindowWithSelector,
  startWebdriverApplication,
  stopWebdriverApplication,
  tauriCapabilities,
  webdriverServerArgs,
} from "./e2e-capabilities.mjs";
import { openPageByName, currentPageTitle } from "./lib/e2e-navigation.mjs";
import { clickWhenReachable } from "./lib/e2e-click.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";
import { PROBE, buildGraph, diffSnapshots, externalChange, snapshot } from "./diagnose-win-measure-graph.mjs";

const WIN = process.platform === "win32";
const GRAPH_KIND = process.env.TINE_MEASURE_GRAPH ?? "tiny";
const APPS = (process.env.TINE_MEASURE_APPS ?? "")
  .split(";").map((s) => s.trim()).filter(Boolean)
  .map((entry) => { const i = entry.indexOf("="); return { label: entry.slice(0, i), exe: entry.slice(i + 1) }; });
if (!APPS.length) throw new Error("TINE_MEASURE_APPS is required");
const OUT = path.resolve(process.env.TINE_MEASURE_OUT ?? "test-results/win-measure", GRAPH_KIND);
const WORK = path.resolve(process.env.TINE_MEASURE_WORK ?? path.join(os.tmpdir(), "tine-win-measure"), GRAPH_KIND);
const ALL_PHASES = ["cold", "steady", "open", "edit", "rename", "reopen", "external"];
const PHASES = new Set((process.env.TINE_MEASURE_PHASES ?? ALL_PHASES.join(",")).split(","));
const REPS = Number(process.env.TINE_MEASURE_REPS ?? 10);
const BIG = GRAPH_KIND === "realistic10k";
const BUDGET = {
  coldMs: Number(process.env.TINE_MEASURE_COLD_MS ?? (BIG ? 1_500_000 : 600_000)),
  reopenMs: BIG ? 900_000 : 300_000,
  externalMs: BIG ? 1_200_000 : 600_000,
  renameMs: BIG ? 900_000 : 600_000,
  queryMs: BIG ? 180_000 : 120_000,
  appMs: Number(process.env.TINE_MEASURE_APP_MS ?? (BIG ? 100 * 60_000 : 60 * 60_000)),
};
const SEARCH_BUTTON = 'button[title^="Search (Ctrl+K)"]';
const REDACT = GRAPH_KIND === "real295"; // keep anon-graph words out of public logs
const INDEX_STATUS = /index|rebuild|incomplete|waiting|reading pages|checking/i;

fs.mkdirSync(OUT, { recursive: true });
fs.mkdirSync(WORK, { recursive: true });
const log = (...parts) => console.log(`WM ${GRAPH_KIND}`, ...parts);
const sha256 = (file) => crypto.createHash("sha256").update(fs.readFileSync(file)).digest("hex");
const redact = (s) => (REDACT && typeof s === "string" ? `<${crypto.createHash("sha1").update(s).digest("hex").slice(0, 8)}>` : s);

function percentile(values, q) {
  const v = values.filter((x) => Number.isFinite(x)).sort((a, b) => a - b);
  if (!v.length) return null;
  return v[Math.min(v.length - 1, Math.ceil(q * v.length) - 1)];
}
const stats = (values) => {
  const v = values.filter((x) => Number.isFinite(x));
  return { n: v.length, p50: percentile(v, 0.5), p90: percentile(v, 0.9), max: v.length ? Math.max(...v) : null };
};
const round = (x) => (Number.isFinite(x) ? Math.round(x) : x);

// ---------------------------------------------------------------------------
// Process counters (Rust host process only; WebView2 renderers are separate).

function procCounters(pid) {
  if (!pid) return null;
  if (WIN) {
    const r = spawnSync("pwsh", ["-NoProfile", "-Command",
      `Get-CimInstance Win32_Process -Filter "ProcessId=${pid}" | Select-Object ReadTransferCount,WriteTransferCount,KernelModeTime,UserModeTime,WorkingSetSize,PeakWorkingSetSize | ConvertTo-Json -Compress`],
    { encoding: "utf8", timeout: 30_000 });
    try {
      const j = JSON.parse(r.stdout.trim());
      return { at: Date.now(), readBytes: Number(j.ReadTransferCount), writeBytes: Number(j.WriteTransferCount),
        cpuMs: (Number(j.KernelModeTime) + Number(j.UserModeTime)) / 10_000, workingSetBytes: Number(j.WorkingSetSize),
        peakWorkingSetBytes: Number(j.PeakWorkingSetSize) * 1024 };
    } catch { return null; }
  }
  try {
    const io = Object.fromEntries(fs.readFileSync(`/proc/${pid}/io`, "utf8").trim().split("\n").map((l) => l.split(/:\s+/)).map(([k, v]) => [k, Number(v)]));
    const stat = fs.readFileSync(`/proc/${pid}/stat`, "utf8").split(") ")[1].split(" ");
    const status = fs.readFileSync(`/proc/${pid}/status`, "utf8");
    const kb = (key) => Number((status.match(new RegExp(`${key}:\\s+(\\d+)`)) ?? [])[1] ?? NaN) * 1024;
    return { at: Date.now(), readBytes: io.read_bytes, writeBytes: io.write_bytes, cpuMs: (Number(stat[11]) + Number(stat[12])) * 10,
      workingSetBytes: kb("VmRSS"), peakWorkingSetBytes: kb("VmHWM") };
  } catch { return null; }
}
const delta = (a, b) => (a && b ? {
  ms: b.at - a.at, writeBytes: b.writeBytes - a.writeBytes, readBytes: b.readBytes - a.readBytes, cpuMs: round(b.cpuMs - a.cpuMs),
} : null);

// ---------------------------------------------------------------------------
// In-page instrumentation: IPC timing, status transitions, readiness polls.

function installInstrumentation() {
  if (window.__wm) return "already";
  const native = window.__TAURI_INTERNALS__;
  const original = native.invoke.bind(native);
  const wm = window.__wm = { installedAt: Date.now(), ipc: [], status: [], warm: [], progress: [], nav: null, keys: null };
  const WATCH = /^(run_graph_search|search|quick_switch|get_backlinks|get_unlinked_refs|rename_page|merge_pages|save_page|load_graph|get_page)$/;
  const summarize = (command, value) => {
    if (command === "run_graph_search" && value) {
      const hits = value.hits ?? [];
      return { pages: hits.filter((h) => h.entity === "page").length, blocks: hits.filter((h) => h.entity === "block").length,
        hasMore: value.has_more ?? null, diagnostics: (value.diagnostics ?? []).map((d) => String(d.message).slice(0, 160)).slice(0, 3),
        cancelled: value.cancelled ?? null,
        pageNames: hits.filter((h) => h.entity === "page").slice(0, 20).map((h) => h.page?.name ?? null) };
    }
    if (command === "get_backlinks" || command === "get_unlinked_refs") {
      return Array.isArray(value) ? { groups: value.length, blocks: value.reduce((a, g) => a + (g.blocks?.length ?? 0), 0) } : null;
    }
    if (command === "rename_page") return { touched: value?.touched?.length ?? null, skipped: value?.skippedConflictedReferrers?.length ?? null };
    return null;
  };
  // Tauri defines `__TAURI_INTERNALS__.invoke` non-writable, but its IPC
  // transport calls the global `fetch` at call time (scripts/ipc-protocol.js),
  // so the round trip is observed there: command from the URL, arguments from
  // the JSON body, completion when the response body has been read.
  const nativeFetch = window.fetch.bind(window);
  window.fetch = async (input, init) => {
    const url = typeof input === "string" ? input : input?.url ?? "";
    const m = /^(?:ipc:\/\/localhost|https?:\/\/ipc\.localhost)\/([^?#]+)/.exec(url);
    const command = m ? decodeURIComponent(m[1]) : null;
    if (!command || !WATCH.test(command)) return nativeFetch(input, init);
    let args = null;
    try { if (typeof init?.body === "string") args = JSON.parse(init.body); } catch {}
    const rec = { command, at: Date.now(), start: performance.now(), source: args?.source ?? args?.name ?? args?.query ?? args?.old ?? null, lane: args?.lane ?? null };
    wm.ipc.push(rec);
    if (wm.ipc.length > 6000) wm.ipc.splice(0, 2000);
    let response;
    try {
      response = await nativeFetch(input, init);
    } catch (error) {
      rec.ok = false; rec.error = String(error).slice(0, 300); rec.end = performance.now(); rec.ms = rec.end - rec.start;
      throw error;
    }
    rec.headersMs = performance.now() - rec.start;
    rec.ok = response.headers.get("Tauri-Response") === "ok";
    response.clone().text().then((text) => {
      rec.end = performance.now();
      rec.ms = rec.end - rec.start;
      rec.bytes = text.length;
      try {
        const value = JSON.parse(text);
        if (rec.ok) rec.summary = summarize(command, value);
        else rec.error = String(typeof value === "string" ? value : JSON.stringify(value)).slice(0, 300);
      } catch { if (!rec.ok) rec.error = text.slice(0, 300); }
    }, () => { rec.end = performance.now(); rec.ms = rec.end - rec.start; });
    return response;
  };
  const texts = (selector, attr) => [...document.querySelectorAll(selector)]
    .map((n) => ((attr && n.getAttribute(attr)) || n.textContent || "").trim()).filter(Boolean);
  let last = "";
  setInterval(() => {
    const snap = {
      prog: texts(".indexing-progress", "aria-label"),
      sw: texts('.switcher [role="status"], .switcher [data-search-index-building], .switcher [role="alert"], .switcher .switcher-error'),
      refs: texts(".references-loading"),
      toasts: texts(".toast-msg").slice(0, 6).map((t) => t.slice(0, 160)),
    };
    const key = JSON.stringify(snap);
    if (key !== last) { last = key; wm.status.push({ at: Date.now(), ...snap }); if (wm.status.length > 4000) wm.status.splice(0, 1000); }
  }, 100);
  let warmLast;
  let progLast;
  let warmOn = true;
  let progOn = true;
  let busy = false;
  setInterval(async () => {
    if (busy) return;
    busy = true;
    try {
      if (warmOn) {
        let v;
        try { v = String(await original("warm_done")); } catch (e) { v = `error:${String(e?.message ?? e).slice(0, 120)}`; if (/not found|unknown command|not allowed/i.test(v)) warmOn = false; }
        if (v !== warmLast) { warmLast = v; wm.warm.push({ at: Date.now(), v }); }
      }
      if (progOn) {
        let v;
        try { v = JSON.stringify(await original("indexing_progress")); } catch (e) { v = `error:${String(e?.message ?? e).slice(0, 120)}`; if (/not found|unknown command|not allowed/i.test(v)) { progOn = false; v = "unsupported"; } }
        if (v !== progLast) { progLast = v; wm.progress.push({ at: Date.now(), v }); }
      }
    } finally { busy = false; }
  }, 250);
  const frame = () => new Promise((r) => requestAnimationFrame(() => r()));
  const input = () => document.querySelector(".switcher-input");
  const setValue = (v) => {
    const el = input();
    if (!el) return false;
    el.value = v;
    el.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: v }));
    return true;
  };
  wm.setQuery = setValue;
  wm.snapshot = () => ({
    blockRows: document.querySelectorAll(".switcher-row.block-result").length,
    rows: document.querySelectorAll(".switcher-row").length,
    pending: texts('.switcher [role="status"], .switcher [data-search-index-building]').join(" | "),
    error: texts('.switcher [role="alert"], .switcher .switcher-error').join(" | "),
    noMatch: [...document.querySelectorAll(".switcher-empty")].some((n) => /No matched results/.test(n.textContent)),
    pageNames: [...document.querySelectorAll(".switcher-row:not(.block-result) .switcher-name")].map((n) => n.textContent.trim()).slice(0, 30),
    title: document.querySelector("h1.page-title")?.textContent?.trim() ?? null,
  });
  // One measured Ctrl+K query: clear, type the whole needle at once (the input
  // event a paste produces), wait for the IPC answering exactly this needle,
  // then for the frame after the rows reflecting it are in the DOM.
  wm.measureQuery = async (q, timeoutMs) => {
    if (!input()) return { error: "switcher input absent" };
    setValue("");
    const clearBy = performance.now() + 3000;
    while (performance.now() < clearBy) {
      await frame();
      if (!document.querySelector(".switcher-row.block-result") && !wm.snapshot().pending) break;
    }
    // Outlast the switcher's 110 ms query debounce so the debounced query is
    // really "" before the needle goes in. Without it a repeated needle skips
    // the debounce (debouncedQuery still equals it) and the repetition
    // measures a path no typing user takes.
    const clearedAt = performance.now();
    while (performance.now() - clearedAt < 300) await frame();
    const mark = wm.ipc.length;
    const t0 = performance.now();
    setValue(q);
    const deadline = t0 + timeoutMs;
    let rec = null;
    let settledAt = null;
    while (performance.now() < deadline) {
      await frame();
      rec = null;
      for (let i = wm.ipc.length - 1; i >= Math.min(mark, wm.ipc.length); i--) {
        const r = wm.ipc[i];
        if (r && r.command === "run_graph_search" && r.source === q && r.ms !== undefined) { rec = r; break; }
      }
      if (!rec) continue;
      const s = wm.snapshot();
      if (rec.ok) {
        const reflects = (rec.summary?.blocks ?? 0) > 0 ? s.blockRows > 0 : true;
        if (!s.pending && reflects) { settledAt = performance.now(); break; }
      } else if (s.error) { settledAt = performance.now(); break; }
    }
    if (settledAt !== null) { await frame(); settledAt = performance.now(); }
    const calls = wm.ipc.slice(mark).filter((r) => r.command === "run_graph_search" && r.source === q);
    const s = wm.snapshot();
    return {
      q, timedOut: settledAt === null, keyToRenderMs: settledAt === null ? null : settledAt - t0,
      ipcMs: rec?.ms ?? null, ipcCalls: calls.length, ipcTotalMs: calls.reduce((a, r) => a + (r.ms ?? 0), 0),
      ok: rec?.ok ?? null, error: rec?.error ?? (s.error || null), summary: rec?.summary ?? null,
      renderedBlockRows: s.blockRows, pending: s.pending || null, noMatch: s.noMatch, pageNames: s.pageNames,
    };
  };
  // Page-open recorder: the switcher row mousedown that routes is t0.
  wm.watchNav = (target) => {
    const nfc = (x) => (x ?? "").trim().normalize("NFC");
    const nav = wm.nav = { target, t0: null, t0At: null, title: null, linked: null, linkedCount: null, unlinked: null, unlinkedCount: null, done: false };
    const onDown = (e) => { if (nav.t0 === null && e.target?.closest?.(".switcher-row")) { nav.t0 = performance.now(); nav.t0At = Date.now(); } };
    document.addEventListener("mousedown", onDown, true);
    const started = performance.now();
    const tick = () => {
      if (nav.done) return;
      const now = performance.now();
      if (nav.t0 !== null) {
        if (nav.title === null && nfc(document.querySelector("h1.page-title")?.textContent) === nfc(target)) nav.title = now - nav.t0;
        if (nav.title !== null) {
          const lc = document.querySelector(".linked-references .references-count");
          if (nav.linked === null && lc) { nav.linked = now - nav.t0; nav.linkedCount = lc.textContent.trim(); }
          const uc = document.querySelector(".unlinked-references .references-count");
          if (nav.unlinked === null && uc && !document.querySelector(".unlinked-references .references-loading")) { nav.unlinked = now - nav.t0; nav.unlinkedCount = uc.textContent.trim(); }
        }
      }
      if (now - started > 300_000) { nav.done = true; document.removeEventListener("mousedown", onDown, true); return; }
      requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
    return true;
  };
  // Key recorder for real WebDriver key events into the switcher input.
  wm.watchKeys = () => {
    const state = wm.keys = { pending: [], rows: [] };
    document.addEventListener("keydown", (event) => {
      if (!event.target?.classList?.contains("switcher-input")) return;
      const row = state.pending.shift() ?? { index: state.rows.length, key: event.key, dispatchAt: performance.now() };
      row.keydownAt = performance.now();
      state.rows.push(row);
    }, true);
    document.addEventListener("input", (event) => {
      if (!event.target?.classList?.contains("switcher-input")) return;
      const row = state.rows[state.rows.length - 1];
      if (!row || row.inputAt !== undefined) return;
      row.inputAt = performance.now();
      requestAnimationFrame(() => requestAnimationFrame(() => { row.secondFrameAt = performance.now(); }));
    }, true);
    return true;
  };
  return "installed";
}

// ---------------------------------------------------------------------------
// App sessions.

function tailer(file) {
  const state = { lines: [], offset: 0, readyAt: null, readyLine: null, projectionSeen: false };
  const timer = setInterval(() => {
    try {
      const fd = fs.openSync(file, "r");
      const size = fs.fstatSync(fd).size;
      if (size > state.offset) {
        const buf = Buffer.alloc(size - state.offset);
        fs.readSync(fd, buf, 0, buf.length, state.offset);
        state.offset = size;
        const at = Date.now();
        for (const line of buf.toString("utf8").split(/\r?\n/)) {
          if (!line) continue;
          if (/projection \+/.test(line)) state.projectionSeen = true;
          if (/projection \+\d+ms ready at generation/.test(line) && state.readyAt === null) { state.readyAt = at; state.readyLine = line; }
          if (/projection|warm|index|rebuild|panic|error|fail/i.test(line) && state.lines.length < 3000) state.lines.push({ at, line: line.slice(0, 300) });
        }
      }
      fs.closeSync(fd);
    } catch {}
  }, 250);
  state.stop = () => clearInterval(timer);
  return state;
}

async function launch(app, dirs, tag, appOut) {
  const prefix = path.join(appOut, tag);
  const env = {
    ...process.env,
    TINE_GRAPH: dirs.graph,
    TINE_DEBUG: "1",
    TINE_DEBUG_LOG: `${prefix}-debug.log`,
    TINE_E2E_APPLICATION_STDOUT_LOG: `${prefix}-stdout.log`,
    TINE_E2E_APPLICATION_STDERR_LOG: `${prefix}-stderr.log`,
  };
  if (WIN) {
    Object.assign(env, { APPDATA: dirs.appdata, LOCALAPPDATA: dirs.localappdata, E2E_WEBVIEW_USER_DATA_ROOT: dirs.webview });
  } else {
    Object.assign(env, { XDG_DATA_HOME: `${dirs.xdg}/data`, XDG_CONFIG_HOME: `${dirs.xdg}/config`, XDG_CACHE_HOME: `${dirs.xdg}/cache`,
      WEBKIT_DISABLE_DMABUF_RENDERER: "1", LIBGL_ALWAYS_SOFTWARE: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", GDK_BACKEND: "x11" });
  }
  const nativePort = await freeLoopbackPort();
  const driverPort = await freeLoopbackPort(new Set([nativePort]));
  const driverLogPath = `${prefix}-driver.log`;
  const driverLog = fs.openSync(driverLogPath, "w");
  const session = { app, tag, prefix, env, driverLog, stderrPath: WIN ? env.TINE_E2E_APPLICATION_STDERR_LOG : driverLogPath };
  let target;
  if (WIN) {
    // Start the driver first so the attach is not charged to the app.
    session.driver = spawn("msedgedriver", [`--port=${driverPort}`], { stdio: ["ignore", driverLog, driverLog] });
    await sleep(500);
    session.t0 = Date.now();
    target = await startWebdriverApplication(app.exe, env, nativePort);
    session.target = target;
    session.pid = target.applicationProcess.pid;
    session.tDevtools = Date.now();
    session.tail = tailer(session.stderrPath);
    session.driverEnv = target.env;
  } else {
    session.driver = spawn(process.env.TAURI_DRIVER || "tauri-driver",
      webdriverServerArgs(driverPort, nativePort, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"),
      { env, stdio: ["ignore", driverLog, driverLog] });
    await sleep(1500);
    session.t0 = Date.now();
    session.tail = tailer(session.stderrPath);
  }
  session.browser = await remote({
    hostname: "127.0.0.1", port: driverPort, path: "/",
    capabilities: WIN ? tauriCapabilities(app.exe, "default", "win32", target.debuggerAddress) : tauriCapabilities(app.exe, tag),
    logLevel: "error", connectionRetryCount: 2, connectionRetryTimeout: 120_000,
  });
  session.tAttached = Date.now();
  if (!WIN) {
    const r = spawnSync("pgrep", ["-n", "-f", app.exe], { encoding: "utf8" });
    session.pid = Number(r.stdout.trim()) || null;
  }
  await selectWebdriverWindowWithSelector(session.browser, SEARCH_BUTTON, 180_000);
  session.tWindow = Date.now();
  await session.browser.setTimeout({ script: 900_000 });
  session.install = await session.browser.execute(installInstrumentation);
  if (WIN) {
    session.sampler = spawn("pwsh", ["-NoProfile", "-File", "scripts/diagnose-543-resources.ps1", "-OutputFile", `${prefix}-resources.jsonl`, "-ProcessId", String(session.pid)], { stdio: "ignore" });
  }
  log(app.label, tag, `window=${session.tWindow - session.t0}ms attached=${session.tAttached - session.t0}ms pid=${session.pid}`);
  return session;
}

async function dumpPage(session) {
  try {
    const wm = await session.browser.execute(() => {
      const w = window.__wm;
      if (!w) return null;
      return { installedAt: w.installedAt, status: w.status, warm: w.warm, progress: w.progress, ipc: w.ipc.slice(-1500).map(({ start, end, ...r }) => r) };
    });
    fs.writeFileSync(`${session.prefix}-page.json`, JSON.stringify(wm, null, 1));
    return wm;
  } catch (error) {
    return { error: String(error).slice(0, 300) };
  }
}

async function processExited(session, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (WIN) {
      if (session.target.applicationProcess.exitCode !== null) return true;
    } else if (session.pid) {
      try { process.kill(session.pid, 0); } catch { return true; }
    } else return true;
    await sleep(200);
  }
  return false;
}

/** Close the window the way the user does (a close request, which flushes). */
async function closeSession(session, { clean = true } = {}) {
  const result = { method: null, ms: null, exited: false };
  const t = Date.now();
  if (clean) {
    try {
      await session.browser.execute(() => {
        window.__TAURI_INTERNALS__.invoke("plugin:window|close", { label: "main" }).catch(() => {});
        return true;
      });
      result.method = "window-close-request";
    } catch (error) { result.method = `close-request-failed: ${String(error).slice(0, 120)}`; }
    result.exited = await processExited(session, 60_000);
    if (!result.exited && WIN) {
      spawnSync("taskkill", ["/PID", String(session.pid)], { stdio: "ignore" });
      result.method += "+WM_CLOSE";
      result.exited = await processExited(session, 20_000);
    }
  }
  result.ms = Date.now() - t;
  if (!result.exited) result.method = `${result.method ?? "none"}+forced`;
  try { await session.browser.deleteSession(); } catch {}
  if (WIN) {
    stopWebdriverApplication(session.target);
    if (session.sampler) spawnSync("taskkill", ["/PID", String(session.sampler.pid), "/T", "/F"], { stdio: "ignore" });
    spawnSync("taskkill", ["/PID", String(session.driver.pid), "/T", "/F"], { stdio: "ignore" });
  } else {
    if (!result.exited && session.pid) { try { process.kill(session.pid, "SIGKILL"); } catch {} }
    try { session.driver.kill("SIGKILL"); } catch {}
  }
  session.tail?.stop();
  try { fs.closeSync(session.driverLog); } catch {}
  await sleep(1000);
  return result;
}

// ---------------------------------------------------------------------------
// Shared measurement steps.

async function openSwitcher(browser) {
  if (await browser.execute(() => Boolean(document.querySelector(".switcher-input")))) return;
  for (let attempt = 1; attempt <= 3; attempt++) {
    await clickWhenReachable(browser, SEARCH_BUTTON, { timeout: 30_000, what: "the Search control" });
    try { await browser.$(".switcher-input").waitForExist({ timeout: 5_000 }); return; } catch {}
  }
  throw new Error("Quick Switcher did not open");
}

async function closeSwitcher(browser) {
  for (let i = 0; i < 3; i++) {
    if (!(await browser.execute(() => Boolean(document.querySelector(".switcher-input"))))) return;
    await browser.keys(["Escape"]);
    await sleep(200);
  }
}

async function pageState(browser) {
  return browser.execute(() => {
    const w = window.__wm;
    return { warm: w.warm.at(-1)?.v ?? null, progress: w.progress.at(-1)?.v ?? null, ...w.snapshot() };
  });
}

/** Index readiness as each build can report it. */
function readyNow(session, state) {
  const warm = state.warm === "true";
  const progressIdle = state.progress === null || state.progress === "null" || state.progress === "unsupported";
  const stderrOk = !session.tail.projectionSeen || session.tail.readyAt !== null;
  return warm && progressIdle && stderrOk;
}

/** Type `needle` once and wait for the first rendered result and the index. */
async function firstResultAndReady(session, needle, budgetMs, { requireReady = true, minObserveMs = 0 } = {}) {
  const { browser } = session;
  const out = { needle: redact(needle), firstAnyMs: null, firstCompleteMs: null, readyMs: null, retries: 0, errors: [], timedOut: false };
  await openSwitcher(browser);
  await browser.execute((q) => window.__wm.setQuery(q), needle);
  const deadline = session.t0 + budgetMs;
  let errorSince = null;
  while (Date.now() < deadline) {
    const s = await pageState(browser);
    const now = Date.now();
    if (s.blockRows > 0 && out.firstAnyMs === null) out.firstAnyMs = now - session.t0;
    if (s.blockRows > 0 && !s.pending && !s.error && out.firstCompleteMs === null) {
      out.firstCompleteMs = now - session.t0;
      // The answer shown first: is it the whole answer, or a partial one
      // served while the index was still building?
      out.firstAnswer = await browser.execute((q) => {
        const r = [...window.__wm.ipc].reverse().find((x) => x.command === "run_graph_search" && x.source === q && x.summary);
        return r ? { pages: r.summary.pages, blocks: r.summary.blocks, hasMore: r.summary.hasMore, diagnostics: r.summary.diagnostics } : null;
      }, needle).catch(() => null);
      if (out.readyMs === null && !out.earlyRare) {
        // Completeness while still indexing: the rare overlay needle has
        // exactly 6 blocks in every graph, so a smaller count is a partial
        // answer the user would see as missing results.
        const rare = await measureQuery(browser, PROBE.rare, 30_000).catch((e) => ({ error: String(e).slice(0, 200) }));
        out.earlyRare = { atMs: Date.now() - session.t0, blocks: rare?.summary?.blocks ?? null, pending: rare?.pending ?? null, timedOut: rare?.timedOut ?? null, error: rare?.error ?? null, expected: 6 };
        await browser.execute((q) => window.__wm.setQuery(q), needle);
      }
    }
    if (out.readyMs === null && readyNow(session, s)) out.readyMs = now - session.t0;
    if (s.error) {
      if (!out.errors.includes(s.error.slice(0, 200))) out.errors.push(s.error.slice(0, 200));
      errorSince ??= now;
      if (now - errorSince > 3000) {
        out.retries++;
        await browser.execute((q) => { window.__wm.setQuery(""); setTimeout(() => window.__wm.setQuery(q), 50); }, needle);
        errorSince = null;
      }
    } else errorSince = null;
    if (!s.blockRows && !s.pending && !s.error && s.noMatch && out.firstCompleteMs === null && now - session.t0 > 5000) {
      // An empty answer while the index builds: ask again as a user would.
      out.retries++;
      await browser.execute((q) => { window.__wm.setQuery(""); setTimeout(() => window.__wm.setQuery(q), 50); }, needle);
    }
    const doneResult = out.firstCompleteMs !== null;
    const doneReady = !requireReady || out.readyMs !== null;
    if (doneResult && doneReady && now - session.t0 >= minObserveMs) break;
    await sleep(out.firstCompleteMs === null ? 150 : 400);
  }
  out.timedOut = out.firstCompleteMs === null || (requireReady && out.readyMs === null);
  if (session.tail.readyAt) out.projectionReadyMs = session.tail.readyAt - session.t0;
  out.projectionReadyLine = session.tail.readyLine?.replace(/^.*projection /, "projection ") ?? null;
  if (out.readyMs !== null) {
    // The same needle asked again once ready: the reference answer the first
    // one is compared against.
    const again = await measureQuery(browser, needle, BUDGET.queryMs).catch((e) => ({ error: String(e).slice(0, 200) }));
    out.afterReady = again?.summary ? { pages: again.summary.pages, blocks: again.summary.blocks, hasMore: again.summary.hasMore, ipcMs: again.ipcMs, keyToRenderMs: again.keyToRenderMs } : { error: again?.error ?? "no answer", timedOut: again?.timedOut ?? null };
  }
  const final = await pageState(browser);
  out.finalBlockRows = final.blockRows;
  out.finalPending = final.pending || null;
  await closeSwitcher(browser);
  return out;
}

function statusSummary(page, t0, until = Infinity) {
  if (!page?.status) return null;
  const rows = page.status.filter((s) => s.at <= until);
  const texts = new Set();
  let visibleMs = 0;
  let progMs = 0;
  let firstSeen = null;
  let lastSeen = null;
  for (let i = 0; i < rows.length; i++) {
    const s = rows[i];
    const end = Math.min(rows[i + 1]?.at ?? until, until === Infinity ? (rows[i + 1]?.at ?? s.at) : until);
    const indexTexts = [...s.prog, ...s.sw.filter((t) => INDEX_STATUS.test(t)), ...s.refs.filter((t) => INDEX_STATUS.test(t)), ...s.toasts.filter((t) => INDEX_STATUS.test(t))];
    for (const t of indexTexts) texts.add(t.replace(/[\d,]+( \/ [\d,]+)?/g, "N").slice(0, 140));
    if (indexTexts.length) {
      visibleMs += Math.max(0, end - s.at);
      firstSeen ??= s.at - t0;
      lastSeen = end - t0;
    }
    if (s.prog.length) progMs += Math.max(0, end - s.at);
  }
  const toasts = new Set(rows.flatMap((s) => s.toasts).map((t) => t.slice(0, 140)));
  return { indexStatusVisibleMs: visibleMs, progressBarVisibleMs: progMs, firstSeenMs: firstSeen, lastSeenMs: lastSeen,
    texts: [...texts], toasts: [...toasts], instrumentedAtMs: page.installedAt - t0 };
}

async function waitReady(session, budgetMs) {
  const deadline = Date.now() + budgetMs;
  while (Date.now() < deadline) {
    const s = await pageState(session.browser);
    if (readyNow(session, s)) return Date.now() - session.t0;
    await sleep(500);
  }
  return null;
}

async function measureQuery(browser, needle, timeoutMs) {
  return browser.execute((q, t) => window.__wm.measureQuery(q, t), needle, timeoutMs);
}

// ---------------------------------------------------------------------------
// Phases.

async function phaseCold(session, ctx) {
  const r = await firstResultAndReady(session, ctx.manifest.needles.common, BUDGET.coldMs);
  const counters = procCounters(session.pid);
  const page = await dumpPage(session);
  return {
    windowMs: session.tWindow - session.t0, devtoolsMs: session.tDevtools ? session.tDevtools - session.t0 : null,
    attachedMs: session.tAttached - session.t0, ...r,
    status: statusSummary(page, session.t0), process: counters && { ...counters, sinceLaunchMs: counters.at - session.t0 },
  };
}

async function phaseSteady(session, ctx) {
  const { browser } = session;
  const readyMs = await waitReady(session, BUDGET.reopenMs);
  const out = { readyBeforeMs: readyMs, needles: {} };
  await openSwitcher(browser);
  for (const [label, needle] of Object.entries(ctx.manifest.needles)) {
    const runs = [];
    for (let i = 0; i < REPS; i++) {
      try { runs.push(await measureQuery(browser, needle, BUDGET.queryMs)); } catch (error) { runs.push({ error: String(error).slice(0, 200) }); }
    }
    const last = runs.at(-1) ?? {};
    const counts = [...new Set(runs.map((r) => JSON.stringify([r.summary?.pages ?? null, r.summary?.blocks ?? null, r.summary?.hasMore ?? null])))];
    out.needles[label] = {
      needle: redact(needle), expected: ctx.manifest.expected[label],
      ipcMs: stats(runs.map((r) => r.ipcMs)), keyToRenderMs: stats(runs.map((r) => r.keyToRenderMs)),
      ipcCallsMax: Math.max(...runs.map((r) => r.ipcCalls ?? 0)),
      timedOut: runs.filter((r) => r.timedOut).length, errors: [...new Set(runs.map((r) => r.error).filter(Boolean))],
      resultCounts: counts.map((c) => JSON.parse(c)), renderedBlockRows: last.renderedBlockRows ?? null,
      diagnostics: last.summary?.diagnostics ?? [], pending: last.pending ?? null,
    };
    log(session.app.label, "steady", label, JSON.stringify({ ipc: out.needles[label].ipcMs, render: out.needles[label].keyToRenderMs, counts: out.needles[label].resultCounts }));
  }
  // One needle typed key by key with real WebDriver key events.
  try {
    await browser.execute(() => window.__wm.setQuery(""));
    await browser.$(".switcher-input").click();
    await browser.execute(() => window.__wm.watchKeys());
    const word = PROBE.rare;
    const hostMs = [];
    for (const [index, key] of [...word].entries()) {
      await browser.execute((i, k) => window.__wm.keys.pending.push({ index: i, key: k, dispatchAt: performance.now() }), index, key);
      const started = Date.now();
      await browser.keys([key]);
      hostMs.push(Date.now() - started);
      await browser.waitUntil(() => browser.execute((i) => Number.isFinite(window.__wm.keys.rows.find((r) => r.index === i)?.secondFrameAt), index), { timeout: 15_000, interval: 20 });
      await sleep(120);
    }
    const lastKeyAt = await browser.execute(() => window.__wm.keys.rows.at(-1).keydownAt);
    await browser.waitUntil(() => browser.execute((q) => window.__wm.ipc.some((r) => r.command === "run_graph_search" && r.source === q && r.ms !== undefined), word), { timeout: BUDGET.queryMs, interval: 50 });
    await browser.waitUntil(() => browser.execute(() => window.__wm.snapshot().blockRows > 0 && !window.__wm.snapshot().pending), { timeout: BUDGET.queryMs, interval: 30 });
    const settled = await browser.execute(() => performance.now());
    const keys = await browser.execute(() => window.__wm.keys.rows);
    const value = await browser.execute(() => document.querySelector(".switcher-input")?.value);
    out.typed = {
      needle: word, value, keys: keys.length,
      keydownToSecondPaintMs: stats(keys.map((k) => k.secondFrameAt - k.keydownAt)),
      dispatchToSecondPaintMs: stats(keys.map((k) => k.secondFrameAt - k.dispatchAt)),
      keydownToInputMs: stats(keys.map((k) => k.inputAt - k.keydownAt)),
      hostKeyRoundTripMs: stats(hostMs),
      lastKeyToResultsMs: round(settled - lastKeyAt),
    };
  } catch (error) {
    out.typed = { error: String(error).split("\n")[0].slice(0, 300) };
  }
  await closeSwitcher(browser);
  return out;
}

async function openAndMeasure(session, name, { unlinked = false, budgetMs = 120_000 } = {}) {
  const { browser } = session;
  await browser.execute((n) => window.__wm.watchNav(n), name);
  const t = Date.now();
  await openPageByName(browser, name, { entry: "button", timeout: 60_000 });
  const navReturnedMs = Date.now() - t;
  // Linked References render only when the page has backlinks.
  const deadline = Date.now() + budgetMs;
  const hasLinks = ctxHasLinks(name);
  while (Date.now() < deadline) {
    const nav = await browser.execute(() => window.__wm.nav);
    if (nav.title !== null && (!hasLinks || nav.linked !== null)) break;
    await sleep(100);
  }
  const out = { page: redact(name), navReturnedMs };
  let unl = null;
  if (unlinked) {
    const clickAt = await browser.execute(() => {
      const header = document.querySelector(".unlinked-references .references-header");
      if (!header) return null;
      header.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, button: 0 }));
      return performance.now();
    });
    if (clickAt === null) unl = { error: "no Unlinked References section" };
    else {
      const until = Date.now() + budgetMs;
      while (Date.now() < until) {
        const s = await browser.execute(() => ({
          count: document.querySelector(".unlinked-references .references-count")?.textContent?.trim() ?? null,
          loading: document.querySelector(".unlinked-references .references-loading")?.textContent?.trim() ?? null,
          groups: document.querySelectorAll(".unlinked-references .reference-group").length,
          error: document.querySelector(".unlinked-references .reference-error, .unlinked-references [role=alert]")?.textContent?.trim() ?? null,
          now: performance.now(),
        }));
        if ((s.count !== null && !s.loading) || s.error) {
          unl = { fromClickMs: round(s.now - clickAt), count: s.count, groups: s.groups, error: s.error };
          break;
        }
        await sleep(100);
      }
      unl ??= { timedOut: true };
    }
  }
  const nav = await browser.execute(() => window.__wm.nav);
  const ipc = await browser.execute((n) => window.__wm.ipc.filter((r) => (r.command === "get_backlinks" || r.command === "get_unlinked_refs") && r.source === n)
    .map((r) => ({ command: r.command, ms: r.ms, ok: r.ok, error: r.error, summary: r.summary })), name);
  Object.assign(out, {
    titleMs: round(nav.title), linkedMs: round(nav.linked), linkedCount: nav.linkedCount,
    unlinkedLoadedMs: round(nav.unlinked), unlinkedCount: nav.unlinkedCount, unlinkedExpand: unl,
    backlinksIpc: ipc.filter((r) => r.command === "get_backlinks").map((r) => ({ ms: round(r.ms), ok: r.ok, error: r.error, ...r.summary })),
    unlinkedIpc: ipc.filter((r) => r.command === "get_unlinked_refs").map((r) => ({ ms: round(r.ms), ok: r.ok, error: r.error, ...r.summary })),
  });
  await browser.execute(() => { if (window.__wm.nav) window.__wm.nav.done = true; });
  return out;
}

let CURRENT_MANIFEST = null;
const ctxHasLinks = (name) => {
  const m = CURRENT_MANIFEST;
  return Boolean(m && (m.hub?.name === name || m.renameTarget?.name === name));
};

async function phaseOpen(session, ctx) {
  const m = ctx.manifest;
  const out = {};
  const attempt = async (key, fn) => { try { out[key] = await fn(); } catch (error) { out[key] = { error: String(error).split("\n")[0].slice(0, 300) }; } };
  if (m.hub) await attempt("hub", () => openAndMeasure(session, m.hub.name, { unlinked: true }));
  await attempt("small", () => openAndMeasure(session, PROBE.onePage));
  for (const u of m.unlinkedPages.filter((p) => p.label !== "hub")) {
    await attempt(`unlinked_${u.label}`, async () => ({ expectedMentionBlocks: u.plainMentionBlocks, ...(await openAndMeasure(session, u.name, { unlinked: true })) }));
  }
  return out;
}

async function editOnce(session, graph, page, marker, file) {
  const { browser } = session;
  await openPageByName(browser, page, { entry: "button", timeout: 60_000 });
  const found = await browser.execute((mark) => {
    for (const el of document.querySelectorAll("[data-wm-edit]")) el.removeAttribute("data-wm-edit");
    const content = [...document.querySelectorAll(".page-section .ls-block .block-content, .ls-block .block-content")]
      .find((c) => (c.textContent ?? "").trim().startsWith(mark));
    if (!content) return false;
    content.setAttribute("data-wm-edit", "1");
    return true;
  }, marker);
  if (!found) throw new Error("edit target block not found");
  const target = await browser.$('[data-wm-edit="1"]');
  await target.scrollIntoView({ block: "center" });
  const idleA = procCounters(session.pid);
  await sleep(10_000);
  const idleB = procCounters(session.pid);
  await target.click();
  await browser.$("textarea.block-editor").waitForExist({ timeout: 10_000 });
  await browser.keys(["End"]);
  const before = procCounters(session.pid);
  const t0 = Date.now();
  for (const ch of ` ${PROBE.editWord}`) await browser.keys([ch]);
  const typedAt = Date.now();
  const full = path.join(graph, file);
  await waitForFileText(full, (text) => text.includes(`${marker} ${PROBE.editWord}`) || text.includes(`${marker}${PROBE.editWord}`), `${page} edit`, { timeoutMs: 120_000, intervalMs: 25 });
  const onDisk = Date.now();
  const atDisk = procCounters(session.pid);
  await sleep(10_000);
  const settled = procCounters(session.pid);
  await browser.keys(["Escape"]);
  return {
    typedMs: typedAt - t0, lastKeyToDiskMs: onDisk - typedAt, firstKeyToDiskMs: onDisk - t0,
    writeBytesToDisk: delta(before, atDisk), writeBytesSettled10s: delta(before, settled), idleBaseline10s: delta(idleA, idleB),
    fileBytes: fs.statSync(full).size,
  };
}

async function phaseEdit(session, ctx) {
  const out = {};
  try { out.oneBlock = await editOnce(session, ctx.graph, PROBE.onePage, "probe one block start", `pages/${PROBE.onePage}.md`); } catch (error) { out.oneBlock = { error: String(error).split("\n")[0].slice(0, 300) }; }
  try { out.sixtyBlock = await editOnce(session, ctx.graph, PROBE.sixtyPage, "probe sixty block 30 filler text for the edit measurement", `pages/${PROBE.sixtyPage}.md`); } catch (error) { out.sixtyBlock = { error: String(error).split("\n")[0].slice(0, 300) }; }
  return out;
}

async function phaseRename(session, ctx) {
  const { browser } = session;
  const from = ctx.manifest.renameTarget?.name;
  if (!from) return { error: "no rename target" };
  await openPageByName(browser, from, { entry: "button", timeout: 60_000 });
  const before = snapshot(ctx.graph);
  const title = await browser.$("h1.page-title");
  await title.doubleClick();
  await browser.$("input.page-title-input").waitForExist({ timeout: 10_000 });
  const mark = await browser.execute((to) => {
    const el = document.querySelector("input.page-title-input");
    el.focus();
    el.value = to;
    el.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: to }));
    return window.__wm.ipc.length;
  }, PROBE.renameTo);
  const counters0 = procCounters(session.pid);
  const t0 = Date.now();
  await browser.keys(["Enter"]);
  const deadline = t0 + BUDGET.renameMs;
  let ipc = null;
  let uiMs = null;
  while (Date.now() < deadline) {
    const s = await browser.execute((m) => ({ rec: window.__wm.ipc.slice(m).find((r) => r.command === "rename_page" || r.command === "merge_pages") ?? null, title: document.querySelector("h1.page-title")?.textContent?.trim() ?? null }), mark);
    if (s.rec && s.rec.ms !== undefined) ipc = s.rec;
    if (ipc && s.title === PROBE.renameTo) { uiMs = Date.now() - t0; break; }
    if (ipc && ipc.ok === false) break;
    await sleep(100);
  }
  // Disk quiescence: three identical snapshots one second apart.
  let after = snapshot(ctx.graph);
  let stable = 0;
  const quietBy = Date.now() + 180_000;
  while (stable < 3 && Date.now() < quietBy) {
    await sleep(1000);
    const next = snapshot(ctx.graph);
    const d = diffSnapshots(after, next);
    stable = d.changed.length + d.added.length + d.removed.length === 0 ? stable + 1 : 0;
    after = next;
  }
  const d = diffSnapshots(before, after);
  const lastWrite = Math.max(...[...d.changed, ...d.added].map((rel) => after.get(rel).mtimeMs));
  const counters1 = procCounters(session.pid);
  const out = {
    from: redact(from), referringFiles: ctx.manifest.renameTarget.referringFiles, linkBlocks: ctx.manifest.renameTarget.linkBlocks,
    ipcMs: round(ipc?.ms ?? null), ipcOk: ipc?.ok ?? null, ipcError: ipc?.error ?? null, ipcCommand: ipc?.command ?? null, touchedReported: ipc?.summary?.touched ?? null,
    enterToTitleMs: uiMs, enterToLastFileWriteMs: Number.isFinite(lastWrite) ? round(lastWrite - t0) : null,
    filesChanged: d.changed.length, filesAdded: d.added.length, filesRemoved: d.removed.length,
    process: delta(counters0, counters1),
  };
  try {
    await openSwitcher(browser);
    const q = await measureQuery(browser, PROBE.renameTo, BUDGET.queryMs);
    out.ctrlK = { keyToRenderMs: round(q.keyToRenderMs), ipcMs: round(q.ipcMs), found: (q.pageNames ?? []).includes(PROBE.renameTo) || (q.summary?.pageNames ?? []).includes(PROBE.renameTo), pages: q.summary?.pages ?? null, timedOut: q.timedOut };
    const oldQ = await measureQuery(browser, from, BUDGET.queryMs);
    out.ctrlKOldName = { exactPageRow: (oldQ.pageNames ?? []).includes(from), pages: oldQ.summary?.pages ?? null };
    await closeSwitcher(browser);
  } catch (error) { out.ctrlK = { error: String(error).split("\n")[0].slice(0, 300) }; }
  return out;
}

async function phaseReopen(session, ctx) {
  const sample30 = (async () => {
    const wait = session.t0 + 30_000 - Date.now();
    if (wait > 0) await sleep(wait);
    return procCounters(session.pid);
  })();
  const r = await firstResultAndReady(session, ctx.manifest.needles.common, BUDGET.reopenMs, { minObserveMs: 30_000 });
  const c30 = await sample30;
  const page = await dumpPage(session);
  return {
    windowMs: session.tWindow - session.t0, devtoolsMs: session.tDevtools ? session.tDevtools - session.t0 : null, ...r,
    status: statusSummary(page, session.t0),
    first30s: c30 && { writeBytes: c30.writeBytes, readBytes: c30.readBytes, cpuMs: round(c30.cpuMs), atMs: c30.at - session.t0 },
  };
}

async function phaseExternal(session, ctx, change) {
  const { browser } = session;
  const r = await firstResultAndReady(session, ctx.manifest.needles.common, BUDGET.externalMs, { requireReady: false });
  const out = { windowMs: session.tWindow - session.t0, firstResult: r, change: { modified: change.modified.length, added: change.added.length, deleted: change.deleted.length } };
  const goals = [
    { key: "modified", needle: PROBE.extModify, ok: (b) => b >= change.modified.length, want: change.modified.length },
    { key: "added", needle: PROBE.extAdd, ok: (b) => b >= change.added.length, want: change.added.length },
    { key: "deletedGone", needle: PROBE.extDelete, ok: (b) => b === 0, want: 0 },
  ];
  const found = {};
  const deadline = session.t0 + BUDGET.externalMs;
  await openSwitcher(browser);
  while (Date.now() < deadline && goals.some((g) => !found[g.key]?.done)) {
    for (const g of goals) {
      if (found[g.key]?.done) continue;
      const q = await measureQuery(browser, g.needle, BUDGET.queryMs);
      const blocks = q.summary?.blocks ?? null;
      const cur = found[g.key] ??= { want: g.want, firstAnyMs: null, doneMs: null, lastBlocks: null, polls: 0 };
      cur.polls++;
      cur.lastBlocks = blocks;
      if (blocks > 0 && cur.firstAnyMs === null && g.key !== "deletedGone") cur.firstAnyMs = Date.now() - session.t0;
      if (blocks !== null && g.ok(blocks)) { cur.doneMs = Date.now() - session.t0; cur.done = true; }
    }
    await sleep(1000);
  }
  await closeSwitcher(browser);
  out.detect = found;
  const page = await dumpPage(session);
  out.status = statusSummary(page, session.t0);
  out.readyMs = await waitReady(session, 1000) ?? null;
  return out;
}

// ---------------------------------------------------------------------------

async function runApp(app, pristine, manifest) {
  const appOut = path.join(OUT, app.label);
  fs.mkdirSync(appOut, { recursive: true });
  const root = path.join(WORK, app.label);
  fs.rmSync(root, { recursive: true, force: true });
  const dirs = { graph: path.join(root, "graph"), appdata: path.join(root, "appdata"), localappdata: path.join(root, "localappdata"), webview: path.join(root, "webview"), xdg: path.join(root, "xdg") };
  for (const d of Object.values(dirs)) fs.mkdirSync(d, { recursive: true });
  fs.cpSync(pristine, dirs.graph, { recursive: true });
  const record = { app: app.label, exeSha256: sha256(app.exe), graph: GRAPH_KIND, startedAt: new Date().toISOString(), phases: {}, closes: {} };
  const ctx = { manifest, graph: dirs.graph };
  const appDeadline = Date.now() + BUDGET.appMs;
  const save = () => fs.writeFileSync(path.join(appOut, "record.json"), JSON.stringify(record, null, 2));
  const runPhase = async (name, fn) => {
    if (!PHASES.has(name)) { record.phases[name] = { skipped: "not selected" }; return; }
    if (Date.now() > appDeadline) { record.phases[name] = { skipped: "per-app budget exhausted" }; return; }
    const t = Date.now();
    try { record.phases[name] = await fn(); } catch (error) {
      record.phases[name] = { error: String(error?.stack ?? error).split("\n").slice(0, 4).join(" | ").slice(0, 600) };
    }
    record.phases[name].phaseWallMs = Date.now() - t;
    log(app.label, name, "done", `${Date.now() - t}ms`, record.phases[name].error ? `ERROR ${record.phases[name].error.slice(0, 200)}` : "");
    save();
  };
  const withSession = async (tag, phases) => {
    let session;
    try {
      session = await launch(app, dirs, tag, appOut);
    } catch (error) {
      for (const [name] of phases) record.phases[name] = { error: `launch failed: ${String(error).split("\n")[0].slice(0, 400)}` };
      save();
      return;
    }
    for (const [name, fn] of phases) await runPhase(name, () => fn(session));
    await dumpPage(session);
    record.closes[tag] = await closeSession(session);
    if (session.tail) fs.writeFileSync(`${session.prefix}-stderr-lines.json`, JSON.stringify(session.tail.lines, null, 1));
    save();
  };
  await withSession("session1", [
    ["cold", (s) => phaseCold(s, ctx)],
    ["steady", (s) => phaseSteady(s, ctx)],
    ["open", (s) => phaseOpen(s, ctx)],
    ["edit", (s) => phaseEdit(s, ctx)],
    ["rename", (s) => phaseRename(s, ctx)],
  ]);
  if (PHASES.has("reopen") && Date.now() < appDeadline) {
    await withSession("reopen", [["reopen", (s) => phaseReopen(s, ctx)]]);
  }
  if (PHASES.has("external") && Date.now() < appDeadline) {
    const change = externalChange(dirs.graph);
    record.externalChange = { modified: change.modified.length, added: change.added.length, deleted: change.deleted.length };
    await withSession("external", [["external", (s) => phaseExternal(s, ctx, change)]]);
  }
  record.finishedAt = new Date().toISOString();
  save();
  return record;
}

async function main() {
  const pristine = path.join(WORK, "pristine");
  const t = Date.now();
  const manifest = await buildGraph({ kind: GRAPH_KIND, dest: pristine, publicGraph: process.env.TINE_295_PUBLIC_GRAPH });
  CURRENT_MANIFEST = manifest;
  fs.writeFileSync(path.join(OUT, "manifest.json"), JSON.stringify(manifest, null, 2));
  log("graph built", `${Date.now() - t}ms files=${manifest.files} blocks=${manifest.blocks} bytes=${manifest.bytes}`);
  if (process.env.TINE_MEASURE_WARMUP === "1") {
    try {
      const warmGraph = path.join(WORK, "warmup-pristine");
      await buildGraph({ kind: "tiny", dest: warmGraph });
      const root = path.join(WORK, "warmup");
      fs.rmSync(root, { recursive: true, force: true });
      const dirs = { graph: path.join(root, "graph"), appdata: path.join(root, "appdata"), localappdata: path.join(root, "localappdata"), webview: path.join(root, "webview"), xdg: path.join(root, "xdg") };
      for (const d of Object.values(dirs)) fs.mkdirSync(d, { recursive: true });
      fs.cpSync(warmGraph, dirs.graph, { recursive: true });
      const warmOut = path.join(OUT, "warmup");
      fs.mkdirSync(warmOut, { recursive: true });
      for (const app of APPS) {
        const s = await launch(app, dirs, `warmup-${app.label}`, warmOut);
        await sleep(5000);
        await closeSession(s);
      }
    } catch (error) { log("warmup failed", String(error).split("\n")[0]); }
  }
  const records = [];
  for (const app of APPS) {
    log("app", app.label, "begin");
    records.push(await runApp(app, pristine, manifest));
  }
  fs.writeFileSync(path.join(OUT, "summary.json"), JSON.stringify({ graph: GRAPH_KIND, manifest, records }, null, 2));
  log("finished");
}

await main();
