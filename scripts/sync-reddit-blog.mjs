#!/usr/bin/env node

import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const version = process.argv.find((arg) => arg.startsWith("--version="))?.slice(10);
if (!/^\d+\.\d+\.0$/.test(version ?? "")) {
  console.error("usage: npm run blog:sync -- --version=X.Y.0");
  process.exit(2);
}
const manifest = JSON.parse(fs.readFileSync(path.join(root, "website/blog/reddit-sources.json"), "utf8"));
const output = path.join(root, `docs/releases/v${version}-reddit.json`);
const userAgent = "tine-release-readiness/1.0 (+https://github.com/martinkoutecky/tine)";

function decodeXml(value = "") {
  return value
    .replaceAll("&lt;", "<")
    .replaceAll("&gt;", ">")
    .replaceAll("&quot;", '"')
    .replaceAll("&#39;", "'")
    .replaceAll("&amp;", "&");
}

function field(xml, expression) {
  return decodeXml(xml.match(expression)?.[1]?.trim() ?? "");
}

function entries(xml) {
  return [...xml.matchAll(/<entry>([\s\S]*?)<\/entry>/g)].map((match) => {
    const entry = match[1];
    return {
      id: field(entry, /<id>([\s\S]*?)<\/id>/),
      title: field(entry, /<title[^>]*>([\s\S]*?)<\/title>/),
      author: field(entry, /<author>[\s\S]*?<name>([\s\S]*?)<\/name>/).replace(/^\/u\//, ""),
      url: field(entry, /<link[^>]+href="([^"]+)"/),
      updated: field(entry, /<updated>([^<]+)<\/updated>/),
      contentHtml: field(entry, /<content[^>]*>([\s\S]*?)<\/content>/),
    };
  });
}

async function fetchText(url) {
  let last;
  for (const delay of [0, 2_000, 6_000]) {
    if (delay) await sleep(delay);
    try {
      const response = await fetch(url, { headers: { "user-agent": userAgent, accept: "application/atom+xml" } });
      if (response.ok) return await response.text();
      last = new Error(`${response.status} ${response.statusText}`);
    } catch (error) {
      last = error;
    }
  }
  throw new Error(`${url}: ${last}`);
}

const feedUrl = `https://www.reddit.com/r/${manifest.subreddit}/new/.rss`;
const feed = await fetchText(feedUrl);
const authorPosts = entries(feed).filter((entry) => entry.author === manifest.author);
const registered = new Map(manifest.sources.map((source) => [source.url, source]));
for (const post of authorPosts) post.processedIn = registered.get(post.url)?.blogFile ?? null;

const threadSnapshots = [];
const failedThreads = [];
for (const source of manifest.sources) {
  await sleep(1_500);
  try {
    const xml = await fetchText(`${source.url}.rss`);
    const items = entries(xml);
    threadSnapshots.push({
      version: source.version,
      url: source.url,
      blogFile: source.blogFile,
      entryCount: items.length,
      latestUpdate: items.map((item) => item.updated).sort().at(-1) ?? null,
      sha256: crypto.createHash("sha256").update(xml).digest("hex"),
    });
  } catch (error) {
    failedThreads.push({ url: source.url, error: String(error) });
  }
}

const evidence = {
  schemaVersion: 1,
  version,
  generatedAt: new Date().toISOString(),
  feedUrl,
  feedUpdated: field(feed, /<feed[\s\S]*?<updated>([^<]+)<\/updated>/),
  author: manifest.author,
  authorPosts,
  unprocessed: authorPosts.filter((post) => !post.processedIn).map((post) => post.url),
  threadSnapshots,
  failedThreads,
};
fs.writeFileSync(output, JSON.stringify(evidence, null, 2) + "\n");
console.log(`Wrote ${path.relative(root, output)}: ${authorPosts.length} author posts, ${threadSnapshots.length} discussion snapshots.`);
if (evidence.unprocessed.length || failedThreads.length) {
  if (evidence.unprocessed.length) console.error(`Unprocessed r/${manifest.subreddit} posts:\n${evidence.unprocessed.join("\n")}`);
  if (failedThreads.length) console.error(`Discussion refresh failures:\n${failedThreads.map((item) => `${item.url}: ${item.error}`).join("\n")}`);
  process.exit(1);
}
