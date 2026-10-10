// Pure helpers for scripts/bench-storage-ab.mjs: statistics, publish matching,
// A/A noise floors and the syscall-trace aggregation behind "unit cost".
// No I/O here, so every function is unit-tested (scripts/bench-storage-ab.test.mjs).

/** Nearest-rank quantile (the convention scripts/bench-og-parity.mjs uses). */
export function quantile(values, fraction) {
  if (!values.length) return null;
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.min(sorted.length - 1, Math.max(0, Math.ceil(sorted.length * fraction) - 1))];
}

/** Median of the sorted middle (upper middle for an even count, as the existing bench does). */
export function median(values) {
  if (!values.length) return null;
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.floor(sorted.length / 2)];
}

/** n, median, p95, min, max, mean, sd, and range/median ("rangePct"). null for no samples. */
export function describe(values) {
  const v = values.filter(Number.isFinite);
  if (!v.length) return null;
  const mean = v.reduce((a, b) => a + b, 0) / v.length;
  const sd = v.length > 1 ? Math.sqrt(v.reduce((a, b) => a + (b - mean) ** 2, 0) / (v.length - 1)) : 0;
  const med = median(v);
  const min = Math.min(...v);
  const max = Math.max(...v);
  return { n: v.length, median: med, p95: quantile(v, 0.95), min, max, mean, sd,
    rangePct: med ? ((max - min) / Math.abs(med)) * 100 : null };
}

/** Relative difference of two medians as a percentage of their mean: the A/A
 *  noise floor of a median comparison. null when either side has no samples. */
export function medianSpreadPct(a, b) {
  if (!a || !b) return null;
  const mean = (Math.abs(a.median) + Math.abs(b.median)) / 2;
  if (mean === 0) return 0;
  return (Math.abs(a.median - b.median) / mean) * 100;
}

/** The change from `base` to `cand` in percent of base's median; null if undefined. */
export function deltaPct(base, cand) {
  if (!base || !cand || !base.median) return null;
  return ((cand.median / base.median) - 1) * 100;
}

export const NOISY_PCT = 10;

/** Verdict for one metric in an A/B report, given its measured A/A floor (percent). */
export function verdict({ delta, floor, lowerIsBetter = true, absDelta, absFloor }) {
  if (delta == null) return "n/a";
  if (floor == null) return "no A/A floor";
  // A tiny absolute change on a tiny value is not a finding however large the percentage (timer
  // quantization makes 7 ms vs 8 ms a 13 % "spread"), so the absolute tolerance is checked first.
  if (absFloor != null && absDelta != null && Math.abs(absDelta) <= absFloor) return "within noise (absolute)";
  if (floor > NOISY_PCT) return `noisy metric (A/A ${floor.toFixed(0)}%)`;
  if (Math.abs(delta) <= Math.max(floor, 0) * 1.0 + 1e-9) return "within noise";
  const worse = lowerIsBetter ? delta > 0 : delta < 0;
  return worse ? "WORSE (beyond noise)" : "better (beyond noise)";
}

/** Longest common prefix length of two strings. */
export function lcpLen(a, b) {
  const n = Math.min(a.length, b.length);
  let i = 0;
  while (i < n && a.charCodeAt(i) === b.charCodeAt(i)) i++;
  return i;
}

/** How much of the typed `token` a published text carries: the largest k such
 *  that `token.slice(0, k)` occurs in `text` (0 when none). Typing appends one
 *  character at a time to a unique marker, so a save of the page text carries
 *  exactly the prefix typed when it was captured. */
export function typedPrefixIn(text, token) {
  if (!text || !token) return 0;
  let best = 0;
  let from = 0;
  const head = token[0];
  for (;;) {
    const at = text.indexOf(head, from);
    if (at < 0) return best;
    const k = lcpLen(text.slice(at, at + token.length), token);
    if (k > best) best = k;
    if (best === token.length) return best;
    from = at + 1;
  }
}

/** Per-key keystroke -> Published latency. `keys`: [{i, t}] in order, i = 1-based
 *  count of token characters typed at epoch ms t. `publishes`: [{t, k}] where k
 *  is typedPrefixIn() of that publish. Key i is published by the first publish
 *  at or after t(i) with k >= i. Unpublished keys are returned as `lost`. */
export function keyToPublish(keys, publishes) {
  const sorted = [...publishes].sort((a, b) => a.t - b.t);
  const latencies = [];
  const lost = [];
  for (const key of keys) {
    const hit = sorted.find((p) => p.t >= key.t && p.k >= key.i);
    if (hit) latencies.push(hit.t - key.t); else lost.push(key.i);
  }
  return { latencies, lost };
}

