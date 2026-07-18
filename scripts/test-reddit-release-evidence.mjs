#!/usr/bin/env node

import assert from "node:assert/strict";
import {
  buildRedditReleaseEvidence,
  createRedditJsonClient,
  fetchAuthorSubmissions,
  fetchThreadSnapshot,
  projectThreadResponse,
  redditAuthorFeedUrl,
  redditThreadSourceUrl,
  validateRedditReleaseEvidence,
} from "./reddit-release-evidence-lib.mjs";

const manifest = {
  author: "al-Quaknaa",
  subreddit: "TineOutline",
  sources: [{
    version: "0.6.0",
    url: "https://www.reddit.com/r/TineOutline/comments/abc123/release_post/",
    blogFile: "v0.6.0.html",
  }],
};

function response(value, { status = 200, headers = {} } = {}) {
  return new Response(typeof value === "string" ? value : JSON.stringify(value), {
    status,
    headers: { "content-type": "application/json", ...headers },
  });
}

function listing(children, after = null) {
  return { kind: "Listing", data: { after, children } };
}

function submission(overrides = {}) {
  return {
    kind: "t3",
    data: {
      id: "abc123",
      name: "t3_abc123",
      title: "Release post",
      author: "al-Quaknaa",
      subreddit: "TineOutline",
      permalink: "/r/TineOutline/comments/abc123/release_post/",
      selftext: "release body",
      selftext_html: "<p>release body</p>",
      created_utc: 100,
      edited: false,
      score: 50,
      all_awardings: [],
      ...overrides,
    },
  };
}

function comment(name, parentId, overrides = {}, replies = "") {
  return {
    kind: "t1",
    data: {
      name,
      parent_id: parentId,
      author: "reader",
      body: `body ${name}`,
      permalink: `/r/TineOutline/comments/abc123/release_post/${name.slice(3)}/`,
      created_utc: 110,
      edited: false,
      score: 7,
      all_awardings: [],
      replies,
      ...overrides,
    },
  };
}

function threadResponse({ rootChildren } = {}) {
  const nested = comment("t1_child", "t1_root", { created_utc: 130 });
  const root = comment("t1_root", "t3_abc123", { edited: 140 }, listing([nested]));
  return [listing([submission()]), listing(rootChildren ?? [root])];
}

// Author discovery follows every page, filters locally, and exposes unregistered posts.
{
  const calls = [];
  const fetchImpl = async (url, init) => {
    calls.push({ url, init });
    if (url.includes("after=t3_first")) {
      return response(listing([
        submission({ id: "new999", name: "t3_new999", title: "New", permalink: "/r/TineOutline/comments/new999/new/", created_utc: 300 }),
        submission({ id: "other", name: "t3_other", author: "someone", permalink: "/r/TineOutline/comments/other/nope/" }),
      ]));
    }
    return response(listing([
      submission(),
      submission({ id: "wrongsub", name: "t3_wrongsub", subreddit: "other", permalink: "/r/other/comments/wrongsub/nope/" }),
    ], "t3_first"));
  };
  const client = createRedditJsonClient({ author: manifest.author, fetchImpl, sleepFn: async () => {} });
  const result = await fetchAuthorSubmissions({ client, manifest });
  assert.equal(result.feedPages, 2);
  assert.equal(result.feedAfter, null);
  assert.deepEqual(result.authorPosts.map((post) => post.id), ["t3_abc123", "t3_new999"]);
  assert.equal(result.authorPosts[0].processedIn, "v0.6.0.html");
  assert.equal(result.authorPosts[1].processedIn, null);
  assert.equal(result.authorPosts[1].url, "https://www.reddit.com/r/TineOutline/comments/new999/new/");
  assert.equal(calls.length, 2);
}

