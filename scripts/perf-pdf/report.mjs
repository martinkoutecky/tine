// Aggregate harness JSONL rows into markdown tables: median [min–max] over reps.
//   node scripts/perf-pdf/report.mjs <rawDir> [prefix,prefix] [dropRegex] > tables.md
import fs from "node:fs";
import path from "node:path";

const dir = process.argv[2];
// argv[3]: comma-separated tag prefixes to include; argv[4]: regex over
// "<tag> <scenario> <name>" for rows to drop (e.g. superseded trials).
const filters = (process.argv[3] || "").split(",");
const drop = process.argv[4] ? new RegExp(process.argv[4]) : null;
const rows = [];
for (const f of fs.readdirSync(dir)) {
  if (!f.endsWith(".jsonl") || !filters.some((x) => f.startsWith(x))) continue;
  for (const line of fs.readFileSync(path.join(dir, f), "utf8").split("\n")) {
    if (!line.trim()) continue;
    const r = JSON.parse(line);
    // PERF_LOADMAX: keep only trials whose 1-min load at start was <= it.
    const loadMax = Number(process.env.PERF_LOADMAX || Infinity);
    if ((!drop || !drop.test(`${r.tag} ${r.scenario} ${r.name}`)) && !(r.load?.[0]?.[0] > loadMax)) rows.push(r);
  }
}
const get = (o, p) => p.split(".").reduce((a, k) => (a == null ? a : a[k]), o);
const med = (xs) => { const s = xs.filter((x) => x != null && Number.isFinite(x)).sort((a, b) => a - b); return s.length ? s[(s.length - 1) >> 1] : null; };
const fmt = (xs) => {
  const s = xs.filter((x) => x != null && Number.isFinite(x)).sort((a, b) => a - b);
  if (!s.length) return xs.length ? "n/a" : "";
  const r = (x) => (Math.abs(x) >= 100 ? Math.round(x) : Math.round(x * 10) / 10);
  const miss = xs.length - s.length;
  return s.length === 1 ? `${r(s[0])}${miss ? ` (+${miss} n/a)` : ""}` : `${r(med(s))} [${r(s[0])}–${r(s.at(-1))}]${miss ? ` +${miss} n/a` : ""}`;
};
function table(title, select, cols) {
  const groups = new Map();
  for (const r of rows.filter(select)) {
    // S1c's three in/out cycles pool into one cell (n = 3 cycles x sessions).
    const name = r.scenario === "S1c" ? r.name.replace(/-\d+$/, "") : r.name;
    const k = `${r.scenario} ${name}|${r.dpr}|${r.theme}|${r.viewer}`;
    if (!groups.has(k)) groups.set(k, []);
    groups.get(k).push(r);
  }
  if (!groups.size) return;
  console.log(`\n#### ${title}\n`);
  console.log(`| trial | DPR | theme | viewer | n | ${cols.map((c) => c[0]).join(" | ")} |`);
  console.log(`|---|---|---|---|---|${cols.map(() => "---").join("|")}|`);
  const keys = [...groups.keys()].sort();
  for (const k of keys) {
    const g = groups.get(k);
    const [name, dpr, theme, viewer] = k.split("|");
    console.log(`| ${name} | ${dpr} | ${theme} | ${viewer} | ${g.length} | ${cols.map(([, fn]) => fmt(g.flatMap((r) => { const v = typeof fn === "function" ? fn(r) : get(r, fn); return Array.isArray(v) ? v : [v]; }))).join(" | ")} |`);
  }
}
const errors = rows.filter((r) => r.error);
console.log(`rows: ${rows.length}, errored trials: ${errors.length}`);
for (const e of errors) console.log(`- ERROR ${e.tag} ${e.viewer} ${e.scenario} ${e.name}: ${String(e.error).split("\n")[0]}`);
const loads = rows.map((r) => r.load?.[0]?.[0]).filter((x) => x != null);
console.log(`load (1-min) at trial start: median ${med(loads)}, min ${Math.min(...loads)}, max ${Math.max(...loads)}; trials started at load > 4: ${loads.filter((x) => x > 4).length}/${loads.length}`);

