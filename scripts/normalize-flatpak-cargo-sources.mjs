#!/usr/bin/env node

// flatpak-cargo-generator puts registry crates and git packages in one Cargo
// directory source. That becomes ambiguous when the lockfile contains the same
// name+version from both origins (currently tao-macros 0.1.3): Cargo can select
// the checksum-less git copy for the checksummed registry package and abort the
// offline build. Keep the generated downloads intact, but route git packages
// through a separate directory source.

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const sourcesPath = path.resolve(process.argv[2] ?? path.join(root, "flatpak/cargo-sources.json"));
const sources = JSON.parse(fs.readFileSync(sourcesPath, "utf8"));

const gitPackages = new Set();
for (const source of sources) {
  if (source.type !== "shell" || !Array.isArray(source.commands)) continue;
  source.commands = source.commands.map((command) => {
    if (!command.includes("flatpak-cargo/git/")) return command;
    const match = command.match(/"cargo\/(?:git-)?vendor\/([^/\"]+)"\s*$/);
    if (!match) throw new Error(`unrecognized generated git-vendor command: ${command}`);
    gitPackages.add(match[1]);
    return command.replace(`"cargo/vendor/${match[1]}"`, `"cargo/git-vendor/${match[1]}"`);
  });
}
if (!gitPackages.size) throw new Error("generated Flatpak sources contain no git packages");

for (const source of sources) {
  if (typeof source.dest !== "string") continue;
  for (const packageName of gitPackages) {
    if (source.dest === `cargo/vendor/${packageName}`) {
      source.dest = `cargo/git-vendor/${packageName}`;
    }
  }
}

const cargoConfig = sources.find(
  (source) => source.type === "inline" && source.dest === "cargo" && source["dest-filename"] === "config",
);
if (!cargoConfig || typeof cargoConfig.contents !== "string") {
  throw new Error("generated Flatpak sources contain no Cargo source-replacement config");
}

const lines = cargoConfig.contents.split("\n");
const normalized = [];
let section = "";
let insertedGitDirectory = lines.includes("[source.vendored-git]");
for (const line of lines) {
  if (line === "[source.vendored-sources]" || line === "[source.vendored-registry]") {
    section = "source.vendored-registry";
    normalized.push("[source.vendored-registry]");
    continue;
  }
  if (line === "[source.vendored-git]") {
    section = "source.vendored-git";
    insertedGitDirectory = true;
    normalized.push(line);
    continue;
  }
  if (line.startsWith("[source.")) section = line.slice(1, -1);
  if (line === 'directory = "cargo/vendor"' && section === "source.vendored-registry") {
    normalized.push(line);
    if (!insertedGitDirectory) {
      normalized.push("", "[source.vendored-git]", 'directory = "cargo/git-vendor"');
      insertedGitDirectory = true;
    }
    continue;
  }
  if (line.startsWith("replace-with = ")) {
    normalized.push(
      section === "source.crates-io"
        ? 'replace-with = "vendored-registry"'
        : 'replace-with = "vendored-git"',
    );
    continue;
  }
  normalized.push(line);
}
cargoConfig.contents = normalized.join("\n");

fs.writeFileSync(sourcesPath, `${JSON.stringify(sources, null, 4)}\n`);
console.log(
  `separated ${gitPackages.size} git package(s) from the Flatpak registry vendor: ${[...gitPackages].join(", ")}`,
);
