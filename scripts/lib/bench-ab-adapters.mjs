// Publish-signal adapters for scripts/bench-storage-ab.mjs.
//
// "Save completed" is never file readability (STEP3-DESIGN §13). Each arm is
// observed through ONE adapter that yields the same neutral records:
//   publishes  [{t, text, ...}]   a completed, successful publication; `text` is
//                                 what it carried (matched against the typed token)
//   saveSpans  [{t0, t1}]         intervals in which a save was running
//   drafts     [{t, text}]        a draft became durable (custody)
//   calls      [{cmd, t0, t1, ms, ok}]  every tracked IPC command (round trips)
//   extra      {...}              candidate-only event streams
//
// "ipc" (the base, og 9a0ebf01b and the og-storage head e19281dd7, which still run
// the old engine): the base logs NOTHING at publication (flight.rs records only
// failed or >= 150 ms saves, with no timestamp; commands.rs `log_save_kinds` runs
// BEFORE the save), so the publish signal is the `save_pages` IPC response: the
// command returns only after tine_graph_features::pages::save_pages has written,
// synced and indexed the page (src-tauri/src/commands.rs save_pages ->
// save_wire::save_pages_wire). The window's fetch is wrapped, as bench-og-parity
// already does for rename_page; request bodies are matched for the typed token.
//
// "events" (the candidate): a bench-only NDJSON file named by TINE_BENCH_EVENTS,
// specified in docs/bench-storage-ab.md. The harness reads it; the app writes it.
import fs from "node:fs";
import { parseNdjson, typedPrefixIn } from "./bench-ab-stats.mjs";

/** IPC commands whose round trips are recorded (both generations of the surface). */
export const TRACKED_COMMANDS = [
  "save_pages", "store_draft", "retire_draft", "rename_page", "delete_page",
  "page_open", "page_submit", "page_move", "page_discard", "page_close", "page_delete", "page_rename",
];

/** Request-body patterns kept per tracked call (regex sources, global). */
export const BODY_PATTERNS = ["qzx[a-z0-9]*", "carrytask[0-9]+", "extburst[a-z0-9]+", "id:: [0-9a-f-]{36}"];

