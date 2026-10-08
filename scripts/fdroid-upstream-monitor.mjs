#!/usr/bin/env node

import { appendFileSync } from "node:fs";
import { pathToFileURL } from "node:url";

export const CHECKUPDATES_PROJECT_ID = 37197382;
export const TINE_SOURCE_BRANCH = "page.tine.app";
export const FDROID_MERGE_REQUESTS_API =
  "https://gitlab.com/api/v4/projects/fdroid%2Ffdroiddata/merge_requests" +
  `?state=opened&source_branch=${TINE_SOURCE_BRANCH}&per_page=20`;
const FDROID_MERGE_REQUEST_API =
  "https://gitlab.com/api/v4/projects/fdroid%2Ffdroiddata/merge_requests";

const FAILURE_STATUSES = new Set(["failed", "canceled"]);

export function newestBotUpdate(mergeRequests) {
  return mergeRequests
    .filter(
      (mr) =>
        mr?.source_project_id === CHECKUPDATES_PROJECT_ID &&
        mr?.source_branch === TINE_SOURCE_BRANCH,
    )
    .sort((a, b) => Number(b.iid) - Number(a.iid))[0] ?? null;
}

export function monitorResult(mergeRequests) {
  const mr = newestBotUpdate(mergeRequests);
  if (!mr) return { state: "idle" };

  const pipeline = mr.head_pipeline;
  if (!pipeline) {
    return { state: "pending", mr: pickMergeRequest(mr), pipeline: null };
  }

  return {
    state: FAILURE_STATUSES.has(pipeline.status) ? "failed" : "healthy",
    mr: pickMergeRequest(mr),
    pipeline: {
      id: Number(pipeline.id),
      status: String(pipeline.status),
      url: checkedGitLabUrl(pipeline.web_url),
    },
  };
}

function pickMergeRequest(mr) {
  return {
    iid: Number(mr.iid),
    title: String(mr.title),
    url: checkedGitLabUrl(mr.web_url),
  };
}

function checkedGitLabUrl(value) {
  const url = new URL(String(value));
  if (url.protocol !== "https:" || url.hostname !== "gitlab.com") {
    throw new Error(`unexpected F-Droid URL: ${url.href}`);
  }
  return url.href;
}

function writeGithubOutputs(result) {
  const path = process.env.GITHUB_OUTPUT;
  if (!path) return;
  const values = {
    state: result.state,
    mr_iid: result.mr?.iid ?? "",
    mr_title: result.mr?.title ?? "",
    mr_url: result.mr?.url ?? "",
    pipeline_id: result.pipeline?.id ?? "",
    pipeline_status: result.pipeline?.status ?? "",
    pipeline_url: result.pipeline?.url ?? "",
  };
  for (const [key, value] of Object.entries(values)) {
    const text = String(value);
    if (text.includes("\n") || text.includes("\r")) {
      throw new Error(`unsafe multiline GitHub output: ${key}`);
    }
    appendFileSync(path, `${key}=${text}\n`);
  }
}

async function main() {
  const candidates = await fetchJson(FDROID_MERGE_REQUESTS_API);
  if (!Array.isArray(candidates)) {
    throw new Error("F-Droid GitLab API returned a non-array merge-request list");
  }
  const newest = newestBotUpdate(candidates);
  const payload = newest
    ? [await fetchJson(`${FDROID_MERGE_REQUEST_API}/${newest.iid}`)]
    : [];
  const result = monitorResult(payload);
  writeGithubOutputs(result);
  console.log(JSON.stringify(result));
}

async function fetchJson(url) {
  const response = await fetch(url, {
    headers: { Accept: "application/json", "User-Agent": "tine-fdroid-monitor" },
  });
  if (!response.ok) {
    throw new Error(`F-Droid GitLab API returned HTTP ${response.status}`);
  }
  return response.json();
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((error) => {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  });
}
