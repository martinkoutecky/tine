// PDF perf gate verdict (docs/pdf-perf-harness.md): base vs candidate.
//
// Input: the harness JSONL rows of both binaries, each row tagged
// `<side>-dpr<N>-r<rep>` (side = base | cand), one tag per session.
// Every metric is reduced to ONE value per session (the median over that
// session's trials of the cell: S1c pools its three in/out cycles), so each
// side has `sessions` values per cell and DPR.
//
// A metric regresses when the candidate's BEST session is worse than the
// base's WORST session (the two run-to-run ranges do not overlap) and the gap
// between them clears
//   max(base range, floor, rel × base median)
// i.e. the candidate is worse by more than the run-to-run spread measured on
// the base in this very run, and by more than an absolute floor per metric
// family. Improvements mirror this against the candidate's range. A median
// shift beyond the threshold whose ranges overlap is reported as "watch" and
// never fails the gate. Floors were chosen from same-binary (A/A) runs on the
// reference machine (docs/pdf-perf-harness.md).

export const FAMILIES = {
  blank: { label: "blank", unit: "ms", floor: 150, rel: 0.25 },
  p95: { label: "p95 frame", unit: "ms", floor: 15, rel: 0.2 },
  max: { label: "max frame", unit: "ms", floor: 100, rel: 0.25 },
  over100: { label: "frames >100 ms", unit: "", floor: 3, rel: 0.5 },
  sharp: { label: "time to sharp", unit: "ms", floor: 80, rel: 0.25 },
  drift: { label: "anchor drift", unit: "px", floor: 25, rel: 0.2 },
  rss: { label: "web RSS peak", unit: "MiB", floor: 64, rel: 0.1 },
};

const get = (o, p) => p.split(".").reduce((a, k) => (a == null ? a : a[k]), o);
const scroll = { blank: "during.blankMs", p95: "during.p95", max: "during.max", over100: "during.over100", sharp: "toSharpAfterEndMs" };
const zoom = { blank: "whole.blankMs", p95: "whole.p95", max: "whole.max", over100: "whole.over100", sharp: "sharpFromLastMs", drift: "anchorDriftPx" };
const open = { blank: "whole.blankMs", max: "whole.max", over100: "whole.over100", sharp: "openSettledMs" };

/** The gate's cells: [id, row selector, { family: row path }]. */
export const CELLS = [
  ["open S1 paper", (r) => r.scenario === "open" && r.name.startsWith("arikan"), open],
  ["open S2 book", (r) => r.scenario === "open" && r.name.startsWith("bv_cvxbook"), open],
  ["open S3 scan", (r) => r.scenario === "open" && r.name.startsWith("sourcebook"), open],
  ["S1a inpage fast", (r) => r.scenario === "S1a" && r.name === "inpage-fast", scroll],
  ["S1a inpage moderate", (r) => r.scenario === "S1a" && r.name === "inpage-moderate", scroll],
  ["S1c wheel in", (r) => r.scenario === "S1c" && /^ctrlwheel-in-/.test(r.name), zoom],
  ["S1c wheel out", (r) => r.scenario === "S1c" && /^ctrlwheel-out-/.test(r.name), zoom],
  ["S1c button in", (r) => r.scenario === "S1c" && /^ctrlbutton-in-/.test(r.name), zoom],
  ["S1c button out", (r) => r.scenario === "S1c" && /^ctrlbutton-out-/.test(r.name), zoom],
  ["S2 inpage fast", (r) => r.scenario === "S2" && r.name === "inpage-fast-50p", scroll],
  ["S2 wheel fast", (r) => r.scenario === "S2" && r.name === "wheel-fast-50p", scroll],
];

export const median = (xs) => {
  const s = xs.filter((x) => x != null && Number.isFinite(x)).sort((a, b) => a - b);
  if (!s.length) return null;
  const m = s.length >> 1;
  return s.length % 2 ? s[m] : (s[m - 1] + s[m]) / 2;
};

export const sideOf = (tag) => (/^base-/.test(tag) ? "base" : /^cand-/.test(tag) ? "cand" : null);

/**
 * Per-session values: Map<"dpr|cell|family", { base: number[], cand: number[], baseMissing, candMissing }>.
 * A session whose cell value is null (e.g. never became sharp) counts as missing.
 */