/** The earliest time at which every token in `needles` has appeared in some
 *  publish, scanning in time order. null when some needle never appears. */
export function allSeenAt(publishes, needles) {
  const sorted = [...publishes].sort((a, b) => a.t - b.t);
  const seen = new Set();
  for (const p of sorted) {
    for (const needle of needles) if ((p.text ?? "").includes(needle)) seen.add(needle);
    if (seen.size === needles.length) return p.t;
  }
  return null;
}

/** Least-squares slope of (xMs, y) in y-units per minute; null for fewer than 2 points. */
export function slopePerMinute(points) {
  if (points.length < 2) return null;
  const n = points.length;
  const mx = points.reduce((a, p) => a + p.x, 0) / n;
  const my = points.reduce((a, p) => a + p.y, 0) / n;
  let num = 0;
  let den = 0;
  for (const p of points) { num += (p.x - mx) * (p.y - my); den += (p.x - mx) ** 2; }
  return den === 0 ? null : (num / den) * 60_000;
}

const WRITE_SYS = new Set(["write", "pwrite64", "writev", "pwritev", "pwritev2", "sendfile", "copy_file_range"]);
const SYNC_SYS = new Set(["fsync", "fdatasync", "sync_file_range"]);

/** Classify a traced path. `roots` = {graph, appData}. */
export function pathClass(p, roots) {
  if (!p) return "other";
  if (roots.graph && p.startsWith(roots.graph)) return "graph";
  if (roots.appData && p.startsWith(roots.appData)) return "appData";
  return "other";
}

/** Aggregate iotrace events inside [from, to) into the unit-cost fields for one
 *  edit. A write is attributed to its path's class; a "file created" is a
 *  successful open with O_CREAT (0x40) of a path not seen before in the window
 *  (or a mkdir); a "rename" is a successful rename*, a "sync" any fsync-like call
 *  (split into directory syncs by the traced fd). Failed syscalls (ret < 0) count
 *  as `failed`, never as work done. */
export function summarizeIo(events, from, to, roots) {
  const out = { bytes: 0, graphBytes: 0, appDataBytes: 0, writeCalls: 0, filesCreated: 0, filesTouched: 0,
    renames: 0, fileSyncs: 0, dirSyncs: 0, syncCalls: 0, unlinks: 0, mkdirs: 0, failed: 0, truncates: 0 };
  const created = new Set();
  const touched = new Set();
  for (const e of events) {
    if (!(e.t >= from && e.t < to)) continue;
    if (e.ret < 0) { out.failed++; continue; }
    const cls = pathClass(e.path, roots);
    if (cls === "other" && pathClass(e.path2, roots) === "other") continue;
    if (WRITE_SYS.has(e.sys)) {
      out.bytes += e.n; out.writeCalls++;
      if (cls === "graph") out.graphBytes += e.n; else if (cls === "appData") out.appDataBytes += e.n;
      touched.add(e.path);
    } else if (SYNC_SYS.has(e.sys) || e.sys === "syncfs") {
      out.syncCalls++;
      if (e.isdir) out.dirSyncs++; else out.fileSyncs++;
    } else if (e.sys === "open" || e.sys === "openat" || e.sys === "creat") {
      if ((e.flags & 0o100) && !created.has(e.path)) { created.add(e.path); out.filesCreated++; }
      touched.add(e.path);
    } else if (e.sys.startsWith("rename")) {
      out.renames++; touched.add(e.path); touched.add(e.path2);
    } else if (e.sys === "unlink" || e.sys === "unlinkat") {
      out.unlinks++; touched.add(e.path);
    } else if (e.sys === "mkdir" || e.sys === "mkdirat") {
      out.mkdirs++; out.filesCreated++; touched.add(e.path);
    } else if (e.sys === "ftruncate" || e.sys === "truncate") {
      out.truncates++; touched.add(e.path);
    }
  }
  out.filesTouched = touched.size;
  return out;
}

/** Parse NDJSON text into objects, ignoring a torn final line. */
export function parseNdjson(text) {
  const rows = [];
  for (const line of text.split("\n")) {
    if (!line.trim()) continue;
    try { rows.push(JSON.parse(line)); } catch { /* a torn last line while the writer is live */ }
  }
  return rows;
}

/** Percent formatting used in the markdown tables. */
export const fmt = (v, digits = 1) => (v == null || !Number.isFinite(v) ? "-" : v.toFixed(digits));
