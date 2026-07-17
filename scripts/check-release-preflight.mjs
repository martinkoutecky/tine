#!/usr/bin/env node

// Cheap release metadata guard. Run on ordinary/manual CI and again before a
// tagged release creates its draft, so a mistyped tag or partial version bump
// cannot start publishing platform artifacts.

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const readJson = (relative) => JSON.parse(fs.readFileSync(path.join(root, relative), "utf8"));
const tauri = readJson("src-tauri/tauri.conf.json");
const pkg = readJson("package.json");
const pkgLock = readJson("package-lock.json");
const cargo = fs.readFileSync(path.join(root, "Cargo.toml"), "utf8");
const cargoLock = fs.readFileSync(path.join(root, "Cargo.lock"), "utf8");
const changelog = fs.readFileSync(path.join(root, "CHANGELOG.md"), "utf8");

const cargoVersion = cargo.match(/^version = "([^"]+)"$/m)?.[1];
const lockedWorkspaceVersions = new Map(
  cargoLock
    .split(/\n\[\[package\]\]\n/)
    .slice(1)
    .map((block) => [
      block.match(/^name = "([^"]+)"$/m)?.[1],
      block.match(/^version = "([^"]+)"$/m)?.[1],
    ])
    .filter(([name]) => name === "tine" || name === "tine-core")
);
const versions = new Map([
  ["src-tauri/tauri.conf.json", tauri.version],
  ["package.json", pkg.version],
  ["package-lock.json", pkgLock.version],
  ["package-lock.json root package", pkgLock.packages?.[""]?.version],
  ["Cargo.toml workspace", cargoVersion],
  ["Cargo.lock tine", lockedWorkspaceVersions.get("tine")],
  ["Cargo.lock tine-core", lockedWorkspaceVersions.get("tine-core")],
]);
const expected = tauri.version;
const problems = [];

const benchPolicy = spawnSync(process.execPath, [path.join(root, "scripts", "check-bench-policy.mjs")], {
  encoding: "utf8",
  env: process.env,
});
if (benchPolicy.status !== 0) {
  problems.push(`check-bench-policy.mjs failed:\n${benchPolicy.stderr || benchPolicy.stdout}`);
}

for (const [source, version] of versions) {
  if (version !== expected) problems.push(`${source} has ${version ?? "no version"}; expected ${expected}`);
}

const parts = expected.split(".").map(Number);
if (parts.length !== 3 || parts.some((part) => !Number.isSafeInteger(part) || part < 0)) {
  problems.push(`Tauri version is not a three-part numeric semver: ${expected}`);
} else {
  const expectedCode = parts[0] * 1_000_000 + parts[1] * 1_000 + parts[2];
  if (tauri.bundle?.android?.versionCode !== expectedCode) {
    problems.push(
      `Android versionCode is ${tauri.bundle?.android?.versionCode ?? "missing"}; expected ${expectedCode}`
    );
  }
}

if (!new RegExp(`^## \\[${expected.replaceAll(".", "\\.")}\\] - \\d{4}-\\d{2}-\\d{2}$`, "m").test(changelog)) {
  problems.push(`CHANGELOG.md has no dated [${expected}] release heading`);
}

if (process.env.GITHUB_REF?.startsWith("refs/tags/")) {
  const tag = process.env.GITHUB_REF.slice("refs/tags/".length);
  if (tag !== `v${expected}`) problems.push(`tag ${tag} does not match metadata version v${expected}`);
}

if (process.env.REQUIRE_RELEASE_READINESS === "1") {
  for (const [script, args = []] of [
    ["check-regression-catalog.mjs"],
    ["check-release-readiness.mjs"],
    ["check-reddit-blog.mjs"],
    ["build-guide-demo.mjs", ["--check"]],
  ]) {
    const result = spawnSync(process.execPath, [path.join(root, "scripts", script), ...args], { encoding: "utf8" });
    if (result.status !== 0) problems.push(`${script} failed:\n${result.stderr || result.stdout}`);
  }
}

if (problems.length) {
  console.error(`Release preflight failed (${problems.length} problem(s)):`);
  for (const problem of problems) console.error(`  ${problem}`);
  process.exit(1);
}

console.log(`Release preflight OK: v${expected}, Android code ${tauri.bundle.android.versionCode}.`);
