#!/usr/bin/env node

// Same-runner release A/B orchestrator. Each tree is measured in every round,
// with the order rotated so runner warm-up/load cannot consistently favor one
// version. The aggregate preserves every round and uses the median of round
// minima; check-bench-ab.mjs separately rejects excessive per-metric spread.

import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

function arg(name) {
  const index = process.argv.indexOf(name);
  if (index < 0 || !process.argv[index + 1]) throw new Error(`missing ${name}`);
  return process.argv[index + 1];
}

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const policyPath = path.resolve(arg("--policy"));
const policy = JSON.parse(readFileSync(policyPath, "utf8"));
const outputDir = path.resolve(arg("--output-dir"));
const rounds = policy.reliability?.rounds;
const runsPerRound = policy.reliability?.runsPerRound;
if (!Number.isInteger(rounds) || rounds < 3) throw new Error("policy reliability.rounds must be at least 3");
if (!Number.isInteger(runsPerRound) || runsPerRound < 2) throw new Error("policy reliability.runsPerRound must be at least 2");

const versions = [
  { label: "immutable", dir: path.resolve(arg("--immutable-dir")) },
  { label: "previous", dir: path.resolve(arg("--previous-dir")) },
  { label: "candidate", dir: path.resolve(arg("--candidate-dir")) },
];
mkdirSync(path.join(outputDir, "rounds"), { recursive: true });

const measurements = new Map(versions.map(({ label }) => [label, []]));
const orderLog = [];
for (let round = 0; round < rounds; round++) {
  // Rotate the three-version order each round. With three rounds every version
  // occupies the first, middle, and last position exactly once.
  const order = versions.map((_, index) => versions[(index + round) % versions.length]);
  orderLog.push(order.map(({ label }) => label));
  for (let position = 0; position < order.length; position++) {
    const version = order[position];
    const destination = path.join(outputDir, "rounds", `${version.label}-r${round + 1}.json`);
    const port = 5260 + round * versions.length + position;
    console.log(`\nround ${round + 1}/${rounds}: ${version.label} (position ${position + 1})`);
    const run = spawnSync(
      process.execPath,
      [
        path.join(root, "scripts", "bench.mjs"),
        "--update",
        "--app-dir", version.dir,
        "--runs", String(runsPerRound),
        "--port", String(port),
        "--output", destination,
      ],
      { cwd: root, encoding: "utf8", stdio: "inherit" },
    );
    if (run.status !== 0) process.exit(run.status ?? 1);
    measurements.get(version.label).push(JSON.parse(readFileSync(destination, "utf8")));
  }
}

const median = (values) => {
  const sorted = [...values].sort((a, b) => a - b);
  const middle = Math.floor(sorted.length / 2);
  return sorted.length % 2 ? sorted[middle] : (sorted[middle - 1] + sorted[middle]) / 2;
};
const round1 = (value) => Math.round(value * 10) / 10;
const spreadPct = (values) => (Math.max(...values) / Math.min(...values) - 1) * 100;

for (const { label } of versions) {
  const samples = measurements.get(label);
  const metricNames = Object.keys(samples[0].metrics);
  const aggregate = {
    schemaVersion: 2,
    label,
    rounds: samples,
    calib: round1(median(samples.map((sample) => sample.calib))),
    metrics: {},
    // A parse virtualization regression in any candidate round must remain
    // visible; taking a median could hide a one-round full-parse failure.
    parseStats: {
      calls: Math.max(...samples.map((sample) => sample.parseStats?.calls ?? 0)),
      hits: Math.max(...samples.map((sample) => sample.parseStats?.hits ?? 0)),
      misses: Math.max(...samples.map((sample) => sample.parseStats?.misses ?? 0)),
    },
  };
  for (const name of metricNames) {
    const roundMins = samples.map((sample) => sample.metrics[name].rawMin);
    aggregate.metrics[name] = {
      rawMedianOfRoundMins: round1(median(roundMins)),
      roundMins,
      roundSpreadPct: Number(spreadPct(roundMins).toFixed(1)),
    };
  }
  writeFileSync(path.join(outputDir, `${label}.json`), JSON.stringify(aggregate, null, 2) + "\n");
}

writeFileSync(
  path.join(outputDir, "manifest.json"),
  JSON.stringify({ schemaVersion: 1, rounds, runsPerRound, order: orderLog }, null, 2) + "\n",
);
console.log(`\ninterleaved A/B measurements written to ${outputDir}`);