/** Installed in the page. Self-contained: it is serialized by WebDriver. */
export function installProbe(tracked, patternSources) {
  if (window.__abProbe) return window.__abProbe.mode;
  const p = { mode: "raf-gap", journey: "open", cmds: {}, calls: [], keys: [], longs: [], inflight: 0, last: performance.now() };
  const patterns = patternSources.map((s) => new RegExp(s, "g"));
  const trackedSet = new Set(tracked);
  const originalFetch = window.fetch;
  const decode = (body) => {
    try {
      if (typeof body === "string") return body;
      if (body instanceof ArrayBuffer || ArrayBuffer.isView(body)) return new TextDecoder().decode(body);
    } catch { /* not decodable */ }
    return "";
  };
  window.fetch = async function (url, options) {
    const cmd = String(url).split("?")[0].split("/").pop();
    p.cmds[cmd] = (p.cmds[cmd] || 0) + 1;
    if (!trackedSet.has(cmd)) return originalFetch.call(this, url, options);
    const rec = { cmd, t0: Date.now(), t1: null, ok: null, runs: [], err: null };
    p.calls.push(rec);
    p.inflight++;
    try {
      const text = decode(options && options.body);
      for (const re of patterns) { re.lastIndex = 0; const m = text.match(re); if (m) rec.runs.push(...m); }
      const response = await originalFetch.call(this, url, options);
      rec.t1 = Date.now();
      const isError = response.headers.get("Tauri-Response") === "error";
      // Judge success from the body without delaying the caller.
      response.clone().text().then((body) => {
        let ok = !isError;
        if (ok && cmd === "save_pages") {
          try { const v = JSON.parse(body); ok = Array.isArray(v.ok) && !v.failed; } catch { ok = false; }
        }
        rec.ok = ok;
      }, () => { rec.ok = false; });
      return response;
    } catch (error) {
      rec.err = String(error);
      rec.t1 = Date.now();
      rec.ok = false;
      throw error;
    } finally {
      p.inflight--;
    }
  };
  const support = typeof PerformanceObserver === "undefined" ? [] : PerformanceObserver.supportedEntryTypes ?? [];
  if (support.includes("longtask")) {
    p.mode = "longtask";
    new PerformanceObserver((list) => {
      for (const entry of list.getEntries()) if (entry.duration > 100) p.longs.push({ journey: p.journey, ms: entry.duration, at: Date.now() });
    }).observe({ entryTypes: ["longtask"] });
  } else {
    const tick = (now) => {
      const gap = now - p.last;
      if (gap > 100) p.longs.push({ journey: p.journey, ms: gap, at: Date.now() });
      p.last = now;
      requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  }
  document.addEventListener("input", (event) => {
    if (!(event.target instanceof HTMLTextAreaElement) || !event.target.classList.contains("block-editor")) return;
    const key = { t: Date.now(), at: performance.now(), lat: null, inSave: p.inflight > 0, journey: p.journey };
    p.keys.push(key);
    // A timer queued inside rAF runs after that rendering opportunity.
    requestAnimationFrame(() => setTimeout(() => { key.lat = performance.now() - key.at; }, 0));
  }, true);
  window.__abProbe = p;
  return p.mode;
}

export function readProbe() {
  const p = window.__abProbe;
  return { mode: p.mode, cmds: p.cmds, inflight: p.inflight, keys: p.keys, longs: p.longs,
    calls: p.calls.map((c) => ({ cmd: c.cmd, t0: c.t0, t1: c.t1, ok: c.ok, runs: c.runs, err: c.err })) };
}

export function setJourney(name) {
  window.__abProbe.journey = name;
  window.__abProbe.last = performance.now();
}

/** In-page stopwatch for "action -> visible" timings. Timing in the page uses one clock and no
 *  WebDriver round trips (which cost 10-40 ms each and made a 20 ms interval read as 17..108 ms).
 *  kind "carry": starts at the click on the carry button, stops when every needle is in the
 *  journal section. kind "blockref": starts at the Enter keydown, stops when ((uuid)) is in the
 *  editor. Self-contained: serialized by WebDriver. */
export function armWatch(kind, needles) {
  const w = { kind, t0: null, dt: null };
  window.__abWatch = w;
  const cond = kind === "carry"
    ? () => { const text = document.querySelector(".page-section")?.innerText ?? ""; return needles.every((n) => text.includes(n)); }
    : () => /\(\([0-9a-f-]{36}\)\)/.test(document.querySelector("textarea.block-editor")?.value ?? "");
  const check = () => {
    if (w.dt != null) return;
    if (cond()) { w.dt = performance.now() - w.t0; return; }
    setTimeout(check, 0);
  };
  const start = () => { if (w.t0 == null) { w.t0 = performance.now(); check(); } };
  if (kind === "carry") document.addEventListener("click", (e) => { if (e.target.closest?.(".carry-btn-days")) start(); }, true);
  else document.addEventListener("keydown", (e) => { if (e.key === "Enter") start(); }, true);
}
export function readWatch() { return window.__abWatch ? { t0: window.__abWatch.t0, dt: window.__abWatch.dt } : null; }

/** Neutral records from the in-page call log (the "ipc" arm). */
function fromCalls(calls) {
  const publishes = [];
  const saveSpans = [];
  const drafts = [];
  const rtt = [];
  for (const c of calls) {
    if (c.t1 == null) continue;
    rtt.push({ cmd: c.cmd, t0: c.t0, t1: c.t1, ms: c.t1 - c.t0, ok: c.ok });
    if (c.cmd === "save_pages") {
      saveSpans.push({ t0: c.t0, t1: c.t1 });
      if (c.ok) publishes.push({ t: c.t1, text: c.runs.join(" "), page: null });
    } else if (c.cmd === "store_draft" && c.ok) {
      drafts.push({ t: c.t1, text: c.runs.join(" ") });
    }
  }
  return { publishes, saveSpans, drafts, rtt, extra: emptyExtra() };
}

/** Candidate-only event streams; empty (never undefined) for an arm that has none. */
function emptyExtra() {
  return { custodyComplete: [], mailParse: [], hostStats: [], indexPublished: [], launchRecovered: [], unfreeze: [] };
}

/** Neutral records from the candidate's events file plus the page's command log. */
function fromEvents(eventsFile, calls) {
  const ipc = fromCalls(calls);
  let rows = [];
  try { rows = parseNdjson(fs.readFileSync(eventsFile, "utf8")); } catch { /* not created yet */ }
  const begins = new Map();
  const publishes = [];
  const saveSpans = [];
  const drafts = [];
  const extra = emptyExtra();
  for (const r of rows) {
    switch (r.ev) {
      case "save_begin": begins.set(`${r.key}`, r.t); break;
      case "published": {
        publishes.push({ t: r.t, text: r.text ?? "", page: r.path ?? null, key: r.key, version: r.version, bytes: r.bytes_len });
        const b = begins.get(`${r.key}`);
        if (b != null) { saveSpans.push({ t0: b, t1: r.t }); begins.delete(`${r.key}`); }
        break;
      }
      case "draft_durable": drafts.push({ t: r.t, text: r.text ?? "", page: r.path ?? null }); break;
      case "custody_complete": extra.custodyComplete.push(r); break;
      case "mail_parse": extra.mailParse.push(r); break;
      case "host_stats": extra.hostStats.push(r); break;
      case "index_published": extra.indexPublished.push(r); break;
      case "launch_recovered": extra.launchRecovered.push(r); break;
      case "unfreeze": extra.unfreeze.push(r); break;
      default: break;
    }
  }
  return { publishes, saveSpans, drafts, rtt: ipc.rtt, extra };
}

export class Adapter {
  /** @param {"ipc"|"events"} kind @param {string} eventsFile the trial's TINE_BENCH_EVENTS path */
  constructor(kind, eventsFile) {
    if (kind !== "ipc" && kind !== "events") throw new Error(`unknown publish adapter ${JSON.stringify(kind)} (ipc | events)`);
    this.kind = kind;
    this.eventsFile = eventsFile;
  }
  /** Extra environment for the launched app. */
  env() { return this.kind === "events" ? { TINE_BENCH_EVENTS: this.eventsFile } : {}; }
  async install(browser) {
    return browser.execute(installProbe, TRACKED_COMMANDS, BODY_PATTERNS);
  }
  async journey(browser, name) { await browser.execute(setJourney, name); }
  async probe(browser) { return browser.execute(readProbe); }
  /** Everything observed so far; callers filter by time. */
  async collect(browser) {
    const probe = await this.probe(browser);
    const records = this.kind === "events" ? fromEvents(this.eventsFile, probe.calls) : fromCalls(probe.calls);
    return { ...records, probe };
  }
  /** A publish after `sinceMs` that carried the whole typed `token`; null if none yet. */
  static findPublish(records, token, sinceMs) {
    return records.publishes.filter((p) => p.t >= sinceMs && typedPrefixIn(p.text, token) >= token.length)
      .sort((a, b) => a.t - b.t)[0] ?? null;
  }
  static findDraft(records, token, sinceMs) {
    return records.drafts.filter((d) => d.t >= sinceMs && typedPrefixIn(d.text, token) >= token.length)
      .sort((a, b) => a.t - b.t)[0] ?? null;
  }
}
