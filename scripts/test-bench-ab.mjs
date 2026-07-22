#!/usr/bin/env node

import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import os from "node:os";
import path from "node:path";

const root = process.cwd();
const policy = JSON.parse(readFileSync(path.join(root, "scripts", "bench-policy.json"), "utf8"));
const temporary = mkdtempSync(path.join(os.tmpdir(), "tine-bench-ab-test-"));

function measurement(label, bigLoad, scrollBig) {
  const round = (load, scroll, index) => ({
    calib: 100 + index * 0.2,
    metrics: { bigLoad: { rawMin: load }, scrollBig: { rawMin: scroll } },
    parseStats: { calls: 12, hits: 0, misses: 12 },
  });
  const median = (values) => [...values].sort((a, b) => a - b)[1];
  const spread = (values) => Number(((Math.max(...values) / Math.min(...values) - 1) * 100).toFixed(1));
  return {
    schemaVersion: 2,
    label,
    rounds: bigLoad.map((load, index) => round(load, scrollBig[index], index)),
    calib: 100.2,
    metrics: {
      bigLoad: {
        rawMedianOfRoundMins: median(bigLoad),
        roundMins: bigLoad,
        roundSpreadPct: spread(bigLoad),
      },
      scrollBig: {
        rawMedianOfRoundMins: median(scrollBig),
        roundMins: scrollBig,
        roundSpreadPct: spread(scrollBig),
      },
    },
    parseStats: { calls: 12, hits: 0, misses: 12 },
  };
}

function check(candidate, immutable, previous) {
  const files = { candidate, immutable, previous };
  const args = [path.join(root, "scripts", "check-bench-ab.mjs"), "--policy", path.join(root, "scripts", "bench-policy.json")];
  for (const [label, value] of Object.entries(files)) {
    const file = path.join(temporary, `${label}.json`);
    writeFileSync(file, JSON.stringify(value));
    args.push(`--${label}`, file);
  }
  return spawnSync(process.execPath, args, { cwd: root, encoding: "utf8" });
}

try {
  const stableImmutable = measurement("immutable", [100, 101, 99], [100, 101, 99]);
  const stablePrevious = measurement("previous", [105, 106, 104], [105, 106, 104]);
  const stableCandidate = measurement("candidate", [110, 111, 109], [110, 111, 109]);
  const stable = check(stableCandidate, stableImmutable, stablePrevious);
  assert.equal(stable.status, 0, stable.stderr || stable.stdout);

  // Exact reproduction of the former false-green shape: candidate scroll is
  // stable while the immutable anchor contains the 34.6 -> 45.9 ms swing that
  // flipped run 29395811537. Its median comparison is harmless, but reliability
  // must fail independently.
  const unstableImmutable = measurement("immutable", [100, 101, 99], [75.4, 100, 100.2]);
  const unstable = check(stableCandidate, unstableImmutable, stablePrevious);
  assert.notEqual(unstable.status, 0);
  assert.match(`${unstable.stdout}\n${unstable.stderr}`, /immutable\/scrollBig: .*round spread exceeds/);

  // A fast outlier must not block a clearly favorable candidate. Full spread
  // is still reported, and the candidate's slowest round must remain inside
  // both regression budgets.
  const favorableVariableCandidate = measurement(
    "candidate",
    [90, 92, 91],
    [80, 95, 80],
  );
  const favorableVariable = check(favorableVariableCandidate, stableImmutable, stablePrevious);
  assert.equal(favorableVariable.status, 0, favorableVariable.stderr || favorableVariable.stdout);
  assert.match(
    `${favorableVariable.stdout}\n${favorableVariable.stderr}`,
    /warning: candidate\/scrollBig: .*candidate median beats both anchors/,
  );

  // A favorable median cannot conceal an unsafe slow tail.
  const unsafeTailCandidate = measurement("candidate", [90, 92, 91], [50, 50, 200]);
  const unsafeTail = check(unsafeTailCandidate, stableImmutable, stablePrevious);
  assert.notEqual(unsafeTail.status, 0);
  assert.match(`${unsafeTail.stdout}\n${unsafeTail.stderr}`, /candidate\/scrollBig: .*round spread exceeds/);

  const regressedCandidate = measurement("candidate", [140, 141, 139], [140, 141, 139]);
  const regressed = check(regressedCandidate, stableImmutable, stablePrevious);
  assert.notEqual(regressed.status, 0);
  assert.match(`${regressed.stdout}\n${regressed.stderr}`, /slower than immutable/);

  assert.equal(policy.reliability.rounds, 3);
  console.log("Performance A/B multi-round reliability fixtures passed.");
} finally {
  rmSync(temporary, { recursive: true, force: true });
}
