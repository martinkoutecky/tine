#!/usr/bin/env node
// tine#1011: Tine Beta installs next to Tine, so its .deb and .rpm must share no
// file with stable's and declare no relation to another package. Stable's
// published packages are the reference: this compares the packages just built
// with them, file list against file list.
//
//   node scripts/check-linux-package-paths.mjs <dir-or-package>... [--reference <package>...]
//
// Without --reference it downloads the latest stable release's .deb and .rpm
// (gh, GH_TOKEN). A stable build has nothing to compare with and passes.
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { IDENTITIES, SHIP } from "./lib/app-identity.mjs";

const run = (command, args, options = {}) => execFileSync(command, args, { encoding: "utf8", maxBuffer: 64 << 20, ...options });
const packagesIn = (arg) => (fs.statSync(arg).isDirectory()
  ? fs.readdirSync(arg, { recursive: true }).map((name) => path.join(arg, name))
  : [arg]).filter((file) => /\.(deb|rpm)$/.test(file));

/** Every non-directory path a package installs, without a leading "./" or "/". */
function files(file) {
  const names = file.endsWith(".deb")
    ? run("dpkg-deb", ["-c", file]).split("\n").filter((line) => line && !line.startsWith("d"))
      .map((line) => line.match(/^\S+\s+\S+\s+\d+\s+\S+\s+\S+\s+(.*?)(?: -> .*)?$/)[1])
    : run("rpm", ["-qp", "--queryformat", "[%{FILEMODES:perms} %{FILENAMES}\n]", file])
      .split("\n").filter((line) => line && !line.startsWith("d")).map((line) => line.slice(line.indexOf(" ") + 1));
  return new Set(names.map((name) => name.replace(/^\.?\//, "")));
}

/** Relations to other packages that would stop a side-by-side install. */
function relations(file) {
  if (file.endsWith(".deb")) {
    return ["Conflicts", "Replaces", "Breaks", "Provides"]
      .map((field) => [field, run("dpkg-deb", ["-f", file, field]).trim()]).filter(([, value]) => value);
  }
  return ["--conflicts", "--obsoletes"]
    .map((flag) => [flag.slice(2), run("rpm", ["-qp", flag, file]).trim()]).filter(([, value]) => value);
}

const argv = process.argv.slice(2);
const split = argv.indexOf("--reference");
const built = (split < 0 ? argv : argv.slice(0, split)).flatMap(packagesIn);
if (!built.length) throw new Error("no .deb or .rpm to check");
if (SHIP === "release") {
  console.log(`${IDENTITIES[SHIP].productName} is the reference identity: nothing to compare.`);
  process.exit(0);
}
let reference = split < 0 ? [] : argv.slice(split + 1).flatMap(packagesIn);
if (split < 0) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "tine-stable-packages-"));
  run("gh", ["release", "download", "--repo", process.env.GITHUB_REPOSITORY ?? "martinkoutecky/tine",
    "--pattern", "*_amd64.deb", "--pattern", "*.x86_64.rpm", "--dir", dir], { stdio: "inherit" });
  reference = packagesIn(dir);
}
let failed = false;
for (const file of built) {
  const kind = path.extname(file);
  const stable = reference.filter((other) => other.endsWith(kind));
  if (!stable.length) throw new Error(`no stable ${kind} to compare ${file} with`);
  const own = files(file);
  for (const other of stable) {
    const shared = [...files(other)].filter((name) => own.has(name)).sort();
    if (!shared.length) continue;
    failed = true;
    console.error(`${path.basename(file)} shares ${shared.length} path(s) with ${path.basename(other)}:\n  ${shared.join("\n  ")}`);
  }
  for (const [field, value] of relations(file)) {
    failed = true;
    console.error(`${path.basename(file)} declares ${field}: ${value}`);
  }
  if (!failed) console.log(`${path.basename(file)}: ${own.size} paths, none shared with ${stable.map((other) => path.basename(other)).join(", ")}`);
}
if (failed) process.exit(1);