// Complete nested comments have stable semantic hashes independent of volatile fields/order.
{
  const baseline = projectThreadResponse(threadResponse(), "abc123");
  assert.equal(baseline.entryCount, 3);
  assert.equal(baseline.latestUpdate, "1970-01-01T00:02:20.000Z");
  assert.equal(baseline.sha256, "fd9557ee6467ea29e0553a4870cde5b13b802319dfd0c88f1695a25137bf4cea");

  const changedPresentation = threadResponse({
    rootChildren: [
      comment("t1_root", "t3_abc123", { edited: 140, score: 999, all_awardings: [{ id: "award" }] },
        listing([comment("t1_child", "t1_root", { created_utc: 130, score: -5 })])),
    ],
  });
  changedPresentation[0].data.children[0].data.score = -100;
  assert.equal(projectThreadResponse(changedPresentation, "abc123").sha256, baseline.sha256);

  const reordered = threadResponse({
    rootChildren: [
      comment("t1_zed", "t3_abc123", { created_utc: 120 }),
      comment("t1_root", "t3_abc123", { edited: 140 }, listing([comment("t1_child", "t1_root", { created_utc: 130 })])),
    ],
  });
  const reorderedAgain = threadResponse({ rootChildren: [...reordered[1].data.children].reverse() });
  assert.equal(projectThreadResponse(reordered, "abc123").sha256, projectThreadResponse(reorderedAgain, "abc123").sha256);

  const edited = threadResponse();
  edited[1].data.children[0].data.body = "edited body";
  assert.notEqual(projectThreadResponse(edited, "abc123").sha256, baseline.sha256);
  const added = threadResponse();
  added[1].data.children.push(comment("t1_added", "t3_abc123"));
  assert.notEqual(projectThreadResponse(added, "abc123").sha256, baseline.sha256);
}

// Listing pagination must not look complete at Reddit's ceiling or cycle tokens.
{
  const client = createRedditJsonClient({
    author: manifest.author,
    sleepFn: async () => {},
    fetchImpl: async () => response(listing([submission()], "t3_repeat")),
  });
  await assert.rejects(fetchAuthorSubmissions({ client, manifest }), /repeated after token/);
}
{
  const children = Array.from({ length: 1_000 }, (_, index) => submission({
    id: `id${index}`,
    name: `t3_id${index}`,
    permalink: `/r/TineOutline/comments/id${index}/post/`,
  }));
  const client = createRedditJsonClient({
    author: manifest.author,
    sleepFn: async () => {},
    fetchImpl: async () => response(listing(children)),
  });
  await assert.rejects(fetchAuthorSubmissions({ client, manifest }), /1,000-item completeness ceiling/);
}

// Every incompleteness marker and broken parent closure fails closed.
for (const more of [
  { kind: "more", data: { id: "more1", count: 2, children: ["x", "y"] } },
  { kind: "more", data: { id: "_", count: 0, children: [] } },
]) {
  assert.throws(() => projectThreadResponse(threadResponse({ rootChildren: [more] }), "abc123"), /more placeholder/);
}
assert.throws(
  () => projectThreadResponse(threadResponse({ rootChildren: [comment("t1_orphan", "t1_missing")] }), "abc123"),
  /missing parent/,
);

// Retry policy is bounded and honors Retry-After; hard/schema failures do not retry.
{
  const sleeps = [];
  let calls = 0;
  const client = createRedditJsonClient({
    author: manifest.author,
    sleepFn: async (ms) => sleeps.push(ms),
    fetchImpl: async () => {
      calls += 1;
      if (calls === 1) return response({}, { status: 429, headers: { "retry-after": "3" } });
      if (calls === 2) return response({}, { status: 503 });
      return response({ ok: true });
    },
  });
  assert.deepEqual(await client.getJson(redditAuthorFeedUrl(manifest.author)), { ok: true });
  assert.equal(calls, 3);
  assert.deepEqual(sleeps, [3_000, 2_000]);
}
{
  let calls = 0;
  const client = createRedditJsonClient({
    author: manifest.author,
    sleepFn: async () => {},
    fetchImpl: async () => {
      calls += 1;
      if (calls < 3) throw new TypeError("temporary network failure");
      return response({ recovered: true });
    },
  });
  assert.deepEqual(await client.getJson(redditAuthorFeedUrl(manifest.author)), { recovered: true });
  assert.equal(calls, 3);
}
for (const fixture of [
  { result: response({}, { status: 403 }), pattern: /HTTP 403/ },
  { result: response("not json"), pattern: /malformed JSON/ },
]) {
  let calls = 0;
  const client = createRedditJsonClient({
    author: manifest.author,
    sleepFn: async () => {},
    fetchImpl: async () => { calls += 1; return fixture.result; },
  });
  await assert.rejects(client.getJson(redditAuthorFeedUrl(manifest.author)), fixture.pattern);
  assert.equal(calls, 1);
}
{
  let calls = 0;
  const client = createRedditJsonClient({
    author: manifest.author,
    sleepFn: async () => {},
    fetchImpl: async () => { calls += 1; return response({}, { status: 500 }); },
  });
  await assert.rejects(client.getJson(redditAuthorFeedUrl(manifest.author)), /HTTP 500/);
  assert.equal(calls, 4);
}

