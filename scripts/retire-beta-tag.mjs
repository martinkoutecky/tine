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
import { IDENTITIES } from "./lib/app-identity.mjs";
// The bridge only ever serves Tine Beta installs, whichever identity this tree ships.
const BETA = IDENTITIES.experiment;
import { releaseLayout } from "./release-layout.mjs";
import { BETA_ENDPOINT, BETA_TAG, LEGACY_BETA_ENDPOINT } from "./release-policy.mjs";

/** The direct downloads the bridge page offers: [label, asset, url]. */
export function bridgeLinks(repository, tag, layout) {
  const base = `https://github.com/${repository}/releases/download/${tag}/`;
  const find = (suffix) => layout.platformAssets.find((name) => name.endsWith(suffix));
  return [
    ["macOS (Apple Silicon and Intel)", find("_universal.dmg")],
    ["Windows x64 installer", find("_x64-setup.exe")],
    ["Windows ARM64 installer", find("_arm64-setup.exe")],
    ["Windows 32-bit installer", find("_x86-setup.exe")],
    ["Linux x64 AppImage", find("_amd64.AppImage")],
    ["Linux ARM64 AppImage", find("_aarch64.AppImage")],
  ].filter(([, asset]) => asset).map(([label, asset]) => [label, asset, `${base}${asset}`]);
}

export function bridgeNotes(repository, tag, layout) {
  return [
    `This release only helps older ${BETA.productName} installs find their next update. It contains no app.`,
    ``,
    `**Get the current Beta from [${tag}](https://github.com/${repository}/releases/tag/${tag})**, or download it directly:`,
    ``,
    ...bridgeLinks(repository, tag, layout).map(([label, asset, url]) => `- ${label}: [${asset}](${url})`),
    ``,
    `Install it over your current ${BETA.productName} and reopen it. From ${tag} on, ${BETA.productName} updates itself from each Beta's own release, so you will not land on this page again.`,
    ``,
    `(Betas before ${tag} read the update information in this release. It stays here, pointing at ${tag}, so that any of them can still update.)`,
  ].join("\n");
}

function compare(a, b) {
  for (let i = 0; i < 4; i++) if (a[i] !== b[i]) return a[i] - b[i];
  return 0;
}

/** Download URLs of a manifest, its zsync files and `extra` that do not answer 200. */
function unreachable(manifest, zsync, extra = []) {
  const urls = [
    ...Object.values(manifest.platforms ?? {}).map((entry) => entry.url),
    ...Object.values(zsync).map((text) => /^URL: (.+)$/m.exec(text.slice(0, text.indexOf("\n\n")))?.[1]),
    ...extra,
  ];
  return [...new Set(urls)].flatMap((url) => {
    const status = spawnSync("curl", ["-fsIL", "--max-time", "60", "-o", "/dev/null", "-w", "%{http_code}", url], { encoding: "utf8" }).stdout;
    return status === "200" ? [] : [`${url} answers ${status || "nothing"}`];
  });
}

