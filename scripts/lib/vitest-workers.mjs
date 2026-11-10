// The vitest worker cap for every config (vitest.config.ts, vitest.render.config.ts; the CM6 and storage configs
// spread those). TINE_VITEST_MAX_WORKERS=<n> caps the forked workers; unset or empty keeps vitest's default
// (cores - 1), so CI is unchanged. A shared build host sets it: each worker holds 200-850 MB, and 11 workers per
// run times several concurrent runs exhausted a 24 GB slice (2026-11-09).
export function vitestMaxWorkers(env = process.env) {
  const raw = (env.TINE_VITEST_MAX_WORKERS ?? "").trim();
  if (raw === "") return {};
  const n = Number(raw);
  if (!Number.isInteger(n) || n < 1) throw new Error(`TINE_VITEST_MAX_WORKERS must be a positive integer, got "${raw}"`);
  return { maxWorkers: n };
}

// The V8 old-space cap per forked worker for the jsdom and CM6 configs (D-20261110-ML). Node lets the heap grow
// before it collects, so a jsdom/CM6 worker that needs 0.6-1 GB live data balloons to 1.7-2.0 GB; capping the old
// space makes it collect earlier. Set through the pool's execArgv, so it reaches only the test workers (a global
// NODE_OPTIONS would also slow tsc and vite). TINE_VITEST_HEAP_MB=<n> overrides it; "0" turns it off.
export const VITEST_WORKER_HEAP_MB = 1536;
export function vitestWorkerHeap(env = process.env) {
  const raw = (env.TINE_VITEST_HEAP_MB ?? "").trim();
  const mb = raw === "" ? VITEST_WORKER_HEAP_MB : Number(raw);
  if (!Number.isInteger(mb) || mb < 0) throw new Error(`TINE_VITEST_HEAP_MB must be a non-negative integer, got "${raw}"`);
  if (mb === 0) return {};
  return { poolOptions: { forks: { execArgv: [`--max-old-space-size=${mb}`] } } };
}
