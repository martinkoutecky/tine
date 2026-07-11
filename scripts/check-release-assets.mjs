#!/usr/bin/env node

// Verify the exact public/draft release payload before the workflow flips the
// draft to published. Uses authenticated gh so it can inspect draft releases.

import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { candidateProblems, releaseLayout } from "./release-layout.mjs";

const tag = process.argv[2];
if (!/^v\d+\.\d+\.\d+$/.test(tag ?? "")) {
  console.error("usage: check-release-assets.mjs vX.Y.Z");
  process.exit(2);
}
const version = tag.slice(1);
const layout = releaseLayout(version);
const gh = (...args) =>
  execFileSync("gh", args, { encoding: "utf8", stdio: ["ignore", "pipe", "inherit"] });

const release = JSON.parse(gh("release", "view", tag, "--json", "assets,tagName"));
const names = new Set(release.assets.map((asset) => asset.name));
const missing = layout.allAssets.filter((name) => !names.has(name));
const unexpected = [...names].filter((name) => !layout.allAssets.includes(name)).sort();
const problems = [];
if (release.tagName !== tag) problems.push(`release tag is ${release.tagName}, expected ${tag}`);
if (missing.length) problems.push(`missing assets: ${missing.join(", ")}`);
if (unexpected.length) problems.push(`unexpected assets: ${unexpected.join(", ")}`);

const temp = fs.mkdtempSync(path.join(os.tmpdir(), `tine-release-${version}-`));
try {
  gh("release", "download", tag, "--pattern", "latest.json", "--dir", temp, "--clobber");
  const local = path.join(temp, "candidate");
  fs.mkdirSync(local);
  for (const name of layout.platformAssets) fs.writeFileSync(path.join(local, name), "remote-asset-present");
  fs.copyFileSync(path.join(temp, "latest.json"), path.join(local, "latest.json"));
  problems.push(...candidateProblems(local, version));
} finally {
  fs.rmSync(temp, { recursive: true, force: true });
}

if (problems.length) {
  console.error(`Release asset verification failed (${problems.length} problem(s)):`);
  for (const problem of problems) console.error(`  ${problem}`);
  process.exit(1);
}

console.log(`Release assets OK: ${tag}, ${layout.allAssets.length} assets, ${Object.keys(layout.updaterPlatforms).length} updater platforms.`);
