#!/usr/bin/env node

import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { releaseLayout } from "./release-layout.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const [lane, outputRoot = "release-candidate", commit = process.env.GITHUB_SHA] = process.argv.slice(2);
const version = JSON.parse(fs.readFileSync(path.join(root, "src-tauri/tauri.conf.json"), "utf8")).version;
const laneLayout = releaseLayout(version).lanes[lane];
if (!laneLayout) throw new Error(`unknown release lane: ${lane}`);
if (!/^[0-9a-f]{40}$/.test(commit ?? "")) throw new Error(`invalid source commit: ${commit}`);

function walk(directory, found = []) {
  if (!fs.existsSync(directory)) return found;
  for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
    const full = path.join(directory, entry.name);
    if (entry.isDirectory()) walk(full, found);
    else if (entry.isFile()) found.push(full);
  }
  return found;
}

const searchRoots = [path.join(root, "target"), path.join(root, "src-tauri", "gen", "android"), root];
const allFiles = [
  ...walk(searchRoots[0]),
  ...walk(searchRoots[1]),
  ...fs.readdirSync(root, { withFileTypes: true }).filter((entry) => entry.isFile()).map((entry) => path.join(root, entry.name)),
];
const destination = path.resolve(root, outputRoot, lane);
fs.rmSync(destination, { recursive: true, force: true });
fs.mkdirSync(destination, { recursive: true });

const assets = [];
for (const name of laneLayout.assets) {
  const matches = allFiles.filter((file) => path.basename(file) === name);
  if (matches.length !== 1) {
    throw new Error(`${lane}: expected exactly one ${name}, found ${matches.length}: ${matches.join(", ")}`);
  }
  const target = path.join(destination, name);
  fs.copyFileSync(matches[0], target);
  const bytes = fs.readFileSync(target);
  assets.push({
    name,
    size: bytes.length,
    sha256: createHash("sha256").update(bytes).digest("hex"),
  });
}

const platforms = {};
for (const [platform, [asset, signatureAsset]] of Object.entries(laneLayout.platforms)) {
  platforms[platform] = {
    asset,
    signature: fs.readFileSync(path.join(destination, signatureAsset), "utf8").trim(),
  };
}
const fragment = { version, commit, lane, assets, platforms };
fs.writeFileSync(path.join(destination, "release-fragment.json"), `${JSON.stringify(fragment, null, 2)}\n`);
console.log(`${lane}: staged ${assets.length} asset(s), ${Object.keys(platforms).length} updater entries.`);
