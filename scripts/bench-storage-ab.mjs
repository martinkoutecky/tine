#!/usr/bin/env node
// Storage A/B performance harness (STEP3-DESIGN §13, "gate 2"): a baseline and a
// candidate Tine binary, interleaved trial by trial on private copies of the same
// graph, medians and p95 per metric, and a measured A/A noise floor.
//
//   xvfb-run -a -s "-screen 0 1920x1080x24" env LANG=C.UTF-8 nice -n 5 \
//     node scripts/bench-storage-ab.mjs --baseline <bin> --candidate <bin> \
//       [--baseline-receipt <json>] [--candidate-receipt <json>] \
//       [--runs 7] [--scenarios default|all|a,b,..] [--noise-floor <aa/summary.json>]
//   ... --aa <bin>        the same binary on both arms (the A/A validation run)
//   ... --session-minutes <n> --session-runs <n>   the `session` scenario (spec: 30 minutes; 1 run by default)
//   ... --list            print scenarios and metrics, run nothing
//   ... --summarize-only  recompute summary.json / comparison.md from --out
//
// The protocol, the candidate bench-event format and what each metric means:
// docs/bench-storage-ab.md. This script replaces the ddf408c55 master-reference
// shape of scripts/bench-og-parity.mjs (kept as is for the og-vs-master parity run).
import fs from "node:fs";
import crypto from "node:crypto";
import path from "node:path";
import os from "node:os";
import { fileURLToPath } from "node:url";
import { setTimeout as sleep } from "node:timers/promises";
import { generateRealisticGraph } from "./generate-realistic-graph.mjs";
import { Session, findTauriDriver } from "./lib/bench-ab-session.mjs";
import { describe, medianSpreadPct, deltaPct, verdict, NOISY_PCT, fmt } from "./lib/bench-ab-stats.mjs";
import { SCENARIOS, DEFAULT_SCENARIOS, METRICS, metricInfo, writeFixtures } from "./lib/bench-ab-scenarios.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

function usage(code = 2) {
  console.error(fs.readFileSync(fileURLToPath(import.meta.url), "utf8").split("\n").slice(1, 20).map((l) => l.replace(/^\/\/ ?/, "")).join("\n"));
  process.exit(code);
}

const argv = process.argv.slice(2);
const opts = { runs: 7, scenarios: "default", out: path.join(ROOT, "test-results/storage-ab"), corpus: "anonymized",
  anon: process.env.TINE_AB_ANON || path.join(os.homedir(), "research/logseq-anonymized"), sessionMinutes: 30, pilot: false };
const flags = new Set(["--list", "--summarize-only", "--pilot", "--help"]);
for (let i = 0; i < argv.length; i++) {
  const a = argv[i];
  if (!a.startsWith("--")) usage();
  const key = a.slice(2).replace(/-([a-z])/g, (_, c) => c.toUpperCase());
  if (flags.has(a)) { opts[key] = true; continue; }
  if (i + 1 >= argv.length) usage();
  opts[key] = argv[++i];
}
if (opts.help) usage(0);
opts.runs = Number(opts.runs);
opts.sessionMinutes = Number(opts.sessionMinutes);
if (opts.list) {
  console.log("scenarios (default set marked *):");
  for (const [id, s] of Object.entries(SCENARIOS)) console.log(`  ${DEFAULT_SCENARIOS.includes(id) ? "*" : " "} ${id.padEnd(11)} ${s.title}${s.candidateOnly ? "  [candidate-only parts]" : ""}`);
  console.log("\nmetrics:");
  for (const [id, m] of Object.entries(METRICS)) console.log(`  ${id.padEnd(34)} ${m.unit.padEnd(5)} ${m.candidateOnly ? "candidate-only  " : "                "}${m.desc}`);
  process.exit(0);
}
if (!Number.isInteger(opts.runs) || opts.runs < (opts.pilot ? 1 : 5)) throw new Error("the stability budget needs at least 5 runs per arm (--pilot allows fewer, for harness debugging only)");
if (process.platform !== "linux") throw new Error("bench-storage-ab is Linux only");
if (!process.env.DISPLAY && !opts.summarizeOnly) throw new Error("start under xvfb-run -a -s \"-screen 0 1920x1080x24\"");

const sha256 = (file) => crypto.createHash("sha256").update(fs.readFileSync(file)).digest("hex");

