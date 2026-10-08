#!/usr/bin/env node

import { execFileSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { candidateProblems, releaseLayout, releaseNotes } from "./release-layout.mjs";

import { IDENTITY } from "./lib/app-identity.mjs";
import { publicationPlan, releaseTag } from "./release-policy.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const [tag, candidateArg] = process.argv.slice(2);
if (!candidateArg) throw new Error("usage: publish-release-candidate.mjs TAG CANDIDATE_DIR (beta, or v<version> for stable)");
const conf = JSON.parse(fs.readFileSync(path.join(root, "src-tauri/tauri.conf.json"), "utf8"));
const plan = publicationPlan({ conf, tag, mode: process.env.RELEASE_MODE, publish: process.env.RELEASE_PUBLISH === "true" });
const version = conf.version;
const candidate = path.resolve(candidateArg);
const layout = releaseLayout(version);
const repository = process.env.GITHUB_REPOSITORY;
const commit = process.env.GITHUB_SHA;
if (!repository || !/^[0-9a-f]{40}$/.test(commit ?? "")) throw new Error("GITHUB_REPOSITORY and GITHUB_SHA are required");

const problems = candidateProblems(candidate, version, plan.channel);
if (problems.length) throw new Error(`local candidate is invalid:\n  ${problems.join("\n  ")}`);
if (!plan.publish) {
  console.log(`Verified ${releaseTag(conf)} candidate; publish=false, no remote mutation.`);
  process.exit(0);
}
const gh = (...args) => execFileSync("gh", args, { encoding: "utf8", stdio: ["ignore", "pipe", "inherit"] });
const tagCommit = gh("api", `repos/${repository}/commits/${tag}`, "--jq", ".sha").trim();
if (tagCommit !== commit) throw new Error(`${tag} resolves to ${tagCommit}, expected ${commit}`);

const viewed = spawnSync("gh", ["release", "view", tag, "--json", "assets,isDraft,tagName"], {
  encoding: "utf8",
  stdio: ["ignore", "pipe", "pipe"],
});
let release;
if (viewed.status === 0) {
  release = JSON.parse(viewed.stdout);
  if (!release.isDraft) throw new Error(`${tag} is already public; refusing to mutate it`);
}

if (!release) {
  const temporary = fs.mkdtempSync(path.join(os.tmpdir(), `tine-release-notes-${version}-`));
  try {
    const notesPath = path.join(temporary, "notes.md");
    const fallback = `Download an installer for your platform below. Windows and Linux ship both x64 and ARM64 builds (match your CPU). Windows users who prefer no installer can grab the portable \`${layout.lanes["windows-x64"].assets.find((name) => name.endsWith("-portable.zip"))}\` for their architecture. Windows builds are currently unsigned, so Windows may show a warning on first launch. The macOS build is signed and notarized. On macOS, if Tine repeatedly asks to access Documents, see the workaround in the README.`;
    fs.writeFileSync(notesPath, `${releaseNotes(root, version)}\n\n---\n\n${fallback}\n`);
    gh("release", "create", tag, "--draft", `--prerelease=${plan.prerelease}`, "--latest=false", "--target", commit, "--title", `${IDENTITY.productName} ${version}`, "--notes-file", notesPath);
  } finally {
    fs.rmSync(temporary, { recursive: true, force: true });
  }
  release = JSON.parse(gh("release", "view", tag, "--json", "assets,isDraft,tagName"));
}

for (const asset of release.assets) gh("release", "delete-asset", tag, asset.name, "--yes");
const platformPaths = layout.platformAssets.map((name) => path.join(candidate, name));
gh("release", "upload", tag, ...platformPaths, "--clobber");
gh("release", "upload", tag, path.join(candidate, "latest.json"), "--clobber");
execFileSync(process.execPath, [path.join(root, "scripts", "check-release-assets.mjs"), tag, version], {
  stdio: "inherit",
  env: process.env,
});
gh("release", "edit", tag, "--draft=false", `--prerelease=${plan.prerelease}`, `--latest=${plan.latest}`);
console.log(`Published complete release ${tag}.`);
