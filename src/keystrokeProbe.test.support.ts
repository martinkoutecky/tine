// Keystroke-scaling probe (test support; not production code).
//
// Question it answers: after ONE single-node raw write (the input path), how
// much work is done on nodes OTHER than the edited one? Compare the answer on a
// small page and a large page: work that grows with the page is the GH #248
// shape (a reactive pass that walks the page and reads per-node fields, which
// also SUBSCRIBES that pass to every node it read).
//
// Two instruments, both installed before any component mounts:
//  1. Store-read counter. `instrumentDocStore` wraps the Solid store that holds
//     `byId` (mocked into `createStore` by the guard test) with a thin Proxy
//     that counts reads of Node fields (raw, children, collapsed, ...) per node
//     id. Reads still go through the real store proxy, so tracking is
//     unchanged. Each read is attributed to the computation that is running.
//  2. Computation-run counter. Solid's dev hook `afterCreateOwner` lets us wrap
//     every computation's `fn`; each re-run is counted against its creation
//     site (first `src/` frame of the stack taken at creation).
//
// Blind spots are written up in the sweep notes (reads that bypass the store
// proxy via `unwrap`, work that is O(page) but reads no Node field, work after
// a timer/microtask outside the measured window, ...).
import { DEV, untrack } from "solid-js";

export const NODE_FIELDS = ["raw", "children", "collapsed", "parent", "page", "loadedRaw"] as const;
type Field = (typeof NODE_FIELDS)[number];
const FIELD_SET: ReadonlySet<string | symbol> = new Set(NODE_FIELDS);

export interface SiteStat { runs: number; rawReads: number; anyReads: number; readers: Map<string, number> }
export interface Measurement {
  /** Reads of `raw` on nodes other than the edited one. */
  foreignRaw: number;
  /** Reads of any Node field on nodes other than the edited one. */
  foreignAny: number;
  /** Distinct foreign nodes whose `raw` was read. */
  foreignRawNodes: number;
  /** Reads of `raw` on the edited node. */
  ownRaw: number;
  /** Computation re-runs in the window. */
  runs: number;
  /** Wall-clock ms of the edit call (jsdom: only a sanity figure). */
  ms: number;
  bySite: Map<string, SiteStat>;
}

let tracing = false;
let edited: string | null = null;
let cur: { site: string } | null = null;
const runStack: string[] = [];
let bySite = new Map<string, SiteStat>();
let foreignRawIds = new Set<string>();
let foreignRaw = 0, foreignAny = 0, ownRaw = 0, runs = 0;

const THIS_FILE = "keystrokeProbe.test.support";
function frames(stack: string | undefined, limit: number): string[] {
  const out: string[] = [];
  for (const line of (stack ?? "").split("\n")) {
    if (!line.includes("/src/") || line.includes("node_modules") || line.includes(THIS_FILE)) continue;
    const m = /\(?([^()\s]*\/src\/[^():\s]+):(\d+):\d+\)?\s*$/.exec(line.trim());
    if (!m) continue;
    out.push(`${m[1].replace(/^.*\/src\//, "src/")}:${m[2]}`);
    if (out.length >= limit) break;
  }
  return out;
}
const site = (key: string): SiteStat => {
  let s = bySite.get(key);
  if (!s) bySite.set(key, s = { runs: 0, rawReads: 0, anyReads: 0, readers: new Map() });
  return s;
};

Error.stackTraceLimit = 40;

// Wrap every computation's function so re-runs are counted and reads inside are
// attributed. Installed at import time; computations created before the import
// (none: the probe is the first import of the guard test) are not covered.
const hooks = (DEV as unknown as { hooks: { afterCreateOwner: ((o: { fn?: (v: unknown) => unknown; name?: string }) => void) | null } }).hooks;
hooks.afterCreateOwner = (owner) => {
  if (typeof owner.fn !== "function") return;
  const created = frames(new Error().stack, 2).join(" < ") || "(no src frame)";
  const key = owner.name ? `${created} [${owner.name}]` : created;
  const fn = owner.fn;
  owner.fn = function (this: unknown, v: unknown) {
    if (tracing) { runs++; site(key).runs++; }
    runStack.push(key);
    try { return fn.call(this, v); } finally { runStack.pop(); }
  };
};

function noteRead(id: string, field: Field): void {
  if (!tracing) return;
  if (id === edited) { if (field === "raw") ownRaw++; return; }
  foreignAny++;
  const key = runStack.length ? runStack[runStack.length - 1] : "(not in a computation)";
  const s = site(key);
  s.anyReads++;
  if (field !== "raw") return;
  foreignRaw++;
  foreignRawIds.add(id);
  s.rawReads++;
  // Sample the reader's stack (first 25 reads per site) to name the walker.
  if (s.rawReads <= 25) {
    const f = frames(new Error().stack, 3).join(" < ") || "(no src frame)";
    s.readers.set(f, (s.readers.get(f) ?? 0) + 1);
  }
}

const wrappers = new WeakMap<object, object>();
function wrapNode(inner: object, id: string): object {
  let w = wrappers.get(inner);
  if (w) return w;
  w = new Proxy(inner, {
    get(t, p) {
      const v = (t as Record<string | symbol, unknown>)[p];
      if (typeof p === "string" && FIELD_SET.has(p)) noteRead(id, p as Field);
      return v;
    },
  });
  wrappers.set(inner, w);
  return w;
}
function wrapById(inner: object): object {
  let w = wrappers.get(inner);
  if (w) return w;
  w = new Proxy(inner, {
    get(t, p) {
      const v = (t as Record<string | symbol, unknown>)[p];
      if (typeof p === "string" && v && typeof v === "object") return wrapNode(v as object, p);
      return v;
    },
  });
  wrappers.set(inner, w);
  return w;
}
/** Wrap a store iff it is the document store (has `byId` and `pages`). */
export function instrumentDocStore<T extends object>(store: T, initial: unknown): T {
  const i = initial as { byId?: unknown; pages?: unknown } | null;
  if (!i || typeof i !== "object" || !("byId" in i) || !("pages" in i)) return store;
  return new Proxy(store, {
    get(t, p) {
      const v = (t as Record<string | symbol, unknown>)[p];
      if (p === "byId" && v && typeof v === "object") return wrapById(v as object);
      return v;
    },
  });
}

/** Run `edit` as one measured keystroke on node `id`. */
export function measure(id: string, edit: () => void): Measurement {
  bySite = new Map(); foreignRawIds = new Set();
  foreignRaw = foreignAny = ownRaw = runs = 0;
  edited = id; tracing = true;
  const t0 = performance.now();
  try { untrack(edit); } finally { tracing = false; edited = null; }
  const ms = performance.now() - t0;
  return { foreignRaw, foreignAny, foreignRawNodes: foreignRawIds.size, ownRaw, runs, ms, bySite };
}

export function median(xs: number[]): number {
  const s = [...xs].sort((a, b) => a - b);
  return s[Math.floor(s.length / 2)];
}
void cur;
