import assert from "node:assert/strict";
import fs from "node:fs";
import test from "node:test";
import { vitestMaxWorkers } from "./vitest-workers.mjs";

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
