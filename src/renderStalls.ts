// GH #746: renderer work can block typing even when every IPC is fast.
// Memory only; no input text, key, DOM target, URL or function name is retained.
import { isHTMLElementNode, onEachWindow } from "./windowRealm";
export type Stall = {
  kind: "frame" | "key-frame" | "loaf" | "event";
  startTime: number;
  duration: number;
  blockingDuration?: number;
  scripts?: { source?: string; position?: number; duration: number }[];
};
export const STALL_LIMIT = 32;
export const STALL_MS = 100;
export function createStallBuffer(limit = STALL_LIMIT) {
  const entries: Stall[] = [];
  return {
    record(entry: Stall) {
      if (!Number.isFinite(entry.duration) || entry.duration < STALL_MS) return;
      if (entries.length >= limit && entry.duration <= entries[entries.length - 1].duration) return;
      entries.push(entry);
      entries.sort((a, b) => b.duration - a.duration);
      entries.length = Math.min(entries.length, limit);
    },
    snapshot: () => entries.map(entry => ({ ...entry, scripts: entry.scripts?.map(script => ({ ...script })) })),
    clear: () => { entries.length = 0; },
  };
}

/** Attribution only for bundled assets. Extension/user scripts keep duration
 * and position but lose their source: the diagnostic privacy promise wins. */
export function safeScriptSource(source: string): string | undefined {
  try {
    const url = new URL(source);
    if (!["tauri:", "http:", "https:"].includes(url.protocol)) return;
    if (!["localhost", "tauri.localhost"].includes(url.hostname)) return;
    const match = /^\/assets\/([A-Za-z0-9_-]+\.js)$/.exec(url.pathname);
    return match?.[1].slice(0, 120);
  } catch { return; }
}

const ring = createStallBuffer();
const supported: string[] = [];
export const renderStallSnapshot = () => ({ thresholdMs: STALL_MS, capacity: STALL_LIMIT, supported: [...supported], stalls: ring.snapshot() });
export const clearRenderStalls = () => ring.clear();

/** One owner per WebView, shared by main, split and sidebar editors. Nothing
 * runs per frame: a continuous requestAnimationFrame chain during the 2 s after
 * each key cost Chromium WebViews about a frame on large-page load and scroll
 * (hosted bench, 8554ec5a1), so a burst of keys requests one frame and records
 * key -> frame. LoAF and Event Timing observers (Chromium) run from the first
 * key, buffered so its entries arrive, until 2 s without input. */
export function installRenderStallMonitor(): () => void {
  return onEachWindow(win => installWindowMonitor(win));
}

const IDLE_MS = 2000;

function installWindowMonitor(win: Window): () => void {
  const doc = win.document;
  const Observer = (win as Window & { PerformanceObserver?: typeof PerformanceObserver }).PerformanceObserver;
  const types: ("long-animation-frame" | "event")[] = [];
  for (const type of ["long-animation-frame", "event"] as const) {
    try { if (Observer?.supportedEntryTypes.includes(type)) types.push(type); } catch { /* not supported */ }
  }
  for (const type of types) if (!supported.includes(type)) supported.push(type);
  let lastInput = -Infinity, pendingKey: number | undefined, raf = 0, idle: number | undefined;
  // Buffered delivery replays earlier entries; anything before the last stop
  // was either recorded then or belongs to an idle period.
  let observedFrom = -Infinity;
  const observers: PerformanceObserver[] = [];
  const startObservers = () => {
    if (observers.length || !Observer) return;
    for (const type of types) {
      try {
        const observer = new Observer(list => {
          for (const entry of list.getEntries()) {
            if (entry.duration < STALL_MS || entry.startTime < observedFrom) continue;
            if (doc.hidden || entry.startTime > lastInput + IDLE_MS || entry.startTime + entry.duration < lastInput - IDLE_MS) continue;
            const detail = entry as PerformanceEntry & { blockingDuration?: number; scripts?: { sourceURL: string; sourceCharPosition: number; duration: number }[] };
            ring.record({ kind: type === "event" ? "event" : "loaf", startTime: entry.startTime, duration: entry.duration,
              blockingDuration: detail.blockingDuration,
              scripts: detail.scripts?.slice(0, 8).map(script => ({ source: safeScriptSource(script.sourceURL), position: script.sourceCharPosition >= 0 ? script.sourceCharPosition : undefined, duration: script.duration })) });
          }
        });
        observer.observe({ type, durationThreshold: STALL_MS, buffered: true } as PerformanceObserverInit);
        observers.push(observer);
      } catch { /* WebKit and older Chromium still have the key -> frame measure. */ }
    }
  };
  const stopObservers = () => {
    if (idle !== undefined) { win.clearTimeout(idle); idle = undefined; }
    if (!observers.length) return;
    observers.forEach(observer => observer.disconnect());
    observers.length = 0;
    observedFrom = win.performance.now();
  };
  // One timer per idle period, re-armed for the remainder, not one per key.
  const armIdle = (delay: number) => {
    idle = win.setTimeout(() => {
      idle = undefined;
      const left = lastInput + IDLE_MS - win.performance.now();
      if (left > 0) armIdle(left); else stopObservers();
    }, delay);
  };
  const frame = (at: number) => {
    raf = 0;
    if (!doc.hidden && pendingKey !== undefined && at - pendingKey >= STALL_MS) ring.record({ kind: "key-frame", startTime: pendingKey, duration: at - pendingKey });
    pendingKey = undefined;
  };
  const input = (event: Event) => {
    if (!isHTMLElementNode(event.target) || !event.target.matches("textarea, input, [contenteditable=true]")) return;
    lastInput = win.performance.now();
    if (doc.hidden) return;
    // Keep the earliest key before the next frame (a freeze queues many keys).
    const timestamp = event.timeStamp <= lastInput && event.timeStamp >= 0 ? event.timeStamp : lastInput;
    pendingKey ??= timestamp;
    if (!raf) raf = win.requestAnimationFrame(frame);
    startObservers();
    if (idle === undefined) armIdle(IDLE_MS);
  };
  const visibility = () => {
    win.cancelAnimationFrame(raf);
    raf = 0; pendingKey = undefined;
    lastInput = -Infinity;
    stopObservers();
  };
  doc.addEventListener("keydown", input, true);
  doc.addEventListener("input", input, true);
  doc.addEventListener("visibilitychange", visibility);
  return () => {
    doc.removeEventListener("keydown", input, true);
    doc.removeEventListener("input", input, true);
    doc.removeEventListener("visibilitychange", visibility);
    win.cancelAnimationFrame(raf);
    stopObservers();
  };
}
