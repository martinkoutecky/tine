#!/usr/bin/env node
// Publish numerical evidence without trial logs, screenshots, DOM text, or
// failure messages that could contain content from the anonymized graph.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const source = path.resolve(process.argv[2] || path.join(root, "test-results/og-bench-baseline"));
const report = JSON.parse(fs.readFileSync(path.join(source, "summary.json"), "utf8"));
const safe = {
  schemaVersion: report.schemaVersion,
  sourceRevisions: report.sourceRevisions,
  binarySha256: report.binarySha256,
  seed: report.seed,
  runs: report.runs,
  probe: report.probe,
  summary: report.summary,
  results: report.results.map((trial) => ({
    kind: trial.kind,
    corpus: trial.corpus,
    index: trial.index,
    loadAvgBefore: trial.loadAvgBefore,
    loadAvgAfter: trial.loadAvgAfter,
    metrics: trial.metrics,
    probeMode: trial.probeMode,
    longTasks: trial.longTasks,
    longTaskMaxMs: trial.longTaskMaxMs,
    budgetViolations: trial.budgetViolations,
    failedJourneys: Object.keys(trial.journeyFailures || {}),
    failure: trial.failure ? { kind: trial.failure.kind, stage: trial.failure.stage } : null,
  })),
};
fs.writeFileSync(path.join(root, "docs/og-bench-baseline.json"), JSON.stringify(safe, null, 2) + "\n");
fs.copyFileSync(path.join(source, "comparison.md"), path.join(root, "docs/og-bench-baseline.md"));
console.log(`published ${safe.results.length} content-free trial records`);
