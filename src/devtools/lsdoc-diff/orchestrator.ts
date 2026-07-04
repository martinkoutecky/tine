// The diff/bench orchestrator — drives the whole "Help improve Tine" run:
// enumerate graph files (backend), parse each with lsdoc (main thread) and mldoc
// (worker), compare canonical projections, and for every mismatch re-verify in a
// FRESH mldoc realm, shrink to the smallest divergent range, and produce an
// anonymized-and-re-verified snippet. This is the faithful in-app analog of
// graph-check.mjs's runDiff/runBench; the pure logic lives in the engine modules.
import { backend } from "../../backend";
import type { GraphSourceFile } from "../../backend";
import { MldocClient, type Format, type Projection } from "./mldoc-client";
import { lsdocDocumentAvailable, parseLsdocDocument } from "./lsdoc-document";
import { projectionKey } from "./projection";
import { minimize, toBytes } from "./minimize";
import { anonymizeAndVerify } from "./anonymize";
import { benchFromResults, summarizeBenchRuns, type BenchRun, type BenchSummary } from "./bench";

export interface DiffOptions {
  mode: "diff" | "bench" | "both";
  includeJournals: boolean;
  fast: boolean; // scan with a warm worker only (non-authoritative absences)
  timeoutMs: number;
}

export type Finding =
  | {
      type: "divergence";
      rel: string;
      lineStart: number;
      lineEnd: number;
      contextDependent: boolean;
      anonymized:
        | { ok: true; tier: string; input: string; lsdocKey: string; mldocKey: string }
        | { ok: false };
    }
  | { type: "mldoc-failure"; rel: string; status: string; detail: string }
  | { type: "unstable-divergence"; rel: string };

export interface DiffReport {
  stats: { files: number; totalBytes: number };
  lsdocAvailable: boolean;
  bench?: { lsdoc: BenchSummary | null; mldoc: BenchSummary };
  findings?: Finding[];
}

export interface ProgressEvent {
  phase: "scan" | "verify" | "bench";
  done: number;
  total: number;
  current?: string;
}

const BENCH_RUNS = 3; // best-of-3, like graph-check

interface PairResult {
  ok: boolean;
  diverges: boolean;
  lsdocProjection?: Projection;
  mldocProjection?: Projection;
}

export async function runComparison(
  opts: DiffOptions,
  onProgress: (e: ProgressEvent) => void,
): Promise<DiffReport> {
  // Screenshot/dev hook: a preloaded fixture lets the harness render the panel's
  // populated state without a live mldoc+lsdoc run. Never set in a real build.
  const fixture = (globalThis as unknown as { __tineDiffFixture?: DiffReport }).__tineDiffFixture;
  if (fixture) return fixture;

  const files = await backend().graphSourceFiles(opts.includeJournals);
  const stats = { files: files.length, totalBytes: files.reduce((n, f) => n + f.bytes, 0) };
  const lsdocAvailable = lsdocDocumentAvailable();
  const client = new MldocClient();

  // Fresh (authoritative) both-parser parse — feeds re-verify, minimize, anon.
  const parseBothFresh = async (text: string, format: Format): Promise<PairResult> => {
    if (!lsdocAvailable) return { ok: false, diverges: false };
    let lsdocProjection: Projection;
    try {
      lsdocProjection = parseLsdocDocument(text, format === "org");
    } catch {
      return { ok: false, diverges: false };
    }
    const m = await client.parseFresh(text, format, opts.timeoutMs);
    if (!m.ok) return { ok: false, diverges: false };
    return {
      ok: true,
      diverges: projectionKey(lsdocProjection) !== projectionKey(m.projection),
      lsdocProjection,
      mldocProjection: m.projection,
    };
  };

  try {
    let bench: DiffReport["bench"];
    let findings: Finding[] | undefined;

    if (opts.mode === "bench" || opts.mode === "both") {
      bench = await runBench(client, files, opts, onProgress);
    }
    if (opts.mode === "diff" || opts.mode === "both") {
      findings = lsdocAvailable ? await runDiff(client, files, opts, parseBothFresh, onProgress) : [];
    }

    return { stats, lsdocAvailable, bench, findings };
  } finally {
    client.dispose();
  }
}

