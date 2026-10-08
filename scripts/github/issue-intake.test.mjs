import assert from "node:assert/strict";
import test from "node:test";
import { classificationLabel, labelOpenedIssue, reopenFollowup, shouldReopen } from "./issue-intake.mjs";
import { readFileSync } from "node:fs";

const reporter = { id: 123, login: "reporter", type: "User" };
const maintainer = { id: 456, login: "maintainer", type: "User" };
const at = (seconds) => new Date(Date.UTC(2026, 7, 29, 12, 0, seconds)).toISOString();
function fixture(closeSeconds = 0, commentSeconds = 0, actor = reporter) {
  const issue = { number: 436, state: "closed", user: reporter, labels: [] };
  const event = {
    action: "created", issue: { ...issue, labels: [] },
    comment: { id: 999, user: reporter, author_association: "NONE",
      created_at: at(commentSeconds), body: "I found my mistake, I'm closing the issue!" },
  };
  return { event, issue, timeline: [
    { id: 998, event: "closed", actor, created_at: at(closeSeconds) },
  ] };
}

test("same actor can reopen after 30 seconds, including much later", () => {
  for (const seconds of [30, 31, 3600]) {
    const { event, issue, timeline } = fixture(0, seconds);
    assert.equal(shouldReopen(event, issue, timeline), seconds > 30);
  }
  const { event, issue, timeline } = fixture(30, 0);
  assert.equal(shouldReopen(event, issue, timeline), false);
});

test("another actor's close permits an immediate or later follow-up", () => {
  for (const seconds of [0, 2, 3600]) {
    const { event, issue, timeline } = fixture(0, seconds, maintainer);
    assert.equal(shouldReopen(event, issue, timeline), true);
  }
});

test("delayed comments from before the latest close cannot undo it", () => {
  const { event, issue, timeline } = fixture(100, 0, maintainer);
  assert.equal(shouldReopen(event, issue, timeline), false);
});

test("uses the latest lifecycle event across the full timeline", () => {
  const { event, issue, timeline } = fixture(0, 3600, maintainer);
  const newer = { event: "closed", id: 1001, actor: reporter, created_at: at(3600) };
  const reopen = { event: "reopened", id: 1000, actor: maintainer, created_at: at(1) };
  assert.equal(shouldReopen(event, issue, [newer, ...timeline, reopen]), false);
  assert.equal(shouldReopen(event, issue, [...timeline, reopen]), false);
  // Equal-second lifecycle transitions are ordered by event ID.
  assert.equal(shouldReopen(event, issue, [...timeline, { ...reopen, created_at: at(0) }]), false);
});

test("keeps bots, collaborators, PRs, non-created events and open issues quiet", () => {
  for (const association of ["OWNER", "MEMBER", "COLLABORATOR"]) {
    const { event, issue, timeline } = fixture(0, 100, maintainer);
    event.comment.author_association = association;
    assert.equal(shouldReopen(event, issue, timeline), false);
  }
  for (const change of [
    (f) => { f.event.comment.user = { ...reporter, type: "Bot" }; },
    (f) => { f.issue.pull_request = {}; },
    (f) => { f.issue.state = "open"; },
    (f) => { f.event.action = "edited"; },
  ]) {
    const f = fixture(0, 100, maintainer);
    change(f);
    assert.equal(shouldReopen(f.event, f.issue, f.timeline), false);
  }
});

test("missing or malformed close evidence causes no reopen", () => {
  const { event, issue, timeline } = fixture(0, 100, maintainer);
  assert.equal(shouldReopen(event, issue, []), false);
  assert.equal(shouldReopen(event, issue, [{ ...timeline[0], actor: null }]), false);
  assert.equal(shouldReopen(event, issue, [{ ...timeline[0], created_at: "invalid" }]), false);
  event.comment.created_at = "invalid";
  assert.equal(shouldReopen(event, issue, timeline), false);
});

function client(f, { failUpdate = false } = {}) {
  const writes = [];
  const reads = [];
  const github = {
    rest: { issues: {
      get: async (params) => { reads.push(params); return { data: f.issue }; },
      listEventsForTimeline: () => {},
      addLabels: async ({ labels }) => {
        writes.push({ labels });
        for (const name of labels) if (!f.issue.labels.some((label) => label.name === name))
          f.issue.labels.push({ name });
      },
      update: async ({ state }) => {
        if (failUpdate) { failUpdate = false; throw new Error("temporary API failure"); }
        writes.push({ state }); f.issue.state = state;
      },
    } },
    paginate: async (method, params) => {
      assert.equal(method, github.rest.issues.listEventsForTimeline);
      assert.equal(params.per_page, 100);
      return f.timeline;
    },
  };
  return { github, context: { repo: { owner: "owner", repo: "repo" }, payload: f.event }, writes, reads };
}

test("fresh issue state handles an open webhook snapshot followed by a close", async () => {
  const f = fixture(2, 0);
  f.event.issue = { ...f.issue, state: "open" };
  const api = client(f);
  await reopenFollowup(api);
  assert.equal(api.reads.length, 1);
  assert.deepEqual(api.writes, []);
  assert.equal(f.issue.state, "closed");
});