/** An arm: the binary copied into the output dir, its receipt (if any) and its publish adapter. */
function makeArm(id, binary, receiptPath, adapter) {
  if (!binary) throw new Error(`--${id} <binary> is required (or --aa <binary>)`);
  const source = path.resolve(binary);
  if (!fs.existsSync(source)) throw new Error(`missing binary: ${source}`);
  const dest = path.join(opts.outAbs, "binaries", id, "tine");
  fs.mkdirSync(path.dirname(dest), { recursive: true });
  fs.copyFileSync(source, dest);
  fs.chmodSync(dest, 0o755);
  const digest = sha256(dest);
  const receiptFile = receiptPath ? path.resolve(receiptPath) : fs.existsSync(`${source}.build.json`) ? `${source}.build.json` : null;
  let receipt = null;
  if (receiptFile) {
    try { receipt = JSON.parse(fs.readFileSync(receiptFile, "utf8")); } catch (error) { throw new Error(`unreadable build receipt ${receiptFile}: ${error.message}`); }
  }
  const verified = receipt ? receipt.appSha256 === digest : false;
  if (receipt && !verified) console.warn(`WARNING: ${id} receipt ${receiptFile} does not match the binary's SHA-256; its revision is reported as unverified`);
  return { id, binary: dest, source, sha256: digest, adapter, receiptFile, sourceRevision: receipt ? (verified ? receipt.sourceRevision : `unverified (${receipt.sourceRevision})`) : "unrecorded (no receipt)" };
}

fs.mkdirSync(path.resolve(opts.out), { recursive: true });
opts.outAbs = fs.realpathSync(path.resolve(opts.out)); // the traced paths must match what the app sees

function scenarioList() {
  if (opts.scenarios === "default") return DEFAULT_SCENARIOS;
  if (opts.scenarios === "all") return Object.keys(SCENARIOS);
  const ids = opts.scenarios.split(",").filter(Boolean);
  for (const id of ids) if (!(id in SCENARIOS)) throw new Error(`unknown scenario ${id}; --list shows them`);
  return ids;
}

/** The corpus: a COPY of the anonymized graph (never the original) plus the bench fixtures. */
async function prepareCorpus() {
  const dest = path.join(opts.outAbs, "corpora", opts.corpus);
  if (fs.existsSync(path.join(dest, ".ab-bench-ready"))) return dest;
  fs.rmSync(dest, { recursive: true, force: true });
  fs.mkdirSync(dest, { recursive: true });
  if (opts.corpus === "anonymized") {
    const anon = path.resolve(opts.anon);
    if (!fs.existsSync(anon)) throw new Error(`missing anonymized graph: ${anon}`);
    if (path.resolve(opts.outAbs).startsWith(anon + path.sep)) throw new Error("--out must not be inside the anonymized graph");
    fs.cpSync(anon, dest, { recursive: true, dereference: true });
  } else if (opts.corpus === "2k" || opts.corpus === "10k") {
    await generateRealisticGraph({ root: dest, ...(opts.corpus === "2k" ? { pages: 1400, journals: 600 } : { pages: 7000, journals: 3000 }), seed: 543 });
  } else {
    throw new Error(`unknown --corpus ${opts.corpus} (anonymized | 2k | 10k)`);
  }
  writeFixtures(dest);
  fs.writeFileSync(path.join(dest, ".ab-bench-ready"), "fixtures v1\n");
  return dest;
}

const trialFiles = (dir) => (fs.existsSync(dir) ? fs.readdirSync(dir).filter((n) => fs.existsSync(path.join(dir, n, "result.json"))) : []);