// A zero remaining quota delays the next serial request.
{
  const sleeps = [];
  let calls = 0;
  const client = createRedditJsonClient({
    author: manifest.author,
    sleepFn: async (ms) => sleeps.push(ms),
    fetchImpl: async () => {
      calls += 1;
      return response({ calls }, calls === 1 ? { headers: { "x-ratelimit-remaining": "0", "x-ratelimit-reset": "2" } } : {});
    },
  });
  await client.getJson(redditAuthorFeedUrl(manifest.author));
  await client.getJson(redditThreadSourceUrl(manifest.sources[0].url));
  assert.deepEqual(sleeps, [2_000]);
}

// A complete end-to-end fixture uses JSON only and supplies strict schema-v2 readiness.
let completeEvidence;
{
  const requests = [];
  const fetchImpl = async (url, init) => {
    requests.push({ url, init });
    if (url.includes("/submitted.json")) return response(listing([submission()]));
    if (url.includes("/comments/abc123.json")) return response(threadResponse());
    throw new Error(`unexpected URL ${url}`);
  };
  completeEvidence = await buildRedditReleaseEvidence({
    manifest,
    version: "0.6.0",
    fetchImpl,
    sleepFn: async () => {},
    nowFn: () => 1_000_000,
  });
  assert.equal(completeEvidence.complete, true);
  assert.equal(completeEvidence.transport, "reddit-json");
  assert.equal(completeEvidence.threadSnapshots.length, 1);
  assert.deepEqual(validateRedditReleaseEvidence(completeEvidence, {
    version: "0.6.0",
    author: manifest.author,
    requiredSources: manifest.sources,
  }), []);
  for (const { url, init } of requests) {
    const parsed = new URL(url);
    assert.equal(parsed.pathname.endsWith(".json"), true);
    assert.equal(url.includes(".rss"), false);
    assert.equal(init.headers.accept, "application/json");
    assert.match(init.headers["user-agent"], /page\.tine\.release-readiness.*al-Quaknaa/);
  }
}

// Feed or thread failure replaces any prior success with explicit incomplete evidence.
for (const failOn of ["submitted", "comments"]) {
  const broken = await buildRedditReleaseEvidence({
    manifest,
    version: "0.6.0",
    sleepFn: async () => {},
    fetchImpl: async (url) => {
      if (url.includes(failOn)) return response({}, { status: 403 });
      if (url.includes("submitted")) return response(listing([submission()]));
      return response(threadResponse());
    },
  });
  assert.equal(broken.complete, false);
  assert.notDeepEqual(broken, completeEvidence);
  if (failOn === "submitted") assert.equal(broken.feedErrors.length, 1);
  else assert.equal(broken.failedThreads.length, 1);
}

// Readiness rejects old transports, partial pagination and stale thread sets.
for (const mutate of [
  (value) => { value.transport = "rss"; },
  (value) => { value.complete = false; },
  (value) => { value.feedAfter = "t3_more"; },
  (value) => { value.feedErrors = [{ error: "failed" }]; },
  (value) => { value.threadSnapshots = []; },
  (value) => { value.threadSnapshots[0].sourceUrl = `${value.threadSnapshots[0].url}.rss`; },
]) {
  const invalid = structuredClone(completeEvidence);
  mutate(invalid);
  assert.notDeepEqual(validateRedditReleaseEvidence(invalid, {
    version: "0.6.0",
    author: manifest.author,
    requiredSources: manifest.sources,
  }), []);
}

console.log("Reddit REST release-evidence tests passed (pagination, completeness, hashing, retry, and schema-v2 gate). ");
