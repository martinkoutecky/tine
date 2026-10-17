#!/usr/bin/env node
// Copy the current Beta update manifest onto tine.page (run by pages.yml).
//
// From 0.8.0-beta.3 on, Tine Beta reads its updates from the fixed address
// https://tine.page/updates/beta/latest.json, and Beta AppImages read
// https://tine.page/updates/beta/<arch>.AppImage.zsync. Each Beta is published
// to its own immutable release vX.Y.Z-beta.N; this script picks the newest one
// by version and copies its manifest and zsync files into the site, after
// checking that every download URL points into that same release.
//
// Releases before FIRST_FIXED_MANIFEST_BETA still point at the old moving
// `beta` tag; they are never served here. If no eligible release exists yet,
// nothing is written and the deploy proceeds (no client reads the address).

import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { IDENTITIES } from "./lib/app-identity.mjs";
import { releaseLayout } from "./release-layout.mjs";

export const FIRST_FIXED_MANIFEST_BETA = "0.8.0-beta.3";
export const ZSYNC_ARCHES = ["amd64", "aarch64"];

export function betaVersion(tag) {
  const match = /^v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)-beta\.([1-9]\d*)$/.exec(tag ?? "");
  return match ? match.slice(1).map(Number) : null;
}

function compare(a, b) {
  for (let i = 0; i < 4; i++) if (a[i] !== b[i]) return a[i] - b[i];
  return 0;
}

/** The newest published Beta release eligible for the fixed manifest, or null. */
export function selectBetaRelease(releases) {
  const floor = betaVersion(`v${FIRST_FIXED_MANIFEST_BETA}`);
  let best = null;
  for (const release of releases) {
    if (release.draft || !release.prerelease) continue;
    const version = betaVersion(release.tag_name);
    if (!version || compare(version, floor) < 0) continue;
    if (!best || compare(version, best.version) > 0) best = { release, version };
  }
  return best?.release ?? null;
}

export function zsyncAssetName(tag, arch) {
  return `Tine-Beta_${tag.slice(1)}_${arch}.AppImage.zsync`;
}

/**
 * Problems with a release's manifest and zsync files; empty when servable.
 * Every updater target this checkout's release layout knows for the Beta
 * identity must be present and name exactly its asset in this release. A Beta
 * may add targets before this branch learns them: those must still name a Beta
 * asset of this version in this release.
 */
export function servingProblems({ repository, tag, manifest, zsync, identity = IDENTITIES.experiment }) {
  const problems = [];
  const version = tag.slice(1);
  const base = `https://github.com/${repository}/releases/download/${tag}/`;
  const known = releaseLayout(version, identity).updaterPlatforms;
  const product = identity.productName.replace(/\s+/g, "-");
  if (manifest?.version !== version) problems.push(`latest.json version is ${manifest?.version}, expected ${version}`);
  const platforms = manifest?.platforms ?? {};
  for (const platform of Object.keys(known)) if (!platforms[platform]) problems.push(`latest.json is missing ${platform}`);
  for (const [platform, entry] of Object.entries(platforms)) {
    const asset = typeof entry?.url === "string" && entry.url.startsWith(base) ? entry.url.slice(base.length) : null;
    const expected = known[platform]?.[0];
    if (expected ? asset !== expected
      : !asset || asset.includes("/") || !(asset.startsWith(`${product}_${version}_`) || asset.startsWith(`${product}-${version}-`))) {
      problems.push(`latest.json ${platform} is ${entry?.url}, expected ${base}${expected ?? `${product}_${version}_…`}`);
    }
    if (typeof entry?.signature !== "string" || entry.signature.length === 0) problems.push(`latest.json ${platform} has no signature`);
  }
  for (const arch of ZSYNC_ARCHES) {
    const text = zsync?.[arch];
    if (typeof text !== "string") {
      problems.push(`${zsyncAssetName(tag, arch)} is missing`);
      continue;
    }
    const appimage = zsyncAssetName(tag, arch).slice(0, -".zsync".length);
    const url = /^URL: (.+)$/m.exec(text.slice(0, text.indexOf("\n\n")))?.[1];
    if (url !== `${base}${appimage}`) problems.push(`${zsyncAssetName(tag, arch)} URL is ${url}, expected ${base}${appimage}`);
  }
  return problems;
}

function main() {
  const repository = process.env.GITHUB_REPOSITORY;
  const site = path.resolve(process.argv[2] ?? "website");
  if (!repository) throw new Error("GITHUB_REPOSITORY is required");
  const gh = (...args) => execFileSync("gh", args, { encoding: "utf8", stdio: ["ignore", "pipe", "inherit"] });
  const releases = gh("api", "--paginate", `repos/${repository}/releases?per_page=100`,
    "--jq", ".[] | {tag_name, draft, prerelease} | tojson").split("\n").filter(Boolean).map((line) => JSON.parse(line));
  const release = selectBetaRelease(releases);
  if (!release) {
    console.log(`No Beta release at or after ${FIRST_FIXED_MANIFEST_BETA} yet; not serving a Beta manifest.`);
    return;
  }
  const tag = release.tag_name;
  const temporary = fs.mkdtempSync(path.join(os.tmpdir(), "tine-beta-manifest-"));
  try {
    const patterns = ["latest.json", ...ZSYNC_ARCHES.map((arch) => zsyncAssetName(tag, arch))];
    gh("release", "download", tag, "--repo", repository, "--dir", temporary, ...patterns.flatMap((name) => ["--pattern", name]));
    const read = (name) => (fs.existsSync(path.join(temporary, name)) ? fs.readFileSync(path.join(temporary, name), "latin1") : undefined);
    const manifest = JSON.parse(read("latest.json") ?? "null");
    const zsync = Object.fromEntries(ZSYNC_ARCHES.map((arch) => [arch, read(zsyncAssetName(tag, arch))]));
    const problems = servingProblems({ repository, tag, manifest, zsync });
    if (problems.length) throw new Error(`${tag} cannot be served as the Beta manifest:\n  ${problems.join("\n  ")}`);
    const out = path.join(site, "updates", "beta");
    fs.mkdirSync(out, { recursive: true });
    fs.copyFileSync(path.join(temporary, "latest.json"), path.join(out, "latest.json"));
    for (const arch of ZSYNC_ARCHES) fs.copyFileSync(path.join(temporary, zsyncAssetName(tag, arch)), path.join(out, `${arch}.AppImage.zsync`));
    console.log(`Serving ${tag} as the Beta manifest under ${path.relative(process.cwd(), out)}/`);
  } finally {
    fs.rmSync(temporary, { recursive: true, force: true });
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main();
