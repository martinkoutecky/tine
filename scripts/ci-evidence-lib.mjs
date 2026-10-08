export const REQUIRED_FULL_CI_JOBS = Object.freeze([
  "Full CI / Linux tests and release contracts",
  "Full CI / Windows compile and core tests",
  "Full CI / Android core compile",
  "Full CI / performance A/B",
]);

export async function collectGithubPages(loadPage, key, { perPage = 100, maxPages = 1000 } = {}) {
  const collected = [];
  for (let page = 1; page <= maxPages; page += 1) {
    const payload = await loadPage(page);
    const items = payload?.[key];
    if (!Array.isArray(items)) throw new Error(`GitHub page ${page} has no ${key} array.`);
    collected.push(...items);
    if (items.length < perPage) return collected;
  }
  throw new Error(`GitHub pagination for ${key} exceeded ${maxPages} pages.`);
}

function normalizedSha(value) {
  return typeof value === "string" ? value.trim().toLowerCase() : "";
}

export function ciEvidenceProblems(expectedSha, candidate) {
  const problems = [];
  const run = candidate?.run ?? {};
  const jobs = Array.isArray(candidate?.jobs) ? candidate.jobs : [];
  const expected = normalizedSha(expectedSha);

  if (!/^[0-9a-f]{40}$/.test(expected)) {
    problems.push(`expected SHA is not a full 40-character commit: ${expectedSha}`);
  }
  if (run.event !== "workflow_dispatch") {
    problems.push(`run event is ${run.event ?? "missing"}, not workflow_dispatch`);
  }
  if (normalizedSha(run.head_sha) !== expected) {
    problems.push(`run SHA is ${run.head_sha ?? "missing"}, not ${expected}`);
  }
  if (run.status !== "completed") problems.push(`run status is ${run.status ?? "missing"}, not completed`);
  if (run.conclusion !== "success") problems.push(`run conclusion is ${run.conclusion ?? "missing"}, not success`);

  const conclusions = new Map(jobs.map((job) => [job.name, job.conclusion]));
  for (const name of REQUIRED_FULL_CI_JOBS) {
    const conclusion = conclusions.get(name);
    if (conclusion !== "success") problems.push(`${name} concluded ${conclusion ?? "missing"}, not success`);
  }

  return problems;
}

export function selectExactCiEvidence(expectedSha, candidates) {
  const inspected = [];
  for (const candidate of candidates) {
    const problems = ciEvidenceProblems(expectedSha, candidate);
    if (problems.length === 0) return candidate;
    inspected.push({ id: candidate?.run?.id ?? "unknown", problems });
  }

  const detail = inspected.length
    ? inspected.map(({ id, problems }) => `run ${id}: ${problems.join("; ")}`).join("\n")
    : "no completed manual CI runs were returned for this SHA";
  throw new Error(`No successful full CI evidence for exact SHA ${expectedSha}.\n${detail}`);
}