export function sessionValues(rows) {
  const bySession = new Map();
  for (const r of rows) {
    if (r.viewer !== "ours" || !sideOf(r.tag)) continue;
    if (!bySession.has(r.tag)) bySession.set(r.tag, []);
    bySession.get(r.tag).push(r);
  }
  const out = new Map();
  const put = (key, side, v) => {
    if (!out.has(key)) out.set(key, { base: [], cand: [], baseMissing: 0, candMissing: 0 });
    const e = out.get(key);
    if (v == null) e[`${side}Missing`] += 1; else e[side].push(v);
  };
  for (const [tag, rs] of [...bySession].sort(([a], [b]) => a.localeCompare(b))) {
    const side = sideOf(tag), dpr = rs[0].dpr;
    for (const [cell, sel, paths] of CELLS) {
      const trials = rs.filter(sel);
      if (!trials.length) continue;
      for (const [fam, p] of Object.entries(paths)) put(`${dpr}|${cell}|${fam}`, side, median(trials.map((t) => get(t, p))));
    }
    const rss = rs.map((t) => t.rss?.webMiB).filter((x) => x != null);
    put(`${dpr}|session|rss`, side, rss.length ? Math.max(...rss) : null);
  }
  return out;
}

/** Compare one metric: worse = larger for every family. */
export function compare(base, cand, family, baseMissing = 0, candMissing = 0) {
  const f = FAMILIES[family];
  const bm = median(base), cm = median(cand);
  const res = { family, base, cand, baseMed: bm, candMed: cm, status: "same", gap: null, threshold: null };
  if (candMissing > baseMissing) return { ...res, status: "regression", note: `candidate has no value in ${candMissing} session(s), base in ${baseMissing}` };
  if (base.length < 2 || cand.length < 2) return { ...res, status: "n/a", note: "fewer than 2 sessions with a value" };
  const range = (xs) => Math.max(...xs) - Math.min(...xs);
  const threshold = Math.max(range(base), f.floor, f.rel * Math.abs(bm));
  const worse = Math.min(...cand) - Math.max(...base);
  const better = Math.min(...base) - Math.max(...cand);
  res.threshold = threshold;
  if (worse > threshold) return { ...res, status: "regression", gap: worse };
  const improveThreshold = Math.max(range(cand), f.floor, f.rel * Math.abs(bm));
  if (better > improveThreshold) return { ...res, status: "improvement", gap: better, threshold: improveThreshold };
  if (Math.abs(cm - bm) > threshold) return { ...res, status: "watch", gap: cm - bm };
  return res;
}

/** { results: [{ dpr, cell, family, ...compare }], regressions, improvements, watch }. */
export function verdict(rows) {
  const results = [];
  for (const [key, e] of sessionValues(rows)) {
    const [dpr, cell, family] = key.split("|");
    results.push({ dpr: Number(dpr), cell, ...compare(e.base, e.cand, family, e.baseMissing, e.candMissing) });
  }
  const pick = (s) => results.filter((r) => r.status === s);
  return { results, regressions: pick("regression"), improvements: pick("improvement"), watch: pick("watch") };
}

const fmtN = (x) => (x == null ? "–" : Math.abs(x) >= 100 ? String(Math.round(x)) : String(Math.round(x * 10) / 10));
const fmtRange = (xs) => (xs.length ? `${fmtN(median(xs))} [${fmtN(Math.min(...xs))}–${fmtN(Math.max(...xs))}]` : "–");

/** Markdown table of one side-by-side result list. */
export function resultTable(results, { withBase = true } = {}) {
  const lines = [withBase ? "| DPR | cell | metric | base | candidate | threshold | status |" : "| DPR | cell | metric | candidate |",
    withBase ? "|---|---|---|---|---|---|---|" : "|---|---|---|---|"];
  const order = (r) => [r.dpr, CELLS.findIndex(([c]) => c === r.cell), Object.keys(FAMILIES).indexOf(r.family)];
  const sorted = [...results].sort((a, b) => { const x = order(a), y = order(b); return x[0] - y[0] || x[1] - y[1] || x[2] - y[2]; });
  for (const r of sorted) {
    const f = FAMILIES[r.family];
    const label = `${f.label}${f.unit ? ` (${f.unit})` : ""}`;
    if (!withBase) { lines.push(`| ${r.dpr} | ${r.cell} | ${label} | ${fmtRange(r.cand)} |`); continue; }
    const status = r.status === "regression" ? "**REGRESSION**"
      : r.status === "watch" ? `watch (${r.gap > 0 ? "worse" : "better"})` : r.status;
    lines.push(`| ${r.dpr} | ${r.cell} | ${label} | ${fmtRange(r.base)} | ${fmtRange(r.cand)} | ${fmtN(r.threshold)} | ${status}${r.note ? ` (${r.note})` : ""} |`);
  }
  return lines.join("\n");
}