async function runDiff(
  client: MldocClient,
  files: GraphSourceFile[],
  opts: DiffOptions,
  parseBothFresh: (text: string, format: Format) => Promise<PairResult>,
  onProgress: (e: ProgressEvent) => void,
): Promise<Finding[]> {
  // Stage 1 — fast scan (warm worker) to find candidate mismatches.
  const candidates: GraphSourceFile[] = [];
  const findings: Finding[] = [];
  for (let i = 0; i < files.length; i++) {
    const f = files[i];
    onProgress({ phase: "scan", done: i, total: files.length, current: f.rel });
    let lsdocKey: string;
    try {
      lsdocKey = projectionKey(parseLsdocDocument(f.text, f.format === "org"));
    } catch (e) {
      findings.push({ type: "mldoc-failure", rel: f.rel, status: "lsdoc-error", detail: String(e).split("\n")[0] });
      continue;
    }
    const m = await client.parseWarm(f.text, f.format, opts.timeoutMs);
    if (!m.ok) {
      findings.push({ type: "mldoc-failure", rel: f.rel, status: m.status, detail: m.detail });
      continue;
    }
    if (lsdocKey !== projectionKey(m.projection)) candidates.push(f);
  }

  // Stage 2 — re-verify each candidate in fresh realms, minimize, anonymize.
  for (let i = 0; i < candidates.length; i++) {
    const f = candidates[i];
    onProgress({ phase: "verify", done: i, total: candidates.length, current: f.rel });
    const original = await parseBothFresh(f.text, f.format);
    if (!original.ok || !original.diverges) {
      findings.push({ type: "unstable-divergence", rel: f.rel });
      continue;
    }
    const buf = toBytes(f.text);
    const min = await minimize(buf, f.format, (t, fmt) => parseBothFresh(t, fmt));
    const anon = await anonymizeAndVerify<Projection>(min.input, (candidate) => parseBothFresh(candidate, f.format));
    findings.push({
      type: "divergence",
      rel: f.rel,
      lineStart: min.lineStart,
      lineEnd: min.lineEnd,
      contextDependent: min.contextDependent,
      anonymized: anon.ok
        ? {
            ok: true,
            tier: anon.tier!,
            input: anon.input!,
            lsdocKey: projectionKey(anon.lsdocProjection),
            mldocKey: projectionKey(anon.mldocProjection),
          }
        : { ok: false },
    });
  }
  return findings;
}

async function runBench(
  client: MldocClient,
  files: GraphSourceFile[],
  opts: DiffOptions,
  onProgress: (e: ProgressEvent) => void,
): Promise<DiffReport["bench"]> {
  const lsdocAvailable = lsdocDocumentAvailable();
  const idFiles = files.map((f, i) => ({ id: `b${i}`, rel: f.rel }));

  // mldoc: warm worker timing (reused; matches graph-check's benchMldoc).
  const mldocRuns: BenchRun[] = [];
  for (let run = 0; run < BENCH_RUNS; run++) {
    const results = new Map<string, { ok: boolean; parseMicros?: number; status?: string; detail?: string; overTimeout?: boolean }>();
    for (let i = 0; i < files.length; i++) {
      const f = files[i];
      onProgress({ phase: "bench", done: run * files.length + i, total: BENCH_RUNS * files.length, current: `mldoc ${f.rel}` });
      const m = await client.parseWarm(f.text, f.format, opts.timeoutMs);
      results.set(idFiles[i].id, m.ok ? { ok: true, parseMicros: m.parseMicros } : { ok: false, status: m.status, detail: m.detail });
    }
    mldocRuns.push(benchFromResults(idFiles, results));
  }

  // lsdoc: main-thread timing (only if the whole-file parser is wired).
  let lsdocSummary: BenchSummary | null = null;
  if (lsdocAvailable) {
    const lsdocRuns: BenchRun[] = [];
    for (let run = 0; run < BENCH_RUNS; run++) {
      const results = new Map<string, { ok: boolean; parseMicros?: number; status?: string; detail?: string }>();
      for (const [i, f] of files.entries()) {
        const start = performance.now();
        try {
          parseLsdocDocument(f.text, f.format === "org");
          results.set(idFiles[i].id, { ok: true, parseMicros: Math.round((performance.now() - start) * 1000) });
        } catch (e) {
          results.set(idFiles[i].id, { ok: false, status: "lsdoc-error", detail: String(e).split("\n")[0] });
        }
      }
      lsdocRuns.push(benchFromResults(idFiles, results));
    }
    lsdocSummary = summarizeBenchRuns(lsdocRuns);
  }

  return { lsdoc: lsdocSummary, mldoc: summarizeBenchRuns(mldocRuns) };
}