function main() {
  const [tag, flag] = process.argv.slice(2);
  const execute = flag === "--execute";
  const repository = process.env.GITHUB_REPOSITORY ?? "martinkoutecky/tine";
  const version = betaVersion(tag);
  const floor = betaVersion(`v${FIRST_FIXED_MANIFEST_BETA}`);
  if (!version || compare(version, floor) < 0) {
    throw new Error(`usage: retire-beta-tag.mjs v<X.Y.Z-beta.N ≥ ${FIRST_FIXED_MANIFEST_BETA}> [--execute]`);
  }
  const gh = (...args) => execFileSync("gh", [...args, "--repo", repository], { encoding: "utf8", maxBuffer: 512 * 1024 * 1024, stdio: ["ignore", "pipe", "inherit"] });
  const ghApi = (...args) => execFileSync("gh", ["api", ...args], { encoding: "utf8", maxBuffer: 512 * 1024 * 1024, stdio: ["ignore", "pipe", "inherit"] });
  const fetchText = (url) => execFileSync("curl", ["-fsSL", "--max-time", "60", url], { encoding: "latin1" });
  // Reads that may find nothing: only GitHub's own "not there" answer means
  // absent; any other failure (network, auth, 5xx) stops the run.
  const lookup = (args, absent) => {
    const result = spawnSync("gh", args, { encoding: "utf8" });
    if (result.status === 0) return result.stdout;
    if (absent.test(result.stderr)) return null;
    throw new Error(`gh ${args.join(" ")} failed:\n${result.stderr}`);
  };
  const commitOf = (ref) => lookup(["api", `repos/${repository}/commits/${ref}`, "--jq", ".sha"], /HTTP 404|HTTP 422/)?.trim() ?? null;
  const step = (text, action) => {
    console.log(`${execute ? "DO " : "PLAN"} ${text}`);
    if (execute) action();
  };

  // Preflight, with no mutation: the new Beta is published, complete, its
  // downloads (updater, zsync and the bridge page's links) answer, and
  // tine.page serves it (or a newer Beta, on a rerun).
  const target = JSON.parse(gh("release", "view", tag, "--json", "tagName,isDraft,isPrerelease"));
  if (target.isDraft || !target.isPrerelease) throw new Error(`${tag} is not a published prerelease`);
  const commit = commitOf(tag);
  if (!commit) throw new Error(`tag ${tag} does not exist`);
  const layout = releaseLayout(tag.slice(1), BETA);
  const temporary = fs.mkdtempSync(path.join(os.tmpdir(), "tine-retire-beta-"));
  try {
    const bridgeFiles = ["latest.json", ...ZSYNC_ARCHES.map((arch) => zsyncAssetName(tag, arch))];
    gh("release", "download", tag, "--dir", temporary, ...bridgeFiles.flatMap((name) => ["--pattern", name]));
    const read = (name) => fs.readFileSync(path.join(temporary, name), "latin1");
    const manifest = JSON.parse(read("latest.json"));
    const zsync = Object.fromEntries(ZSYNC_ARCHES.map((arch) => [arch, read(zsyncAssetName(tag, arch))]));
    const links = bridgeLinks(repository, tag, layout).map(([, , url]) => url);
    const problems = servingProblems({ repository, tag, manifest, zsync });
    problems.push(...unreachable(manifest, zsync, links));
    if (problems.length) throw new Error(`${tag} cannot be the bridge target:\n  ${problems.join("\n  ")}`);
    const served = betaVersion(`v${JSON.parse(fetchText(`${BETA_ENDPOINT}?retire=${Date.now()}`)).version}`);
    if (!served || compare(served, version) < 0) throw new Error(`tine.page does not serve ${tag} or later yet; dispatch pages.yml on master first`);
    console.log(`${tag} (${commit}) is published, complete, reachable, and tine.page serves it or a later Beta.`);

    // Each step below is resumable: a rerun after any failure finds the state
    // it left and continues from there.
    const bridgeTitle = `${BETA.productName} update bridge`;
    // The release at `beta`, from the REST API so that each answer keeps its
    // status: `gh release view` can report "release not found" for an outage.
    // A published release answers the tag endpoint; only after its 404 does a
    // full authenticated listing decide whether a draft holds the tag.
    const shape = (found) => ({
      name: found.name,
      isDraft: found.draft,
      targetCommitish: found.target_commitish,
      assets: found.assets.map((asset) => ({ name: asset.name, size: asset.size, state: asset.state, apiUrl: asset.url })),
    });
    const view = () => {
      const published = lookup(["api", `repos/${repository}/releases/tags/${BETA_TAG}`], /HTTP 404/);
      if (published !== null) return shape(JSON.parse(published));
      const drafts = JSON.parse(ghApi("--paginate", "--slurp", `repos/${repository}/releases?per_page=100`))
        .flat().filter((found) => found.draft && found.tag_name === BETA_TAG);
      if (drafts.length > 1) throw new Error(`${drafts.length} draft releases name tag ${BETA_TAG}; delete the extra drafts first`);
      return drafts.length ? shape(drafts[0]) : null;
    };
    let release = view();
    if (release && release.name !== bridgeTitle) {
      // (1) An older Beta's release still holds `beta`: give it its own tag.
      gh("release", "download", BETA_TAG, "--dir", path.join(temporary, "legacy"), "--pattern", "latest.json");
      const current = JSON.parse(fs.readFileSync(path.join(temporary, "legacy", "latest.json"), "utf8")).version;
      const currentVersion = betaVersion(`v${current}`);
      if (!currentVersion || compare(currentVersion, version) >= 0) throw new Error(`the ${BETA_TAG} release holds ${current}, not a Beta older than ${tag}`);
      const own = `v${current}`;
      const ownCommit = commitOf(own);
      const betaCommit = commitOf(BETA_TAG);
      if (!ownCommit || ownCommit !== betaCommit) throw new Error(`${own} is ${ownCommit ?? "missing"} but ${BETA_TAG} is ${betaCommit}; tag ${own} at the Beta's commit first`);
      step(`move the ${current} release from tag ${BETA_TAG} to its own tag ${own}`, () => gh("release", "edit", BETA_TAG, "--tag", own));
      release = execute ? view() : null;
      if (release) throw new Error(`a release is still at ${BETA_TAG} after moving ${current} away`);
    }
    if (!release) {
      // (2) Re-create `beta` as a draft bridge. A release created on an
      // existing tag keeps that tag's commit, so the tag goes first.
      const betaCommit = commitOf(BETA_TAG);
      if (betaCommit && betaCommit !== commit) {
        step(`delete tag ${BETA_TAG} (${betaCommit}) so it can be re-created at ${commit}`, () => ghApi("-X", "DELETE", `repos/${repository}/git/refs/tags/${BETA_TAG}`));
      }
      const notes = path.join(temporary, "notes.md");
      fs.writeFileSync(notes, `${bridgeNotes(repository, tag, layout)}\n`);
      step(`create the draft bridge release ${BETA_TAG} at ${commit}`, () => {
        gh("release", "create", BETA_TAG, "--draft", "--target", commit, "--prerelease", "--latest=false", "--title", bridgeTitle, "--notes-file", notes);
      });
      if (!execute) console.log(`Bridge notes:\n\n${fs.readFileSync(notes, "utf8")}`);
      release = execute ? view() : { name: bridgeTitle, isDraft: true, targetCommitish: commit, assets: [] };
      if (!release) throw new Error(`the draft bridge release ${BETA_TAG} was not created`);
    }
    // (3) Complete the bridge's files, then publish it at the new Beta's commit.
    const unexpected = release.assets.map((asset) => asset.name).filter((name) => !bridgeFiles.includes(name));
    if (unexpected.length) throw new Error(`the ${BETA_TAG} bridge holds unexpected files: ${unexpected.join(", ")}`);
    const held = path.join(temporary, "bridge");
    const intact = (name) => {
      const asset = release.assets.find((candidate) => candidate.name === name);
      if (asset?.state !== "uploaded") return false;
      gh("release", "download", BETA_TAG, "--dir", held, "--pattern", name, "--clobber");
      const bytes = fs.readFileSync(path.join(held, name), "latin1");
      if (bytes === read(name)) return true;
      if (name === "latest.json" && !release.isDraft) {
        let bridged;
        try { bridged = JSON.parse(bytes).version; } catch { bridged = undefined; }
        if (bridged) throw new Error(`the ${BETA_TAG} bridge is frozen at ${bridged}, not ${tag.slice(1)}; leave it`);
      }
      return false;
    };
    const redo = bridgeFiles.filter((name) => !intact(name));
    for (const name of redo) {
      const asset = release.assets.find((candidate) => candidate.name === name);
      if (asset) step(`delete the incomplete bridge file ${name} (${asset.state}, ${asset.size} bytes)`, () => ghApi("-X", "DELETE", new URL(asset.apiUrl).pathname.slice(1)));
    }
    if (redo.length) {
      step(`upload ${redo.join(", ")} to the ${BETA_TAG} bridge`, () => gh("release", "upload", BETA_TAG, "--clobber", ...redo.map((name) => path.join(temporary, name))));
    }
    if (release.isDraft) {
      if (release.targetCommitish !== commit) {
        step(`point the draft bridge at ${commit}`, () => gh("release", "edit", BETA_TAG, "--target", commit));
      }
      const tagged = commitOf(BETA_TAG);
      if (tagged && tagged !== commit) {
        step(`delete tag ${BETA_TAG} (${tagged}) so publishing creates it at ${commit}`, () => ghApi("-X", "DELETE", `repos/${repository}/git/refs/tags/${BETA_TAG}`));
      }
      step(`publish the ${BETA_TAG} bridge (prerelease, never latest)`, () => gh("release", "edit", BETA_TAG, "--draft=false", "--prerelease", "--latest=false"));
    }
    if (!execute) {
      console.log("Dry run only: nothing was changed.");
      return;
    }

    // Verify through the public URLs old installs actually use, and the tag.
    const tagged = commitOf(BETA_TAG);
    if (tagged !== commit) throw new Error(`tag ${BETA_TAG} is at ${tagged ?? "nothing"}, not ${commit}`);
    const bridged = JSON.parse(fetchText(`${LEGACY_BETA_ENDPOINT}?retire=${Date.now()}`));
    const bridgedZsync = Object.fromEntries(ZSYNC_ARCHES.map((arch) => [arch,
      fetchText(`https://github.com/${repository}/releases/download/${BETA_TAG}/${zsyncAssetName(tag, arch)}`)]));
    const bridgeProblems = [...servingProblems({ repository, tag, manifest: bridged, zsync: bridgedZsync }), ...unreachable(bridged, bridgedZsync, links)];
    if (bridgeProblems.length) throw new Error(`the ${BETA_TAG} bridge is broken:\n  ${bridgeProblems.join("\n  ")}`);
    console.log(`Bridge OK: ${LEGACY_BETA_ENDPOINT} and both zsync files lead to ${tag}, tag ${BETA_TAG} is ${commit}. Keep this release.`);
  } finally {
    fs.rmSync(temporary, { recursive: true, force: true });
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main();
