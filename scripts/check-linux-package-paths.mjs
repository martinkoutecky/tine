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

/**
 * Every path a package installs, without a leading "./" or "/", mapped to
 * whether it is a directory. Two packages may share a directory, nothing else.
 */
function files(file) {
  // The name list and the long list come in the same order; a link's target appears only in the long one.
  const tar = (flags) => run("sh", ["-c", `dpkg-deb --fsys-tarfile "$1" | tar ${flags}`, "sh", file]).split("\n").filter(Boolean);
  const entries = file.endsWith(".deb")
    ? ((types) => tar("-t").map((name, index) => [name, types[index][0] === "d"]))(tar("-tv"))
    : run("rpm", ["-qp", "--queryformat", "[%{FILEMODES:perms} %{FILENAMES}\n]", file])
      .split("\n").filter(Boolean).map((line) => [line.slice(line.indexOf(" ") + 1), line[0] === "d"]);
  return new Map(entries.map(([name, directory]) => [name.replace(/^\.?\//, "").replace(/\/$/, ""), directory]));
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
const downloads = split < 0 ? fs.mkdtempSync(path.join(os.tmpdir(), "tine-stable-packages-")) : null;
if (downloads) {
  process.on("exit", () => fs.rmSync(downloads, { recursive: true, force: true }));
  // Every architecture's package: a file only one of them installs counts too.
  run("gh", ["release", "download", "--repo", process.env.GITHUB_REPOSITORY ?? "martinkoutecky/tine",
    "--pattern", "*.deb", "--pattern", "*.rpm", "--dir", downloads], { stdio: "inherit" });
  reference = packagesIn(downloads);
}
let failed = false;
for (const file of built) {
  const kind = path.extname(file);
  const stable = reference.filter((other) => other.endsWith(kind));
  if (!stable.length) throw new Error(`no stable ${kind} to compare ${file} with`);
  const own = files(file);
  for (const other of stable) {
    const shared = [...files(other)].filter(([name, directory]) => own.has(name) && !(directory && own.get(name)))
      .map(([name]) => name).sort();
    if (!shared.length) continue;
    failed = true;
    console.error(`${path.basename(file)} shares ${shared.length} path(s) with ${path.basename(other)}:\n  ${shared.join("\n  ")}`);
  }
  for (const [field, value] of relations(file)) {
    failed = true;
    console.error(`${path.basename(file)} declares ${field}: ${value}`);
  }
  if (!failed) console.log(`${path.basename(file)}: ${[...own.values()].filter((directory) => !directory).length} files, none shared with ${stable.map((other) => path.basename(other)).join(", ")}`);
}
if (failed) process.exit(1);
