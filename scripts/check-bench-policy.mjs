#!/usr/bin/env node

import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const policy = JSON.parse(fs.readFileSync(path.join(root, "scripts/bench-policy.json"), "utf8"));
const version = JSON.parse(fs.readFileSync(path.join(root, "src-tauri/tauri.conf.json"), "utf8")).version;
const problems = [];

function argument(name) {
  const index = process.argv.indexOf(name);
  return index >= 0 ? process.argv[index + 1] : undefined;
}

function reachableReleaseTags() {
  const output = execFileSync("git", ["tag", "--merged", "HEAD", "--sort=-version:refname"], {
    cwd: root,
    encoding: "utf8",
  });
  return output
    .split(/\r?\n/)
    .filter((tag) => /^v\d+\.\d+\.\d+$/.test(tag));
}

let expectedPrevious = argument("--expected-previous");
if (!expectedPrevious) {
  const candidateTag = `v${version}`;
  const workflowTag = process.env.GITHUB_REF?.startsWith("refs/tags/")
    ? process.env.GITHUB_REF.slice("refs/tags/".length)
    : undefined;
  let tags = reachableReleaseTags();

  // A tagged candidate still compares with the release before itself. Manual
  // candidate runs and ordinary master builds have no candidate tag at HEAD.
  const candidateAtHead = execFileSync("git", ["tag", "--points-at", "HEAD"], {
    cwd: root,
    encoding: "utf8",
  })
    .split(/\r?\n/)
    .includes(candidateTag);
  if (workflowTag === candidateTag || candidateAtHead) tags = tags.filter((tag) => tag !== candidateTag);
  expectedPrevious = tags[0];
}

if (!/^v\d+\.\d+\.\d+$/.test(policy.immutableBaseline?.ref ?? "")) {
  problems.push("immutableBaseline.ref is not a release tag");
}
if (!expectedPrevious) {
  problems.push("could not determine the most recent published release tag; fetch full tag history");
} else if (policy.previousRelease?.ref !== expectedPrevious) {
  problems.push(
    `previousRelease.ref is ${policy.previousRelease?.ref ?? "missing"}; expected most recent published release ${expectedPrevious}`
  );
}
if (policy.immutableBaseline?.ref !== "v0.4.7") {
  problems.push(`immutableBaseline.ref moved from the fixed v0.4.7 anchor to ${policy.immutableBaseline?.ref ?? "missing"}`);
}

if (problems.length) {
  console.error(`Benchmark policy failed (${problems.length} problem(s)):`);
  for (const problem of problems) console.error(`  ${problem}`);
  process.exit(1);
}

console.log(
  `Benchmark policy OK: immutable ${policy.immutableBaseline.ref}, previous ${policy.previousRelease.ref}.`
);