test("repeated eligible delivery reopens and labels only once, preserving labels", async () => {
  const f = fixture(0, 100, maintainer);
  f.issue.labels = [{ name: "bug" }];
  const api = client(f);
  await reopenFollowup(api);
  await reopenFollowup(api);
  assert.deepEqual(api.writes, [{ labels: ["needs-triage"] }, { state: "open" }]);
  assert.deepEqual(f.issue.labels, [{ name: "bug" }, { name: "needs-triage" }]);
  f.issue.state = "closed";
  f.timeline.push({ event: "closed", id: 1002, actor: maintainer, created_at: at(200) });
  await reopenFollowup(api);
  assert.equal(api.writes.length, 2);
});

test("retry completes a failed reopen without adding the triage label twice", async () => {
  const api = client(fixture(0, 100, maintainer), { failUpdate: true });
  await assert.rejects(reopenFollowup(api), /temporary API failure/);
  await reopenFollowup(api);
  assert.deepEqual(api.writes, [{ labels: ["needs-triage"] }, { state: "open" }]);
});

test("latest close on a later timeline page governs reopening", async () => {
  const f = fixture(0, 1000, maintainer);
  f.timeline.push(...Array.from({ length: 100 }, (_, id) => ({ event: "labeled", id })));
  f.timeline.push({ event: "closed", id: 2000, actor: reporter, created_at: at(1000) });
  const api = client(f);
  await reopenFollowup(api);
  assert.deepEqual(api.writes, []);
});

test("repeated self-close delivery performs no writes", async () => {
  const api = client(fixture(0, 2));
  await reopenFollowup(api);
  await reopenFollowup(api);
  assert.deepEqual(api.writes, []);
});

test("classifies the actual current form headings and matches their declared label", () => {
  for (const [file, label] of [["bug_report.yml", "bug"], ["feature_request.yml", "enhancement"]]) {
    const form = readFileSync(new URL(`../../.github/ISSUE_TEMPLATE/${file}`, import.meta.url), "utf8");
    assert.ok(form.includes(`labels: ["${label}"]`));
    const headings = [...form.matchAll(/^      label: (.+)$/gm)].map((match) => match[1]);
    const body = headings.map((heading) => `### ${heading}\n\nExample answer`).join("\n\n");
    assert.equal(classificationLabel({ title: "CLI issue", body, labels: [] }), label);
  }
});

test("recognizes explicit title prefixes, case-insensitively", () => {
  assert.equal(classificationLabel({ title: "[BuG] failure" }), "bug");
  assert.equal(classificationLabel({ title: " [FEATURE] idea" }), "enhancement");
  assert.equal(classificationLabel({ title: "[buggy] failure" }), null);
});

test("existing explicit classifications win; unrelated labels survive", () => {
  for (const labels of [["bug"], [{ name: "enhancement" }], ["BUG", "enhancement"]]) {
    assert.equal(classificationLabel({ title: "[feature] idea", labels }), null);
  }
  assert.equal(classificationLabel({ title: "[bug] failure", labels: [{ name: "linux" }] }), "bug");
});

test("unrecognized, conflicting, quoted and fenced examples are not classified", () => {
  for (const body of [null, "bug enhancement", "> ### What happened?",
    "### Background\n\n### What happened?", "```md\n### What happened?\n```",
    "~~~~md\n### What happened?\n~~~\n### What happened?\n~~~~"]) {
    assert.equal(classificationLabel({ title: "Question", body }), null);
  }
  assert.equal(classificationLabel({ title: "[bug] failure", body: "### What would you like Tine to do?" }), null);
  assert.equal(classificationLabel({ title: "[bug] PR", pull_request: {} }), null);
});

test("content is data; CRLF bodies and fenced examples before a real heading work", () => {
  assert.equal(classificationLabel({ title: "$(touch /tmp/unsafe)",
    body: "```\r\n### What happened?\r\n```\r\n### What would you like Tine to do?\r\n\r\n${{ secrets.TOKEN }}" }), "enhancement");
});

test("opened issue handler is idempotent and consults live explicit labels", async () => {
  const f = fixture();
  f.event.action = "opened";
  f.issue.title = "[bug] CLI issue";
  f.issue.labels = [{ name: "linux" }];
  const api = client(f);
  await labelOpenedIssue(api);
  await labelOpenedIssue(api);
  assert.deepEqual(api.writes, [{ labels: ["bug"] }]);
  assert.deepEqual(f.issue.labels, [{ name: "linux" }, { name: "bug" }]);
  f.issue.labels = [{ name: "enhancement" }];
  await labelOpenedIssue(api);
  assert.equal(api.writes.length, 1);
});

test("opened handler does nothing for unrecognized issues or other event types", async () => {
  const f = fixture();
  const api = client(f);
  await labelOpenedIssue(api);
  assert.equal(api.reads.length, 0);
  f.event.action = "opened";
  await labelOpenedIssue(api);
  assert.deepEqual(api.writes, []);
});

for (const [name, closeSeconds, commentSeconds] of [
  ["comment then close", 2, 0],
  ["close then comment", 0, 2],
  ["same API timestamp", 0, 0],
]) {
  test(`#436 close-with-comment stays closed: ${name}`, () => {
    const { event, issue, timeline } = fixture(closeSeconds, commentSeconds);
    assert.equal(shouldReopen(event, issue, timeline), false);
  });
}
