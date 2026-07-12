#!/usr/bin/env node

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const blog = path.join(root, "website/blog");
const manifest = JSON.parse(fs.readFileSync(path.join(blog, "reddit-sources.json"), "utf8"));
const problems = [];
if (manifest.schemaVersion !== 1) problems.push("schemaVersion must be 1");
if (!/^[A-Za-z0-9_-]+$/.test(manifest.author ?? "")) problems.push("author is invalid");
if (!/^[A-Za-z0-9_]+$/.test(manifest.subreddit ?? "")) problems.push("subreddit is invalid");
if (!Array.isArray(manifest.sources)) problems.push("sources must be an array");
else {
  const urls = new Set();
  for (const source of manifest.sources) {
    if (!/^\d+\.\d+\.0$/.test(source.version ?? "")) problems.push(`source version is not minor: ${source.version}`);
    if (!/^https:\/\/www\.reddit\.com\/r\/[^/]+\/comments\/[^/]+\/[^/]+\/$/.test(source.url ?? "")) {
      problems.push(`invalid source URL for ${source.version}`);
    }
    if (urls.has(source.url)) problems.push(`duplicate source URL ${source.url}`);
    urls.add(source.url);
    const file = path.join(blog, source.blogFile ?? "");
    if (!fs.existsSync(file)) problems.push(`missing blog file ${source.blogFile}`);
    else {
      const html = fs.readFileSync(file, "utf8");
      if (!html.includes(source.url)) problems.push(`${source.blogFile} does not link its source thread`);
      if (!html.includes(`v${source.version}`)) problems.push(`${source.blogFile} does not identify v${source.version}`);
    }
  }
}
if (problems.length) {
  console.error(`Reddit/blog sources failed (${problems.length} problem(s)):`);
  for (const problem of problems) console.error(`  ${problem}`);
  process.exit(1);
}
console.log(`Reddit/blog sources OK: ${manifest.sources.length} minor release threads.`);
