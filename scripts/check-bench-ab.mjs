import { readFileSync } from "node:fs";

function arg(name) {
  const i = process.argv.indexOf(name);
  if (i < 0 || !process.argv[i + 1]) throw new Error(`missing ${name}`);
  return process.argv[i + 1];
}

const policy = JSON.parse(readFileSync(arg("--policy"), "utf8"));
const candidate = JSON.parse(readFileSync(arg("--candidate"), "utf8"));
const immutable = JSON.parse(readFileSync(arg("--immutable"), "utf8"));
const previous = JSON.parse(readFileSync(arg("--previous"), "utf8"));
const failures = [];

if (policy.schemaVersion !== 2) failures.push(`unsupported policy schema ${policy.schemaVersion}`);

const measurements = { candidate, immutable, previous };
for (const [label, measurement] of Object.entries(measurements)) {
  if (measurement.schemaVersion !== 2 || !Array.isArray(measurement.rounds)) {
    failures.push(`${label}: expected a schema-2 multi-round measurement`);
  } else if (measurement.rounds.length < policy.reliability.rounds) {
    failures.push(
      `${label}: only ${measurement.rounds.length} rounds; policy requires ${policy.reliability.rounds}`,
    );
  }
}

const calibrations = Object.values(measurements).flatMap((measurement) =>
  Array.isArray(measurement.rounds)
    ? measurement.rounds.map((round) => round.calib)
    : [measurement.calib],
);
if (calibrations.some((n) => !Number.isFinite(n) || n <= 0)) {
  failures.push("one or more measurements has an invalid calibration");
} else {
  const spread = ((Math.max(...calibrations) / Math.min(...calibrations)) - 1) * 100;
  console.log(`calibration spread: ${spread.toFixed(1)}% (limit ${policy.reliability.maxCalibrationSpreadPct}%)`);
  if (spread > policy.reliability.maxCalibrationSpreadPct) {
    failures.push(`runner load changed too much during A/B measurement (${spread.toFixed(1)}% calibration spread); rerun`);
  }
}

console.log("metric      candidate  immutable  delta/limit     previous  delta/limit");
for (const [name, budget] of Object.entries(policy.metrics)) {
  const value = candidate.metrics?.[name]?.rawMedianOfRoundMins;
  const old = immutable.metrics?.[name]?.rawMedianOfRoundMins;
  const prev = previous.metrics?.[name]?.rawMedianOfRoundMins;
  if (![value, old, prev].every((n) => Number.isFinite(n) && n > 0)) {
    failures.push(`${name}: missing or invalid median-of-round-mins measurement`);
    continue;
  }
  const vsOld = ((value / old) - 1) * 100;
  const vsPrev = ((value / prev) - 1) * 100;
  const candidateRoundMins = candidate.metrics?.[name]?.roundMins;
  const candidateSlowest = Array.isArray(candidateRoundMins) && candidateRoundMins.length > 0
    ? Math.max(...candidateRoundMins)
    : Number.NaN;
  const slowestVsOld = ((candidateSlowest / old) - 1) * 100;
  const slowestVsPrev = ((candidateSlowest / prev) - 1) * 100;
  // Full max/min spread is still useful diagnostic evidence, but it is
  // symmetric: one unusually fast round can exceed the threshold even when
  // every regression comparison is safe. Tolerate high spread only when the
  // candidate median beats both anchors and even its slowest round remains
  // inside both performance budgets. Slow or ambiguous variance still fails.
  const favorableSafeEnvelope = vsOld <= 0 && vsPrev <= 0
    && Number.isFinite(candidateSlowest)
    && slowestVsOld <= budget.maxVsImmutablePct
    && slowestVsPrev <= budget.maxVsPreviousPct;
  for (const [label, measurement] of Object.entries(measurements)) {
    const metric = measurement.metrics?.[name];
    const spread = metric?.roundSpreadPct;
    if (!Number.isFinite(spread)) {
      failures.push(`${label}/${name}: missing round spread`);
    } else {
      console.log(
        `${label}/${name} round spread: ${spread.toFixed(1)}% (limit ${budget.maxRoundSpreadPct}%)`,
      );
      if (spread > budget.maxRoundSpreadPct) {
        const message = `${label}/${name}: ${spread.toFixed(1)}% round spread exceeds ${budget.maxRoundSpreadPct}% reliability limit`;
        if (favorableSafeEnvelope) {
          console.warn(
            `warning: ${message}, but candidate median beats both anchors and its slowest round remains within both budgets`,
          );
        } else {
          failures.push(`${message}; investigate runner/metric variance`);
        }
      }
    }
  }
  console.log(
    `${name.padEnd(11)} ${value.toFixed(1).padStart(9)}  ${old.toFixed(1).padStart(9)}  ` +
    `${`${vsOld.toFixed(1)}%/${budget.maxVsImmutablePct}%`.padStart(11)}  ${prev.toFixed(1).padStart(9)}  ` +
    `${`${vsPrev.toFixed(1)}%/${budget.maxVsPreviousPct}%`.padStart(11)}`
  );
  if (vsOld > budget.maxVsImmutablePct) {
    failures.push(`${name}: ${vsOld.toFixed(1)}% slower than immutable ${policy.immutableBaseline.ref} (limit ${budget.maxVsImmutablePct}%)`);
  }
  if (vsPrev > budget.maxVsPreviousPct) {
    failures.push(`${name}: ${vsPrev.toFixed(1)}% slower than previous release ${policy.previousRelease.ref} (limit ${budget.maxVsPreviousPct}%)`);
  }
}

const misses = candidate.parseStats?.misses;
console.log(`parse misses: ${misses ?? "missing"} (limit ${policy.parseStats.maxMisses})`);
if (!Number.isFinite(misses) || misses > policy.parseStats.maxMisses) {
  failures.push(`parse misses ${misses ?? "missing"} exceed ${policy.parseStats.maxMisses}`);
}

if (failures.length) {
  console.error("\nPerformance A/B gate failed:");
  for (const failure of failures) console.error(`- ${failure}`);
  process.exit(1);
}
console.log("Performance A/B gate passed.");
