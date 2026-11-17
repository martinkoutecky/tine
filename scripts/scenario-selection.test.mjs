import assert from "node:assert/strict";
import test from "node:test";
import { selectScenarios } from "./lib/e2e-scenario-selection.mjs";

const ordinary = ["ordinary", "scripts/ordinary.mjs", {}];
const local = ["local-prerequisite", "scripts/local.mjs", {}, "by-name"];
const rows = [ordinary, local];

test("workflow whole-suite execution uses the same catalog as workflow fan-out", () => {
  assert.deepEqual(selectScenarios(rows, undefined, true), [ordinary]);
});

test("local whole-suite execution retains every journey", () => {
  assert.deepEqual(selectScenarios(rows), rows);
});

test("explicit dispatch retains by-name journeys on every host", () => {
  for (const workflow of [false, true]) {
    assert.deepEqual(selectScenarios(rows, "local-prerequisite", workflow), [local]);
    assert.deepEqual(selectScenarios(rows, "ordinary", workflow), [ordinary]);
    assert.deepEqual(selectScenarios(rows, "unknown", workflow), []);
  }
});
