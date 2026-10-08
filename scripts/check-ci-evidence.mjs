#!/usr/bin/env node

// Fail closed unless GitHub Actions records a successful manually dispatched
// full CI run for the exact release-candidate commit. Focused and PR runs use
// the same workflow but cannot satisfy this gate because their full jobs are
// absent or skipped.

import { collectGithubPages, selectExactCiEvidence } from "./ci-evidence-lib.mjs";

function option(name) {
  const index = process.argv.indexOf(name);
  return index >= 0 ? process.argv[index + 1] : undefined;
}

const sha = (option("--sha") ?? process.env.CI_EVIDENCE_SHA ?? process.env.GITHUB_SHA)?.trim().toLowerCase();
const repository = option("--repo") ?? process.env.CI_EVIDENCE_REPOSITORY ?? process.env.GITHUB_REPOSITORY;
const workflow = option("--workflow") ?? "ci.yml";
const token = process.env.GH_TOKEN ?? process.env.GITHUB_TOKEN;
const apiBase = process.env.GITHUB_API_URL ?? "https://api.github.com";

if (!sha) throw new Error("Missing candidate SHA; pass --sha or set CI_EVIDENCE_SHA/GITHUB_SHA.");
if (!/^[0-9a-f]{40}$/.test(sha)) throw new Error(`Candidate SHA must be a full 40-character commit: ${sha}`);
if (!repository || !/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repository)) {
  throw new Error("Missing or invalid repository; pass --repo owner/name or set GITHUB_REPOSITORY.");
}
if (!token) throw new Error("Missing GitHub token; set GH_TOKEN or GITHUB_TOKEN with Actions read permission.");

async function github(path) {
  const response = await fetch(`${apiBase}${path}`, {
    headers: {
      Accept: "application/vnd.github+json",
      Authorization: `Bearer ${token}`,
      "X-GitHub-Api-Version": "2022-11-28",
    },
  });
  if (!response.ok) throw new Error(`GitHub API ${response.status} for ${path}: ${await response.text()}`);
  return response.json();
}

const params = new URLSearchParams({
  event: "workflow_dispatch",
  head_sha: sha,
  status: "completed",
  per_page: "100",
});
const encodedWorkflow = encodeURIComponent(workflow);
const runsPath = `/repos/${repository}/actions/workflows/${encodedWorkflow}/runs?${params}`;
const runs = await collectGithubPages((page) => github(`${runsPath}&page=${page}`), "workflow_runs");
const candidates = [];

for (const run of runs) {
  const jobsPath = `/repos/${repository}/actions/runs/${run.id}/jobs?filter=latest&per_page=100`;
  const jobs = await collectGithubPages((page) => github(`${jobsPath}&page=${page}`), "jobs");
  candidates.push({ run, jobs });
}

const evidence = selectExactCiEvidence(sha, candidates);
console.log(
  `Exact-SHA full CI evidence OK: ${sha} via run ${evidence.run.id}`
    + `${evidence.run.html_url ? ` (${evidence.run.html_url})` : ""}.`
);
