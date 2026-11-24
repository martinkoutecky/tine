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

test("no release scenario needs a test-faults build that a release binary cannot give it", async () => {
  const { spawnSync } = await import("node:child_process");
  const fs = await import("node:fs");
  const root = new URL("..", import.meta.url).pathname;
  const contracts = JSON.parse(fs.readFileSync(`${root}tests/ui-regressions/e2e-contracts.json`, "utf8")).scenarios;
  for (const suite of ["linux-release", "linux-smoke", "windows-smoke"]) {
    const listed = spawnSync(process.execPath, [`${root}scripts/run-e2e.mjs`, suite, "--scripts"], { cwd: root, encoding: "utf8" });
    assert.equal(listed.status, 0, listed.stderr);
    // The app's own feature (`--features ...,tine-store/test-faults`); a separate test-faults helper
    // (`-p tine-store --features test-faults --example ...`) leaves the release app usable.
    const needFaults = JSON.parse(listed.stdout).filter((script) => /tine-store\/test-faults/.test(contracts[script]?.setup ?? ""));
    assert.deepEqual(needFaults, [], `${suite} lists journeys whose contract setup needs an app built with tine-store/test-faults`);
  }
});
