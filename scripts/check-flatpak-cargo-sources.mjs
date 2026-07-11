#!/usr/bin/env node

// Fast guard for the generated Flatpak offline Cargo vendor manifest. The full
// Flatpak build is intentionally heavyweight; this catches stale registry and
// git dependency sources on ordinary CI before a release is tagged.

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const lockPath = path.resolve(process.argv[2] ?? path.join(root, "Cargo.lock"));
const sourcesPath = path.resolve(process.argv[3] ?? path.join(root, "flatpak/cargo-sources.json"));

const lockText = fs.readFileSync(lockPath, "utf8");
const sources = JSON.parse(fs.readFileSync(sourcesPath, "utf8"));

const field = (block, name) =>
  block.match(new RegExp(`^${name} = "([^"]+)"$`, "m"))?.[1];
const packages = lockText
  .split(/\n\[\[package\]\]\n/)
  .slice(1)
  .map((block) => ({
    name: field(block, "name"),
    version: field(block, "version"),
    source: field(block, "source"),
    checksum: field(block, "checksum"),
  }))
  .filter((pkg) => pkg.name && pkg.version && pkg.source);

const archives = new Map(
  sources
    .filter((source) => source?.type === "archive" && typeof source.url === "string")
    .map((source) => [source.url, source.sha256])
);
const gitSources = sources.filter(
  (source) => source?.type === "git" && typeof source.url === "string"
);
const inlineCargoTomls = sources.filter(
  (source) => source?.type === "inline" && source["dest-filename"] === "Cargo.toml"
);

const problems = [];
let registryCount = 0;
let gitCount = 0;

for (const pkg of packages) {
  if (pkg.source.startsWith("registry+https://github.com/rust-lang/crates.io-index")) {
    registryCount += 1;
    const url = `https://static.crates.io/crates/${pkg.name}/${pkg.name}-${pkg.version}.crate`;
    const actualChecksum = archives.get(url);
    if (actualChecksum !== pkg.checksum) {
      problems.push(
        `${pkg.name} ${pkg.version}: expected archive checksum ${pkg.checksum}, found ${actualChecksum ?? "no archive"}`
      );
    }
    continue;
  }

  if (pkg.source.startsWith("git+")) {
    gitCount += 1;
    const parsed = new URL(pkg.source.slice(4));
    const commit = parsed.hash.slice(1);
    const tag = parsed.searchParams.get("tag");
    parsed.hash = "";
    parsed.search = "";
    const url = parsed.toString().replace(/\/$/, "");
    const gitSource = gitSources.find((source) => source.url === url);
    if (gitSource?.commit !== commit) {
      problems.push(
        `${pkg.name} ${pkg.version}: expected ${url} at ${commit}, found ${gitSource?.commit ?? "no git source"}`
      );
    }

    const cargoToml = inlineCargoTomls.find(
      (source) =>
        source.contents?.includes(`[package]\nname = "${pkg.name}"\nversion = "${pkg.version}"`)
    );
    if (!cargoToml) {
      problems.push(`${pkg.name} ${pkg.version}: generated vendor Cargo.toml is missing or stale`);
    }

    if (tag) {
      const config = sources.find(
        (source) =>
          source?.type === "inline" &&
          source.dest === "cargo" &&
          source.contents?.includes(`[source."${url}"]`) &&
          source.contents?.includes(`tag = "${tag}"`)
      );
      if (!config) {
        problems.push(`${pkg.name} ${pkg.version}: Cargo source redirect does not pin tag ${tag}`);
      }
    }
  }
}

if (problems.length) {
  console.error(
    `Flatpak cargo-sources is stale relative to ${lockPath} (${problems.length} problem(s)):`
  );
  for (const problem of problems) console.error(`  ${problem}`);
  console.error("Regenerate flatpak/cargo-sources.json before merging or releasing.");
  process.exit(1);
}

console.log(
  `Flatpak cargo-sources covers ${registryCount} registry packages and ${gitCount} git package(s).`
);
