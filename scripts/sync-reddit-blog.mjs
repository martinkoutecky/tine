#!/usr/bin/env node

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { buildRedditReleaseEvidence } from "./reddit-release-evidence-lib.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const version = process.argv.find((arg) => arg.startsWith("--version="))?.slice(10);
if (!/^\d+\.\d+\.0$/.test(version ?? "")) {
  console.error("usage: npm run blog:sync -- --version=X.Y.0");
  process.exit(2);
}

const manifest = JSON.parse(fs.readFileSync(path.join(root, "website/blog/reddit-sources.json"), "utf8"));
const output = path.join(root, `docs/releases/v${version}-reddit.json`);

function atomicWriteJson(file, value) {
  const temporary = `${file}.tmp-${process.pid}-${Date.now()}`;
  try {
    fs.writeFileSync(temporary, `${JSON.stringify(value, null, 2)}\n`, { flag: "wx" });
    fs.renameSync(temporary, file);
  } finally {
    if (fs.existsSync(temporary)) fs.rmSync(temporary);
  }
}

let evidence;
try {
  evidence = await buildRedditReleaseEvidence({ manifest, version });
} catch (error) {
  // Construction failures precede any trustworthy feed/thread snapshot. Still
  // replace prior evidence with an explicit failed current attempt.
  evidence = {
    schemaVersion: 2,
    transport: "reddit-json",
    complete: false,
    version,
    generatedAt: new Date().toISOString(),
    feedUrl: `https://www.reddit.com/user/${encodeURIComponent(manifest.author ?? "unknown")}/submitted.json?raw_json=1&limit=100`,
    feedPages: 0,
    feedAfter: null,
    feedErrors: [{ error: String(error) }],
    feedUpdated: null,
    author: manifest.author,
    authorPosts: [],
    unprocessed: [],
    threadSnapshots: [],
    failedThreads: [],
  };
}

atomicWriteJson(output, evidence);
console.log(
  `Wrote ${path.relative(root, output)} via Reddit JSON: ${evidence.authorPosts.length} author posts, `
  + `${evidence.threadSnapshots.length} complete discussion snapshots.`,
);
if (!evidence.complete) {
  if (evidence.feedErrors.length) {
    console.error(`Author-feed failures:\n${evidence.feedErrors.map((item) => `${item.url ?? evidence.feedUrl}: ${item.error}`).join("\n")}`);
  }
  if (evidence.unprocessed.length) console.error(`Unprocessed r/${manifest.subreddit} posts:\n${evidence.unprocessed.join("\n")}`);
  if (evidence.failedThreads.length) {
    console.error(`Discussion refresh failures:\n${evidence.failedThreads.map((item) => `${item.url}: ${item.error}`).join("\n")}`);
  }
  process.exit(1);
}
