// Pure bench-summary math ported from graph-check.mjs (lines 980-1020). The
// impure orchestration (running each parser 3× and collecting per-file parse
// micros) lives in the comparison worker; these functions just reduce the
// collected samples into the report numbers (best-of-3 total, p50/p95/max,
// 5 slowest). Timings are parser-reported in-process parse micros.

export interface BenchSample {
  rel: string;
  micros: number;
}
export interface BenchFailure {
  rel: string;
  status: string;
  detail: string;
}
export interface BenchRun {
  samples: BenchSample[];
  failures: BenchFailure[];
  totalMicros: number;
}
export interface BenchSummary {
  totalMs: number;
  fileCount: number;
  p50Ms: number;
  p95Ms: number;
  maxMs: number;
  slowest: { rel: string; ms: number }[];
  failures: BenchFailure[];
}

export function microsToMs(micros: number): number {
  return Number(micros || 0) / 1000;
}

export function sum(values: number[]): number {
  return values.reduce((a, b) => a + b, 0);
}

export function percentile(sorted: number[], p: number): number {
  if (!sorted.length) return 0;
  return sorted[Math.min(sorted.length - 1, Math.max(0, Math.ceil(sorted.length * p) - 1))];
}

/** Reduce N repeated runs to a single summary: best (fastest) total run, its
 *  distribution, and its 5 slowest files. Mirrors graph-check's best-of-3. */
export function summarizeBenchRuns(runs: BenchRun[]): BenchSummary {
  const best = runs.slice().sort((a, b) => a.totalMicros - b.totalMicros)[0] || {
    samples: [],
    failures: [],
    totalMicros: 0,
  };
  const values = best.samples.map((s) => s.micros).sort((a, b) => a - b);
  const slowest = best.samples
    .slice()
    .sort((a, b) => b.micros - a.micros)
    .slice(0, 5);
  return {
    totalMs: microsToMs(best.totalMicros),
    fileCount: best.samples.length,
    p50Ms: microsToMs(percentile(values, 0.5)),
    p95Ms: microsToMs(percentile(values, 0.95)),
    maxMs: microsToMs(values[values.length - 1] || 0),
    slowest: slowest.map((s) => ({ rel: s.rel, ms: microsToMs(s.micros) })),
    failures: best.failures,
  };
}

/** Bucket per-file parser results into timing samples + failures (graph-check
 *  benchFromResults, 980-996). `overTimeout` results are treated as failures. */
export function benchFromResults(
  files: { id: string; rel: string }[],
  results: Map<string, { ok?: boolean; overTimeout?: boolean; parseMicros?: number; status?: string; detail?: string }>,
): BenchRun {
  const samples: BenchSample[] = [];
  const failures: BenchFailure[] = [];
  for (const file of files) {
    const res = results.get(file.id);
    if (res?.ok && !res.overTimeout) {
      samples.push({ rel: file.rel, micros: Number(res.parseMicros || 0) });
    } else {
      failures.push({
        rel: file.rel,
        status: res?.status || (res?.overTimeout ? "timeout" : "failed"),
        detail: res?.detail || (res?.overTimeout ? "parse time exceeded timeout" : "parser failed"),
      });
    }
  }
  return { samples, failures, totalMicros: sum(samples.map((s) => s.micros)) };
}
