#!/usr/bin/env node

import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { candidateProblems, releaseLayout, releaseNotes, RELEASE_LANES } from "./release-layout.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

function findFragments(directory, found = []) {
  for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
    const full = path.join(directory, entry.name);
    if (entry.isDirectory()) findFragments(full, found);
    else if (entry.isFile() && entry.name === "release-fragment.json") found.push(full);
  }
  return found;
}

export function assembleCandidate({ input, output, version, commit, repository, pubDate = new Date().toISOString() }) {
  const layout = releaseLayout(version);
  const fragments = findFragments(input).map((file) => ({ file, value: JSON.parse(fs.readFileSync(file, "utf8")) }));
  const byLane = new Map();
  for (const fragment of fragments) {
    const value = fragment.value;
    if (!RELEASE_LANES.includes(value.lane)) throw new Error(`unknown fragment lane ${value.lane}`);
    if (byLane.has(value.lane)) throw new Error(`duplicate fragment lane ${value.lane}`);
    if (value.version !== version) throw new Error(`${value.lane}: version ${value.version}, expected ${version}`);
    if (value.commit !== commit) throw new Error(`${value.lane}: commit ${value.commit}, expected ${commit}`);
    byLane.set(value.lane, fragment);
  }
  const missingLanes = RELEASE_LANES.filter((lane) => !byLane.has(lane));
  if (missingLanes.length) throw new Error(`missing release lanes: ${missingLanes.join(", ")}`);

  fs.rmSync(output, { recursive: true, force: true });
  fs.mkdirSync(output, { recursive: true });
  const platforms = {};
  const seenAssets = new Set();
  for (const lane of RELEASE_LANES) {
    const { file, value } = byLane.get(lane);
    const expectedAssets = layout.lanes[lane].assets;
    const actualAssets = value.assets.map((asset) => asset.name);
    if (expectedAssets.length !== actualAssets.length || expectedAssets.some((name) => !actualAssets.includes(name))) {
      throw new Error(`${lane}: fragment asset contract mismatch`);
    }
    for (const asset of value.assets) {
      if (seenAssets.has(asset.name)) throw new Error(`duplicate release asset ${asset.name}`);
      seenAssets.add(asset.name);
      const source = path.join(path.dirname(file), asset.name);
      const bytes = fs.readFileSync(source);
      const digest = createHash("sha256").update(bytes).digest("hex");
      if (bytes.length !== asset.size || digest !== asset.sha256) throw new Error(`${lane}: checksum mismatch for ${asset.name}`);
      fs.copyFileSync(source, path.join(output, asset.name));
    }
    const expectedPlatformKeys = Object.keys(layout.lanes[lane].platforms);
    const actualPlatformKeys = Object.keys(value.platforms ?? {});
    if (
      expectedPlatformKeys.length !== actualPlatformKeys.length ||
      expectedPlatformKeys.some((platform) => !actualPlatformKeys.includes(platform))
    ) {
      throw new Error(`${lane}: updater platform contract mismatch`);
    }
    for (const [platform, entry] of Object.entries(value.platforms ?? {})) {
      if (platforms[platform]) throw new Error(`duplicate updater platform ${platform}`);
      const [expectedAsset, signatureAsset] = layout.updaterPlatforms[platform];
      if (entry.asset !== expectedAsset) throw new Error(`${lane}: ${platform} points at ${entry.asset}`);
      const signature = fs.readFileSync(path.join(path.dirname(file), signatureAsset), "utf8").trim();
      if (!signature || entry.signature !== signature) throw new Error(`${lane}: signature mismatch for ${platform}`);
      platforms[platform] = {
        signature,
        url: `https://github.com/${repository}/releases/latest/download/${entry.asset}`,
      };
    }
  }

  const updater = { version, notes: releaseNotes(root, version), pub_date: pubDate, platforms };
  fs.writeFileSync(path.join(output, "latest.json"), `${JSON.stringify(updater, null, 2)}\n`);
  const problems = candidateProblems(output, version);
  if (problems.length) throw new Error(`candidate verification failed:\n  ${problems.join("\n  ")}`);
  console.log(`Release candidate OK: v${version}, ${layout.allAssets.length} assets, ${Object.keys(platforms).length} updater platforms.`);
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const [inputArg, outputArg = "release-candidate-assembled"] = process.argv.slice(2);
  const version = JSON.parse(fs.readFileSync(path.join(root, "src-tauri/tauri.conf.json"), "utf8")).version;
  if (!inputArg) throw new Error("usage: assemble-release-candidate.mjs INPUT [OUTPUT]");
  assembleCandidate({
    input: path.resolve(inputArg),
    output: path.resolve(outputArg),
    version,
    commit: process.env.GITHUB_SHA,
    repository: process.env.GITHUB_REPOSITORY ?? "martinkoutecky/tine",
  });
}
