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
