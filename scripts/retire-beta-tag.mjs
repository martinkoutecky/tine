#!/usr/bin/env node
// One-time move off the moving `beta` tag (docs/VERSIONING.md, discussion #682).
//
// Betas up to 0.8.0-beta.2 read their updates from the GitHub release at tag
// `beta` (LEGACY_BETA_ENDPOINT) and their AppImages read zsync files from it.
// From 0.8.0-beta.3 each Beta is its own release v<version> and Betas read
// tine.page. Run this once, after the first such Beta is published and
// tine.page serves it:
//
//   node scripts/retire-beta-tag.mjs v0.8.0-beta.3            # dry run: report only
//   node scripts/retire-beta-tag.mjs v0.8.0-beta.3 --execute  # perform and verify
//
// It (1) moves the release now at `beta` to its own tag v<its version>,
// (2) re-creates `beta` at the new Beta's commit as a frozen bridge release
// holding only the new Beta's latest.json and AppImage zsync files (whose URLs
// point into the new Beta's release) plus a migration page for builds that
// update by hand, and (3) checks the bridge from the public URLs. The bridge is
// kept permanently: an install that has not updated yet still reads it.

import { execFileSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { FIRST_FIXED_MANIFEST_BETA, betaVersion, servingProblems, zsyncAssetName, ZSYNC_ARCHES } from "./pages-beta-manifest.mjs";
import { IDENTITY } from "./lib/app-identity.mjs";
import { releaseLayout } from "./release-layout.mjs";
import { BETA_ENDPOINT, BETA_TAG, LEGACY_BETA_ENDPOINT } from "./release-policy.mjs";

export function bridgeNotes(repository, tag, layout) {
  const base = `https://github.com/${repository}/releases/download/${tag}/`;
  const find = (suffix) => layout.platformAssets.find((name) => name.endsWith(suffix));
  const links = [
    ["macOS (Apple Silicon and Intel)", find("_universal.dmg")],
    ["Windows x64 installer", find("_x64-setup.exe")],
    ["Windows ARM64 installer", find("_arm64-setup.exe")],
    ["Windows 32-bit installer", find("_x86-setup.exe")],
    ["Linux x64 AppImage", find("_amd64.AppImage")],
    ["Linux ARM64 AppImage", find("_aarch64.AppImage")],
  ].filter(([, asset]) => asset);
  return [
    `This release only helps older ${IDENTITY.productName} installs find their next update. It contains no app.`,
    ``,
    `**Get the current Beta from [${tag}](https://github.com/${repository}/releases/tag/${tag})**, or download it directly:`,
    ``,
    ...links.map(([label, asset]) => `- ${label}: [${asset}](${base}${asset})`),
    ``,
    `Install it over your current ${IDENTITY.productName} and reopen it. From ${tag} on, ${IDENTITY.productName} updates itself from each Beta's own release, so you will not land on this page again.`,
    ``,
    `(Betas before ${tag} read the update information in this release. It stays here, pointing at ${tag}, so that any of them can still update.)`,
  ].join("\n");
}

function main() {
  const [tag, flag] = process.argv.slice(2);
  const execute = flag === "--execute";
  const repository = process.env.GITHUB_REPOSITORY ?? "martinkoutecky/tine";
  const version = betaVersion(tag);
  const floor = betaVersion(`v${FIRST_FIXED_MANIFEST_BETA}`);
  const first = version?.findIndex((part, i) => part !== floor[i]) ?? -1;
  if (!version || (first >= 0 && version[first] < floor[first])) {
    throw new Error(`usage: retire-beta-tag.mjs v<X.Y.Z-beta.N ≥ ${FIRST_FIXED_MANIFEST_BETA}> [--execute]`);
  }
  const gh = (...args) => execFileSync("gh", [...args, "--repo", repository], { encoding: "utf8", stdio: ["ignore", "pipe", "inherit"] });
  const ghApi = (...args) => execFileSync("gh", ["api", ...args], { encoding: "utf8", stdio: ["ignore", "pipe", "inherit"] });
  const fetchText = (url) => execFileSync("curl", ["-fsSL", url], { encoding: "latin1" });
  const step = (text, action) => {
    console.log(`${execute ? "DO " : "PLAN"} ${text}`);
    if (execute) action();
  };

  // The new Beta must be published, servable, and already on tine.page.
  const target = JSON.parse(gh("release", "view", tag, "--json", "tagName,isDraft,isPrerelease,targetCommitish"));
  if (target.isDraft || !target.isPrerelease) throw new Error(`${tag} is not a published prerelease`);
  const commit = ghApi(`repos/${repository}/commits/${tag}`, "--jq", ".sha").trim();
  const temporary = fs.mkdtempSync(path.join(os.tmpdir(), "tine-retire-beta-"));
  try {
    const bridgeFiles = ["latest.json", ...ZSYNC_ARCHES.map((arch) => zsyncAssetName(tag, arch))];
    gh("release", "download", tag, "--dir", temporary, ...bridgeFiles.flatMap((name) => ["--pattern", name]));
    const read = (name) => fs.readFileSync(path.join(temporary, name), "latin1");
    const manifest = JSON.parse(read("latest.json"));
    const zsync = Object.fromEntries(ZSYNC_ARCHES.map((arch) => [arch, read(zsyncAssetName(tag, arch))]));
    const problems = servingProblems({ repository, tag, manifest, zsync });
    if (problems.length) throw new Error(`${tag} cannot be the bridge target:\n  ${problems.join("\n  ")}`);
    const served = JSON.parse(fetchText(`${BETA_ENDPOINT}?retire=${Date.now()}`)).version;
    if (served !== tag.slice(1)) throw new Error(`tine.page serves ${served}, not ${tag.slice(1)}; dispatch pages.yml on master first`);
    console.log(`${tag} (${commit}) is published and tine.page serves it.`);

    // Where the `beta` release stands now.
    const legacy = spawnSync("gh", ["release", "view", BETA_TAG, "--json", "assets,name", "--repo", repository], { encoding: "utf8" });
    let current = null;
    if (legacy.status === 0) {
      gh("release", "download", BETA_TAG, "--dir", path.join(temporary, "legacy"), "--pattern", "latest.json");
      current = JSON.parse(fs.readFileSync(path.join(temporary, "legacy", "latest.json"), "utf8")).version;
    }
    if (current === tag.slice(1)) {
      console.log(`The ${BETA_TAG} release already bridges to ${tag}; verifying only.`);
    } else {
      if (!current || !betaVersion(`v${current}`)) throw new Error(`the ${BETA_TAG} release holds ${current ?? "nothing"}, not an older Beta`);
      const own = `v${current}`;
      const ownCommit = ghApi(`repos/${repository}/commits/${own}`, "--jq", ".sha").trim();
      const betaCommit = ghApi(`repos/${repository}/commits/${BETA_TAG}`, "--jq", ".sha").trim();
      if (ownCommit !== betaCommit) throw new Error(`${own} is ${ownCommit} but ${BETA_TAG} is ${betaCommit}; tag ${own} at the Beta's commit first`);
      step(`move the ${current} release from tag ${BETA_TAG} to its own tag ${own}`, () => gh("release", "edit", BETA_TAG, "--tag", own));
      step(`delete tag ${BETA_TAG} (${betaCommit}) so it can be re-created at ${commit}`, () => ghApi("-X", "DELETE", `repos/${repository}/git/refs/tags/${BETA_TAG}`));
      const notes = path.join(temporary, "notes.md");
      fs.writeFileSync(notes, `${bridgeNotes(repository, tag, releaseLayout(tag.slice(1)))}\n`);
      step(`create the frozen bridge release ${BETA_TAG} at ${commit} with ${bridgeFiles.join(", ")}`, () => {
        gh("release", "create", BETA_TAG, "--target", commit, "--prerelease", "--latest=false", "--title", `${IDENTITY.productName} update bridge`, "--notes-file", notes,
          ...bridgeFiles.map((name) => path.join(temporary, name)));
      });
      if (!execute) {
        console.log(`Dry run only. Bridge notes:\n\n${fs.readFileSync(notes, "utf8")}`);
        return;
      }
    }

    // Verify through the public URLs old installs actually use.
    const bridged = JSON.parse(fetchText(`${LEGACY_BETA_ENDPOINT}?retire=${Date.now()}`));
    const bridgedZsync = Object.fromEntries(ZSYNC_ARCHES.map((arch) => [arch,
      fetchText(`https://github.com/${repository}/releases/download/${BETA_TAG}/${zsyncAssetName(tag, arch)}`)]));
    const bridgeProblems = servingProblems({ repository, tag, manifest: bridged, zsync: bridgedZsync });
    for (const entry of Object.values(bridged.platforms ?? {})) {
      const status = spawnSync("curl", ["-fsIL", "-o", "/dev/null", "-w", "%{http_code}", entry.url], { encoding: "utf8" }).stdout;
      if (status !== "200") bridgeProblems.push(`${entry.url} answers ${status}`);
    }
    if (bridgeProblems.length) throw new Error(`the ${BETA_TAG} bridge is broken:\n  ${bridgeProblems.join("\n  ")}`);
    console.log(`Bridge OK: ${LEGACY_BETA_ENDPOINT} and both zsync files lead to ${tag}. Keep this release.`);
  } finally {
    fs.rmSync(temporary, { recursive: true, force: true });
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main();