async function runTrial(arm, scenarioId, run, corpusDir, tauriDriver) {
  const scenario = SCENARIOS[scenarioId];
  const timeoutMs = scenario.timeoutMs(opts);
  const s = new Session({ arm, group: scenarioId, run, corpusDir, outDir: opts.outAbs, repoRoot: ROOT, tauriDriver,
    webDriver: process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver", timeoutMs });
  s.prepare();
  const result = { arm: arm.id, scenario: scenarioId, run, loadAvgBefore: os.loadavg()[0], metrics: {}, series: {}, notes: {}, failures: {} };
  let watchdog;
  try {
    await Promise.race([
      scenario.run(s, opts),
      new Promise((_, reject) => { watchdog = setTimeout(() => { s.timedOut = true; s.shot("timeout"); reject(new Error(`trial exceeded ${timeoutMs} ms`)); }, timeoutMs); }),
    ]);
  } catch (error) {
    s.failures.trial = String(error?.stack ?? error).slice(0, 800);
    if (!s.timedOut) s.shot("failure");
  } finally {
    clearTimeout(watchdog);
    await s.stop();
  }
  Object.assign(result, { metrics: s.metrics, series: s.series, notes: s.notes, failures: s.failures, probeMode: s.probeMode, loadAvgAfter: os.loadavg()[0] });
  fs.writeFileSync(path.join(s.dir, "result.json"), JSON.stringify(result, null, 2) + "\n");
  return result;
}

// -- summary -------------------------------------------------------------------
function collect(results, armId, metric) {
  return results.filter((r) => r.arm === armId).map((r) => r.metrics[metric]).filter(Number.isFinite);
}
function collectSeries(results, armId, name) {
  return results.filter((r) => r.arm === armId).flatMap((r) => r.series[name] ?? []);
}

function summarize(results, arms, noise) {
  const summary = { metrics: {}, series: {} };
  const names = new Set(results.flatMap((r) => Object.keys(r.metrics)));
  for (const name of Object.keys(METRICS)) names.add(name);
  const seriesNames = new Set(results.flatMap((r) => Object.keys(r.series)));
  for (const name of [...names].sort()) {
    const cells = Object.fromEntries(arms.map((a) => [a.id, describe(collect(results, a.id, name))]));
    const info = metricInfo(name);
    const base = cells.baseline;
    const cand = cells.candidate;
    const spread = medianSpreadPct(base, cand);
    const floor = noise?.summary?.metrics?.[name]?.spreadPct ?? null;
    summary.metrics[name] = { unit: info.unit, candidateOnly: !!info.candidateOnly, cells, spreadPct: spread, deltaPct: deltaPct(base, cand),
      noisy: spread != null && spread > NOISY_PCT,
      verdict: verdict({ delta: deltaPct(base, cand), floor, lowerIsBetter: !info.neutral, absDelta: base && cand ? cand.median - base.median : null, absFloor: info.absFloor }) };
  }
  for (const name of [...seriesNames].sort()) {
    const cells = Object.fromEntries(arms.map((a) => [a.id, describe(collectSeries(results, a.id, name))]));
    const spread = medianSpreadPct(cells.baseline, cells.candidate);
    summary.series[name] = { unit: "ms", cells, spreadPct: spread, deltaPct: deltaPct(cells.baseline, cells.candidate), noisy: spread != null && spread > NOISY_PCT };
  }
  return summary;
}

function renderMarkdown(report) {
  const { mode, arms, summary, runs } = report;
  const aa = mode === "A/A";
  const lines = [`# Storage A/B bench (${mode})`, "",
    ...arms.map((a) => `- ${a.id}: ${a.source} sha256 ${a.sha256.slice(0, 16)}... revision ${a.sourceRevision}; publish signal: ${a.adapter === "ipc" ? "save_pages IPC response" : "bench events file (TINE_BENCH_EVENTS)"}`),
    `- runs per arm: ${runs}; corpus: ${report.corpus}; scenarios: ${report.scenarios.join(", ")}; probe: ${report.probe}`,
    `- load average at start: ${report.loadAvgStart.toFixed(2)}; at end: ${report.loadAvgEnd.toFixed(2)}`, "",
    aa ? "A/A spread = |median(A) - median(B)| / mean of the two medians, in percent; a metric above 10% needs more runs or a better signal."
      : "delta = candidate median vs baseline median; verdict uses the A/A floor from --noise-floor when given.", "",
    "| Metric | unit | baseline med [min,max] n | candidate med [min,max] n | p95 base / cand | delta % | A/B-or-A/A spread % | verdict |", "|---|---|---|---|---|---|---|---|"];
  const cell = (c) => (c ? `${fmt(c.median, 2)} [${fmt(c.min, 2)}, ${fmt(c.max, 2)}] n=${c.n}` : "-");
  const unmeasured = [];
  for (const [name, m] of Object.entries(summary.metrics)) {
    const c = m.cells;
    const none = !c.baseline && !c.candidate;
    if (none) { unmeasured.push(m.candidateOnly ? `${name} (candidate-only)` : name); continue; }
    const note = m.candidateOnly && !c.baseline ? "candidate-only" : m.verdict;
    lines.push(`| ${name} | ${m.unit} | ${cell(c.baseline)} | ${cell(c.candidate)} | ${fmt(c.baseline?.p95, 2)} / ${fmt(c.candidate?.p95, 2)} | ${fmt(m.deltaPct)} | ${fmt(m.spreadPct)}${m.noisy ? " NOISY" : ""} | ${note} |`);
  }
  lines.push("", "## Pooled samples (all keystrokes / calls across runs)", "", "| Series | baseline p50 / p95 (n) | candidate p50 / p95 (n) | spread % |", "|---|---|---|---|");
  for (const [name, m] of Object.entries(summary.series)) {
    const f = (c) => (c ? `${fmt(c.median, 2)} / ${fmt(c.p95, 2)} (${c.n})` : "-");
    lines.push(`| ${name} | ${f(m.cells.baseline)} | ${f(m.cells.candidate)} | ${fmt(m.spreadPct)}${m.noisy ? " NOISY" : ""} |`);
  }
  lines.push("", "## Metrics with no samples in this run", "", unmeasured.length ? unmeasured.join(", ") : "none");
  const failed = report.failures;
  lines.push("", "## Failures", "", failed.length ? failed.map((f) => `- ${f.arm} ${f.scenario} #${f.run}: ${Object.entries(f.failures).map(([k, v]) => `${k}: ${String(v).split("\n")[0]}`).join("; ")}`).join("\n") : "none");
  return lines.join("\n") + "\n";
}

// -- main ------------------------------------------------------------------------
const loadAvgStart = os.loadavg()[0];
let arms;
if (opts.summarizeOnly) {
  arms = JSON.parse(fs.readFileSync(path.join(opts.outAbs, "summary.json"), "utf8")).arms;
} else if (opts.aa) {
  arms = [makeArm("baseline", opts.aa, opts.baselineReceipt, opts.baselineAdapter || "ipc"), makeArm("candidate", opts.aa, opts.baselineReceipt, opts.candidateAdapter || "ipc")];
} else {
  arms = [makeArm("baseline", opts.baseline, opts.baselineReceipt, opts.baselineAdapter || "ipc"),
    makeArm("candidate", opts.candidate, opts.candidateReceipt, opts.candidateAdapter || "events")];
}
const mode = opts.aa || (arms[0].sha256 === arms[1].sha256 && arms[0].adapter === arms[1].adapter) ? "A/A" : "A/B";
const scenarios = opts.summarizeOnly ? JSON.parse(fs.readFileSync(path.join(opts.outAbs, "summary.json"), "utf8")).scenarios : scenarioList();
const results = [];
if (opts.summarizeOnly) {
  const dir = path.join(opts.outAbs, "trials");
  for (const name of trialFiles(dir)) results.push(JSON.parse(fs.readFileSync(path.join(dir, name, "result.json"), "utf8")));
} else {
  const corpusDir = await prepareCorpus();
  const tauriDriver = findTauriDriver(ROOT);
  for (const arm of arms) fs.writeFileSync(path.join(opts.outAbs, `arm-${arm.id}.json`), JSON.stringify(arm, null, 2) + "\n");
  for (let run = 1; run <= opts.runs; run++) {
    for (const id of scenarios) {
      const scenario = SCENARIOS[id];
      if (run > scenario.maxRuns(opts)) continue;
      // Alternate which arm goes first so drift hits both equally.
      for (const arm of (run % 2 ? arms : [...arms].reverse())) {
        const result = await runTrial(arm, id, run, corpusDir, tauriDriver);
        results.push(result);
        const bad = Object.keys(result.failures).length;
        console.log(`${id} ${arm.id} #${run}: ${bad ? `FAIL ${Object.entries(result.failures).map(([k, v]) => `${k}: ${String(v).split("\n")[0].slice(0, 160)}`).join(" | ")}` : "ok"} ${JSON.stringify(result.metrics).slice(0, 400)}`);
      }
    }
  }
}
const noise = opts.noiseFloor ? JSON.parse(fs.readFileSync(path.resolve(opts.noiseFloor), "utf8")) : null;
const summary = summarize(results, arms, noise);
const report = { schemaVersion: 1, mode, arms, runs: opts.runs, corpus: opts.corpus, scenarios, probe: "rAF gap over 100 ms where PerformanceObserver longtask is unsupported",
  loadAvgStart, loadAvgEnd: os.loadavg()[0], noiseFloorFrom: opts.noiseFloor ?? null, summary,
  failures: results.filter((r) => Object.keys(r.failures).length).map((r) => ({ arm: r.arm, scenario: r.scenario, run: r.run, failures: r.failures })) };
fs.writeFileSync(path.join(opts.outAbs, "summary.json"), JSON.stringify(report, null, 2) + "\n");
fs.writeFileSync(path.join(opts.outAbs, "comparison.md"), renderMarkdown(report));
console.log(`wrote ${path.join(opts.outAbs, "comparison.md")}`);
if (report.failures.length) process.exitCode = 1;
await sleep(0);
