import assert from "node:assert/strict";
import fs from "node:fs";
import test from "node:test";
import { vitestMaxWorkers, vitestWorkerHeap, VITEST_WORKER_HEAP_MB } from "./vitest-workers.mjs";

test("unset or empty keeps vitest's default", () => {
  assert.deepEqual(vitestMaxWorkers({}), {});
  assert.deepEqual(vitestMaxWorkers({ TINE_VITEST_MAX_WORKERS: " " }), {});
});

test("a positive integer caps the workers; anything else is refused", () => {
  assert.deepEqual(vitestMaxWorkers({ TINE_VITEST_MAX_WORKERS: "4" }), { maxWorkers: 4 });
  for (const bad of ["0", "-2", "2.5", "four"]) assert.throws(() => vitestMaxWorkers({ TINE_VITEST_MAX_WORKERS: bad }));
});

test("every vitest config takes the cap, directly or by spreading one that does", () => {
  for (const file of ["vitest.config.ts", "vitest.render.config.ts"]) {
    assert.match(fs.readFileSync(file, "utf8"), /\.\.\.vitestMaxWorkers\(\)/, file);
  }
  assert.match(fs.readFileSync("vitest.cm6.config.ts", "utf8"), /import renderConfig from "\.\/vitest\.render\.config"[\s\S]*test: \{\s*\.\.\.test,/);
  assert.match(fs.readFileSync("vitest.storage.config.ts", "utf8"), /import base from "\.\/vitest\.config"[\s\S]*test: \{\s*\.\.\.base\.test,/);
});

test("the jsdom and CM6 workers get a V8 old-space cap through the pool, not NODE_OPTIONS", () => {
  const flag = [`--max-old-space-size=${VITEST_WORKER_HEAP_MB}`];
  assert.deepEqual(vitestWorkerHeap({}), { poolOptions: { forks: { execArgv: flag } } });
  assert.deepEqual(vitestWorkerHeap({ TINE_VITEST_HEAP_MB: "2048" }), { poolOptions: { forks: { execArgv: ["--max-old-space-size=2048"] } } });
  assert.deepEqual(vitestWorkerHeap({ TINE_VITEST_HEAP_MB: "0" }), {});
  for (const bad of ["-1", "1.5", "big"]) assert.throws(() => vitestWorkerHeap({ TINE_VITEST_HEAP_MB: bad }));
  assert.match(fs.readFileSync("vitest.render.config.ts", "utf8"), /\.\.\.vitestWorkerHeap\(\)/);
  assert.doesNotMatch(fs.readFileSync("vitest.config.ts", "utf8"), /vitestWorkerHeap/);
});
