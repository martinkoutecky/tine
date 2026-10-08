import assert from "node:assert/strict";
import test from "node:test";
import {
  CHECKUPDATES_PROJECT_ID,
  monitorResult,
  newestBotUpdate,
  TINE_SOURCE_BRANCH,
} from "./fdroid-upstream-monitor.mjs";

function update(iid, status = "success", overrides = {}) {
  return {
    iid,
    title: `bot: Update Tine to ${iid}`,
    source_project_id: CHECKUPDATES_PROJECT_ID,
    source_branch: TINE_SOURCE_BRANCH,
    web_url: `https://gitlab.com/fdroid/fdroiddata/-/merge_requests/${iid}`,
    head_pipeline: status === null ? null : {
      id: iid * 10,
      status,
      web_url: `https://gitlab.com/fdroid/checkupdates-bot-fdroiddata/-/pipelines/${iid * 10}`,
    },
    ...overrides,
  };
}

test("selects only the newest update from F-Droid's checkupdates bot", () => {
  const spoof = update(999, "failed", { source_project_id: 123 });
  assert.equal(newestBotUpdate([update(41), spoof, update(42)])?.iid, 42);
});

test("reports a failed or canceled upstream pipeline", () => {
  assert.deepEqual(monitorResult([update(42, "failed")]), {
    state: "failed",
    mr: {
      iid: 42,
      title: "bot: Update Tine to 42",
      url: "https://gitlab.com/fdroid/fdroiddata/-/merge_requests/42",
    },
    pipeline: {
      id: 420,
      status: "failed",
      url: "https://gitlab.com/fdroid/checkupdates-bot-fdroiddata/-/pipelines/420",
    },
  });
  assert.equal(monitorResult([update(42, "canceled")]).state, "failed");
});

test("keeps successful, running, absent, and not-yet-started updates quiet", () => {
  assert.equal(monitorResult([update(42)]).state, "healthy");
  assert.equal(monitorResult([update(42, "running")]).state, "healthy");
  assert.equal(monitorResult([update(42, null)]).state, "pending");
  assert.equal(monitorResult([]).state, "idle");
});

test("rejects non-GitLab links before exposing them to the issue writer", () => {
  assert.throws(
    () => monitorResult([update(42, "failed", { web_url: "https://example.com/42" })]),
    /unexpected F-Droid URL/,
  );
});