table("Open (time from click / open() call)", (r) => r.scenario === "open", [
  ["first raster ms", "openFirstRasterMs"], ["viewport sharp ms", "openSettledMs"], ["first text layer ms", "openFirstTextMs"],
  ["blank ms", "whole.blankMs"], ["max frame ms", "whole.max"], ["canvas MPx", "whole.canvasMPxEnd"],
  ["web RSS peak MiB", "rss.webMiB"], ["web RSS growth MiB", "rss.webDeltaMiB"], ["main RSS peak MiB", "rss.mainMiB"], ["main RSS growth MiB", "rss.mainDeltaMiB"]]);
table("S0 sanity (idle 2 s; instant jump to unrendered page 15)", (r) => r.scenario === "S0", [
  ["idle p50 frame", "idle.p50"], ["idle blank ms", "idle.blankMs"], ["idle frames>50ms", "idle.over50"],
  ["jump→sharp ms", "jumpToSharpMs"], ["jump blank ms", "whole.blankMs"], ["jump peak blank %", "whole.peakBlankPct"], ["sampler ms/frame p50", "samplerMsP50"]]);
const scroll = [["achieved px/s", "speedPxS"], ["p50 frame", "during.p50"], ["p95 frame", "during.p95"], ["max frame", "during.max"],
  [">50ms", "during.over50"], [">100ms", "during.over100"], ["long-frame ms", "during.longMs"],
  ["blank ms (during)", "during.blankMs"], ["peak blank %", "during.peakBlankPct"], ["stale ms", "during.staleMs"],
  ["end→sharp ms", "toSharpAfterEndMs"], ["blank ms (after end)", "after.blankMs"], ["canvas MPx peak", "during.canvasMPxPeak"], ["web RSS peak MiB", "rss.webMiB"], ["web RSS growth MiB", "rss.webDeltaMiB"]];
table("S1a fit-width scroll top→bottom (math paper)", (r) => r.scenario === "S1a", scroll);
table("S1b zoomed column reading (5 column visits: jump, pan 300px across, 1 viewport down, back, 1 down)", (r) => r.scenario === "S1b", [
  ["zoom (CSS px/pt)", "scale"], ["jump→sharp ms (all jumps)", "jumpToSharpMs"], ["pan blank ms", "panBlankMs"], ["pan stale ms", "panStaleMs"],
  ["pan p95 frame (worst pan)", "panP95"], ["pan max frame", "panMax"], ["whole blank ms", "whole.blankMs"], ["whole >100ms", "whole.over100"],
  ["canvas MPx peak", "whole.canvasMPxPeak"], ["web RSS peak MiB", "rss.webMiB"]]);
table("S1c zoom bursts at mid-page (5 inputs, 60 ms apart)", (r) => r.scenario === "S1c", [
  ["scale after", "scaleAfter"], ["first visible change ms", "firstChangeMs"], ["last input→sharp ms", "sharpFromLastMs"], ["first input→sharp ms", "sharpFromFirstMs"],
  ["anchor drift px", "anchorDriftPx"], ["blank ms", "whole.blankMs"], ["peak blank %", "whole.peakBlankPct"], ["max frame", "whole.max"], [">100ms", "whole.over100"]]);
table("S2 long document (714 pp): fast scroll over 50 pages", (r) => r.scenario === "S2" && /50p/.test(r.name), scroll);
table("S2 jumps (page field 400 + Enter; outline 'III Algorithms' → p. 469)", (r) => r.scenario === "S2" && /jump/.test(r.name), [
  ["first change ms", "firstChangeMs"], ["jump→sharp ms", "jumpToSharpMs"], ["blank ms", "whole.blankMs"], ["max frame", "whole.max"]]);
table("S3 scanned book (748 pp, 34 MB, JPX scans): scroll (moderate 20 pp, fast 50 pp)", (r) => r.scenario === "S3", scroll);
