import crypto from "node:crypto";

const REDDIT_ORIGIN = "https://www.reddit.com";
const RETRYABLE_STATUS = new Set([429, 500, 502, 503, 504]);

export function canonicalRedditThreadUrl(permalink) {
  const url = new URL(permalink, REDDIT_ORIGIN);
  if (url.origin !== REDDIT_ORIGIN || !/^\/r\/[^/]+\/comments\/[^/]+\//.test(url.pathname)) {
    throw new Error(`invalid Reddit thread URL: ${permalink}`);
  }
  url.search = "";
  url.hash = "";
  url.pathname = `${url.pathname.replace(/\/+$/, "")}/`;
  return url.toString();
}

export function redditThreadSourceUrl(threadUrl) {
  const match = canonicalRedditThreadUrl(threadUrl).match(/\/comments\/([A-Za-z0-9]+)\//);
  if (!match) throw new Error(`cannot extract Reddit submission id: ${threadUrl}`);
  return `${REDDIT_ORIGIN}/comments/${match[1]}.json?raw_json=1&limit=500&depth=10&sort=new`;
}

export function redditAuthorFeedUrl(author, after = null) {
  if (!/^[A-Za-z0-9_-]+$/.test(author)) throw new Error(`invalid Reddit author: ${author}`);
  const url = new URL(`${REDDIT_ORIGIN}/user/${author}/submitted.json`);
  url.searchParams.set("raw_json", "1");
  url.searchParams.set("limit", "100");
  if (after) url.searchParams.set("after", after);
  return url.toString();
}

function assertJsonUrl(url) {
  const parsed = new URL(url);
  if (parsed.origin !== REDDIT_ORIGIN || !parsed.pathname.endsWith(".json")) {
    throw new Error(`release evidence request is not a Reddit JSON URL: ${url}`);
  }
}

function headerNumber(headers, name) {
  const value = Number(headers.get(name));
  return Number.isFinite(value) && value >= 0 ? value : null;
}

function retryAfterMs(headers, nowMs) {
  const value = headers.get("retry-after");
  if (!value) return null;
  const seconds = Number(value);
  if (Number.isFinite(seconds) && seconds >= 0) return seconds * 1_000;
  const date = Date.parse(value);
  return Number.isFinite(date) ? Math.max(0, date - nowMs) : null;
}

function boundedWait(ms) {
  return Math.min(60_000, Math.max(0, ms));
}

export function createRedditJsonClient({
  author,
  fetchImpl = globalThis.fetch,
  sleepFn = (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
  nowFn = () => Date.now(),
  timeoutMs = 30_000,
} = {}) {
  if (!/^[A-Za-z0-9_-]+$/.test(author ?? "")) throw new Error("invalid Reddit manifest author");
  if (typeof fetchImpl !== "function") throw new Error("fetch implementation is unavailable");
  const userAgent = `linux:page.tine.release-readiness:v1.0.0 (by /u/${author})`;
  const cache = new Map();
  let pendingRateWaitMs = 0;

  async function request(url) {
    assertJsonUrl(url);
    if (pendingRateWaitMs > 0) {
      const wait = pendingRateWaitMs;
      pendingRateWaitMs = 0;
      await sleepFn(wait);
    }

    let lastError;
    let retryWaitTotal = 0;
    for (let attempt = 0; attempt < 4; attempt += 1) {
      const controller = new AbortController();
      const timer = setTimeout(() => controller.abort(new Error(`timeout after ${timeoutMs}ms`)), timeoutMs);
      let response;
      try {
        response = await fetchImpl(url, {
          headers: { "user-agent": userAgent, accept: "application/json" },
          signal: controller.signal,
        });
      } catch (error) {
        lastError = error instanceof Error ? error : new Error(String(error));
      } finally {
        clearTimeout(timer);
      }

      if (response) {
        const remaining = headerNumber(response.headers, "x-ratelimit-remaining");
        const resetSeconds = headerNumber(response.headers, "x-ratelimit-reset");
        if (remaining === 0 && resetSeconds !== null) pendingRateWaitMs = boundedWait(resetSeconds * 1_000);

        if (response.ok) {
          try {
            return await response.json();
          } catch (error) {
            throw new Error(`${url}: malformed JSON: ${error instanceof Error ? error.message : error}`);
          }
        }
        lastError = new Error(`${url}: HTTP ${response.status} ${response.statusText}`.trim());
        if (!RETRYABLE_STATUS.has(response.status)) throw lastError;
      }

      if (attempt === 3) break;
      const headerDelay = response ? retryAfterMs(response.headers, nowFn()) : null;
      // A retry is itself the next Reddit request. Honor an exhausted-quota
      // reset before that retry, even when exponential backoff is shorter.
      const rateDelay = pendingRateWaitMs;
      const delay = boundedWait(Math.max(headerDelay ?? 2 ** attempt * 1_000, rateDelay));
      if (retryWaitTotal + delay > 120_000) break;
      if (rateDelay > 0) pendingRateWaitMs = 0;
      retryWaitTotal += delay;
      await sleepFn(delay);
    }
    throw new Error(`${url}: ${lastError?.message ?? "request failed"}`);
  }

  return {
    getJson(url) {
      if (!cache.has(url)) cache.set(url, request(url));
      return cache.get(url);
    },
  };
}

function requireListing(value, label) {
  if (value?.kind !== "Listing" || !value.data || !Array.isArray(value.data.children)) {
    throw new Error(`${label} is not a Reddit listing`);
  }
  return value.data;
}

function timestamp(value, label) {
  const numeric = Number(value);
  if (!Number.isFinite(numeric) || numeric < 0) throw new Error(`${label} has an invalid timestamp`);
  return numeric;
}

function editedOrCreated(data, label) {
  return typeof data.edited === "number" ? timestamp(data.edited, `${label}.edited`) : timestamp(data.created_utc, `${label}.created_utc`);
}

function isoTimestamp(seconds) {
  return new Date(seconds * 1_000).toISOString();
}

export async function fetchAuthorSubmissions({ client, manifest }) {
  const author = manifest.author;
  const subreddit = manifest.subreddit;
  if (!/^[A-Za-z0-9_-]+$/.test(author ?? "") || !/^[A-Za-z0-9_]+$/.test(subreddit ?? "")) {
    throw new Error("invalid Reddit source manifest");
  }
  const registered = new Map((manifest.sources ?? []).map((source) => [canonicalRedditThreadUrl(source.url), source]));
  const raw = [];
  const seenAfter = new Set();
  let after = null;
  let pages = 0;
  let feedUrl = redditAuthorFeedUrl(author);

  do {
    if (pages >= 10) throw new Error(`author submission pagination exceeded 10 pages with after=${after}`);
    const url = redditAuthorFeedUrl(author, after);
    if (pages === 0) feedUrl = url;
    const listing = requireListing(await client.getJson(url), "author submissions");
    if (listing.children.some((child) => child?.kind !== "t3" || !child.data)) {
      throw new Error("author submissions contained a non-submission child");
    }
    pages += 1;
    raw.push(...listing.children);
    const next = listing.after;
    if (next !== null && typeof next !== "string") throw new Error("author submissions returned an invalid after token");
    if (next && seenAfter.has(next)) throw new Error(`author submissions repeated after token ${next}`);
    if (next) seenAfter.add(next);
    after = next;
  } while (after !== null);

  if (raw.length >= 1_000) throw new Error("author submission listing reached the 1,000-item completeness ceiling");

  const rawNames = raw.map((child) => child.data.name ?? child.data.id);
  if (new Set(rawNames).size !== rawNames.length) throw new Error("author submission pagination returned a duplicate submission");

  const authorPosts = raw
    .filter((child) => child?.kind === "t3")
    .map((child) => child.data)
    .filter((data) => data && String(data.author).toLowerCase() === author.toLowerCase()
      && String(data.subreddit).toLowerCase() === subreddit.toLowerCase())
    .map((data) => {
      const url = canonicalRedditThreadUrl(data.permalink);
      return {
        id: typeof data.name === "string" && data.name ? data.name : data.id,
        title: String(data.title ?? ""),
        author: String(data.author),
        url,
        updated: isoTimestamp(editedOrCreated(data, data.name ?? data.id ?? "submission")),
        contentHtml: typeof data.selftext_html === "string" ? data.selftext_html : "",
        processedIn: registered.get(url)?.blogFile ?? null,
      };
    });

  const feedUpdated = authorPosts.length
    ? authorPosts.map((post) => post.updated).sort().at(-1)
    : null;
  return { feedUrl, feedPages: pages, feedAfter: after, feedUpdated, authorPosts };
}

function submissionProjection(data, expectedName) {
  if (data.name !== expectedName) throw new Error(`thread returned ${data.name ?? "no submission"}, expected ${expectedName}`);
  return {
    name: data.name,
    author: data.author ?? null,
    title: data.title ?? "",
    selftext: data.selftext ?? "",
    permalink: canonicalRedditThreadUrl(data.permalink),
    created_utc: timestamp(data.created_utc, `${data.name}.created_utc`),
    edited: typeof data.edited === "number" ? timestamp(data.edited, `${data.name}.edited`) : false,
  };
}

function commentProjection(data) {
  if (typeof data.name !== "string" || !data.name.startsWith("t1_")) throw new Error("comment has no stable t1 fullname");
  if (typeof data.parent_id !== "string") throw new Error(`${data.name} has no parent_id`);
  return {
    name: data.name,
    parent_id: data.parent_id,
    author: data.author ?? null,
    body: data.body ?? "",
    permalink: canonicalRedditThreadUrl(data.permalink),
    created_utc: timestamp(data.created_utc, `${data.name}.created_utc`),
    edited: typeof data.edited === "number" ? timestamp(data.edited, `${data.name}.edited`) : false,
  };
}

function collectComments(listing, comments, seen) {
  const data = requireListing(listing, "comment tree");
  for (const child of data.children) {
    if (child?.kind === "more") throw new Error("comment tree contains an incomplete more placeholder");
    if (child?.kind !== "t1" || !child.data) throw new Error(`comment tree contains unexpected kind ${child?.kind ?? "missing"}`);
    const comment = commentProjection(child.data);
    if (seen.has(comment.name)) throw new Error(`duplicate comment ${comment.name}`);
    seen.add(comment.name);
    comments.push(comment);
    const replies = child.data.replies;
    if (replies && replies !== "") collectComments(replies, comments, seen);
  }
}

export function projectThreadResponse(response, requestedId) {
  if (!Array.isArray(response) || response.length !== 2) throw new Error("thread response must contain exactly two listings");
  const submissionListing = requireListing(response[0], "thread submission");
  if (submissionListing.children.length !== 1 || submissionListing.children[0]?.kind !== "t3") {
    throw new Error("thread response must contain exactly one submission");
  }
  const expectedName = `t3_${requestedId}`;
  const submission = submissionProjection(submissionListing.children[0].data, expectedName);
  const comments = [];
  collectComments(response[1], comments, new Set());
  const names = new Set(comments.map((comment) => comment.name));
  for (const comment of comments) {
    if (comment.parent_id !== expectedName && !names.has(comment.parent_id)) {
      throw new Error(`${comment.name} references missing parent ${comment.parent_id}`);
    }
  }
  comments.sort((left, right) => left.name.localeCompare(right.name));
  const canonical = { submission, comments };
  const sha256 = crypto.createHash("sha256").update(JSON.stringify(canonical), "utf8").digest("hex");
  const latestSeconds = Math.max(
    editedOrCreated(submission, submission.name),
    ...comments.map((comment) => editedOrCreated(comment, comment.name)),
  );
  return { canonical, sha256, entryCount: 1 + comments.length, latestUpdate: isoTimestamp(latestSeconds) };
}

export async function fetchThreadSnapshot({ client, source }) {
  const canonicalUrl = canonicalRedditThreadUrl(source.url);
  const sourceUrl = redditThreadSourceUrl(canonicalUrl);
  const requestedId = canonicalUrl.match(/\/comments\/([A-Za-z0-9]+)\//)[1];
  const projection = projectThreadResponse(await client.getJson(sourceUrl), requestedId);
  return {
    version: source.version,
    url: canonicalUrl,
    sourceUrl,
    blogFile: source.blogFile,
    entryCount: projection.entryCount,
    latestUpdate: projection.latestUpdate,
    sha256: projection.sha256,
  };
}

export async function buildRedditReleaseEvidence({ manifest, version, fetchImpl, sleepFn, nowFn = () => Date.now() }) {
  const client = createRedditJsonClient({ author: manifest.author, fetchImpl, sleepFn, nowFn });
  const feedErrors = [];
  let feed = {
    feedUrl: redditAuthorFeedUrl(manifest.author),
    feedPages: 0,
    feedAfter: null,
    feedUpdated: null,
    authorPosts: [],
  };
  try {
    feed = await fetchAuthorSubmissions({ client, manifest });
  } catch (error) {
    feedErrors.push({ url: feed.feedUrl, error: String(error) });
  }

  const threadSnapshots = [];
  const failedThreads = [];
  for (const source of manifest.sources ?? []) {
    try {
      threadSnapshots.push(await fetchThreadSnapshot({ client, source }));
    } catch (error) {
      failedThreads.push({ url: canonicalRedditThreadUrl(source.url), sourceUrl: redditThreadSourceUrl(source.url), error: String(error) });
    }
  }
  const unprocessed = feed.authorPosts.filter((post) => !post.processedIn).map((post) => post.url);
  const complete = feedErrors.length === 0 && unprocessed.length === 0 && failedThreads.length === 0
    && feed.feedPages > 0 && feed.feedAfter === null && threadSnapshots.length === (manifest.sources ?? []).length;
  return {
    schemaVersion: 2,
    transport: "reddit-json",
    complete,
    version,
    generatedAt: new Date(nowFn()).toISOString(),
    feedUrl: feed.feedUrl,
    feedPages: feed.feedPages,
    feedAfter: feed.feedAfter,
    feedErrors,
    feedUpdated: feed.feedUpdated,
    author: manifest.author,
    authorPosts: feed.authorPosts,
    unprocessed,
    threadSnapshots,
    failedThreads,
  };
}

function isJsonSourceUrl(value) {
  try {
    const url = new URL(value);
    return url.origin === REDDIT_ORIGIN && url.pathname.endsWith(".json") && !value.includes(".rss");
  } catch {
    return false;
  }
}

export function validateRedditReleaseEvidence(evidence, { version, author, requiredSources = [] }) {
  const problems = [];
  if (evidence?.schemaVersion !== 2 || evidence?.version !== version) problems.push("Reddit/blog evidence version or schema mismatch");
  if (evidence?.transport !== "reddit-json") problems.push("Reddit/blog evidence must use reddit-json transport");
  if (evidence?.complete !== true) problems.push("Reddit/blog evidence is incomplete");
  if (evidence?.author !== author) problems.push("Reddit/blog evidence has unexpected author");
  if (!isJsonSourceUrl(evidence?.feedUrl)) problems.push("Reddit/blog author feed is not a JSON source URL");
  if (!Number.isInteger(evidence?.feedPages) || evidence.feedPages < 1) problems.push("Reddit/blog evidence has no complete feed pages");
  if (evidence?.feedAfter !== null) problems.push("Reddit/blog evidence has unfinished feed pagination");
  if (!Array.isArray(evidence?.feedErrors) || evidence.feedErrors.length) problems.push("Reddit/blog evidence has author-feed failures");
  if (!Array.isArray(evidence?.unprocessed) || evidence.unprocessed.length) problems.push("Reddit/blog evidence has unprocessed author posts");
  if (!Array.isArray(evidence?.failedThreads) || evidence.failedThreads.length) problems.push("Reddit/blog evidence has discussion refresh failures");
  if (!Array.isArray(evidence?.threadSnapshots)) problems.push("Reddit/blog evidence lacks discussion snapshots");
  else {
    const snapshots = new Map();
    for (const item of evidence.threadSnapshots) {
      let url;
      try { url = canonicalRedditThreadUrl(item.url); } catch { url = item.url; }
      if (snapshots.has(url)) problems.push(`Reddit/blog evidence duplicates discussion ${url}`);
      snapshots.set(url, item);
      if (!/^[0-9a-f]{64}$/.test(item.sha256 ?? "") || !isJsonSourceUrl(item.sourceUrl)
        || item.sourceUrl !== redditThreadSourceUrl(url)
        || !Number.isInteger(item.entryCount) || item.entryCount < 1
        || !Number.isFinite(Date.parse(item.latestUpdate ?? ""))) {
        problems.push(`Reddit/blog evidence has an invalid discussion snapshot for ${url}`);
      }
    }
    for (const source of requiredSources) {
      const url = canonicalRedditThreadUrl(source.url);
      if (!snapshots.has(url)) problems.push(`Reddit/blog evidence is missing discussion ${url}`);
    }
    if (snapshots.size !== requiredSources.length) problems.push("Reddit/blog evidence discussion set does not match the source manifest");
  }
  return problems;
}
