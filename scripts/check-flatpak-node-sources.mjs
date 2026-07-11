#!/usr/bin/env node

// Fast guard for the generated Flatpak offline npm cache. The full Flatpak
// build is intentionally heavyweight; this catches the common stale-manifest
// failure on ordinary CI before a release is tagged.

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const lockPath = path.resolve(process.argv[2] ?? path.join(root, "package-lock.json"));
const sourcesPath = path.resolve(process.argv[3] ?? path.join(root, "flatpak/node-sources.json"));

const lock = JSON.parse(fs.readFileSync(lockPath, "utf8"));
const sources = JSON.parse(fs.readFileSync(sourcesPath, "utf8"));
const available = new Set(
  sources
    .map((source) => source?.url)
    .filter((url) => typeof url === "string")
);

const required = new Set();
for (const pkg of Object.values(lock.packages ?? {})) {
  const url = pkg?.resolved;
  if (typeof url === "string" && url.startsWith("https://registry.npmjs.org/")) {
    required.add(url);
  }
}

const missing = [...required].filter((url) => !available.has(url)).sort();
if (missing.length) {
  console.error(
    `Flatpak node-sources is stale: ${missing.length} package tarball(s) from ${lockPath} are missing in ${sourcesPath}:`
  );
  for (const url of missing) console.error(`  ${url}`);
  console.error("Regenerate flatpak/node-sources.json before merging or releasing.");
  process.exit(1);
}

console.log(`Flatpak node-sources covers ${required.size} npm tarballs.`);
