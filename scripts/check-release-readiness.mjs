#!/usr/bin/env node

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  changelogItems,
  normalizeItemText,
  releaseSection,
  validateDisposition,
  validateGuideDisposition,
} from "./release-readiness-lib.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const version = JSON.parse(fs.readFileSync(path.join(root, "src-tauri/tauri.conf.json"), "utf8")).version;
const packageJson = JSON.parse(fs.readFileSync(path.join(root, "package.json"), "utf8"));
const packageLock = JSON.parse(fs.readFileSync(path.join(root, "package-lock.json"), "utf8"));
const cargoToml = fs.readFileSync(path.join(root, "Cargo.toml"), "utf8");
const cargoLock = fs.readFileSync(path.join(root, "Cargo.lock"), "utf8");
const changelog = fs.readFileSync(path.join(root, "CHANGELOG.md"), "utf8");
const section = releaseSection(changelog, version);
const impactPath = path.join(root, `docs/releases/v${version}-impact.json`);
const regressionIndex = JSON.parse(fs.readFileSync(path.join(root, "tests/regressions/catalog.json"), "utf8"));
const catalogIds = new Set();
for (const inventory of regressionIndex.inventories ?? []) {
  const catalog = JSON.parse(fs.readFileSync(path.join(root, inventory.path), "utf8"));
  for (const entry of catalog.entries ?? []) catalogIds.add(entry.id);
}
const problems = [];

const workspaceVersion = /^version = "([^"]+)"$/m.exec(cargoToml)?.[1];
const releaseVersions = new Map([
  ["package.json", packageJson.version],
  ["package-lock.json", packageLock.version],
  ["package-lock.json root package", packageLock.packages?.[""]?.version],
  ["Cargo.toml workspace", workspaceVersion],
]);
for (const packageBlock of cargoLock.split("[[package]]")) {
  const name = /^name = "([^"]+)"$/m.exec(packageBlock)?.[1];
  if (name === "tine" || name === "tine-core") {
    releaseVersions.set(`Cargo.lock ${name}`, /^version = "([^"]+)"$/m.exec(packageBlock)?.[1]);
  }
}
for (const [source, found] of releaseVersions) {
  if (found !== version) problems.push(`${source} version ${found ?? "missing"} does not match ${version}`);
}

if (!section) problems.push(`CHANGELOG.md has no released section for ${version}`);
if (!fs.existsSync(impactPath)) problems.push(`missing docs/releases/v${version}-impact.json`);

// F-Droid fastlane changelogs: one file per Android versionCode, hard 500-char
// store limit (F-Droid reviewer request on the inclusion MR, 2026-07-15).
const versionCode = JSON.parse(fs.readFileSync(path.join(root, "src-tauri/tauri.conf.json"), "utf8"))
  .bundle?.android?.versionCode;
const changelogsDir = path.join(root, "fastlane/metadata/android/en-US/changelogs");
if (!Number.isInteger(versionCode)) problems.push("tauri.conf.json lacks bundle.android.versionCode");
else if (!fs.existsSync(path.join(changelogsDir, `${versionCode}.txt`))) {
  problems.push(`missing fastlane changelog changelogs/${versionCode}.txt for this release`);
}
for (const file of fs.existsSync(changelogsDir) ? fs.readdirSync(changelogsDir) : []) {
  const length = fs.readFileSync(path.join(changelogsDir, file), "utf8").length;
  if (length > 500) problems.push(`fastlane changelog ${file} is ${length} chars (F-Droid limit is 500)`);
}

if (section && fs.existsSync(impactPath)) {
  const impact = JSON.parse(fs.readFileSync(impactPath, "utf8"));
  if (impact.schemaVersion !== 2) problems.push("impact schemaVersion must be 2");
  if (impact.version !== version) problems.push(`impact version ${impact.version} does not match ${version}`);
  if (!/^v\d+\.\d+\.\d+$/.test(impact.baseTag ?? "")) problems.push("impact baseTag is invalid");
  if (!Array.isArray(impact.items)) problems.push("impact items must be an array");
  else {
    const expected = changelogItems(section).map((item) => `${item.section}\0${normalizeItemText(item.text)}`);
    const actual = impact.items.map((item) => `${item.section}\0${normalizeItemText(item.text ?? "")}`);
    for (const item of expected) if (!actual.includes(item)) problems.push(`impact missing changelog item: ${item.split("\0")[1]}`);
    for (const item of actual) if (!expected.includes(item)) problems.push(`impact contains non-changelog item: ${item.split("\0")[1]}`);
    for (const [index, item] of impact.items.entries()) {
      const owner = `impact item ${index + 1}`;
      if (typeof item.userVisible !== "boolean") problems.push(`${owner}: userVisible must be boolean`);
      if (!Array.isArray(item.regressions)) problems.push(`${owner}: regressions must be an array`);
      else for (const id of item.regressions) if (!catalogIds.has(id)) problems.push(`${owner}: unknown regression ${id}`);
      validateGuideDisposition(root, owner, item, problems);
      validateDisposition(`${owner} docs`, item.docs, problems);
      validateDisposition(`${owner} website`, item.website, problems);
      validateDisposition(`${owner} blog`, item.blog, problems);
      if ([item.guide, item.docs, item.website, item.blog].some((value) => value?.status === "consult")) {
        problems.push(`${owner}: unresolved consult disposition blocks release`);
      }
    }
  }
  // Audits are not a release gate (2026-10-06): the maintainer requests them by
  // kind before a major release. `auditableSourceFingerprint` stays available
  // for an attestation he asks for.
}

if (problems.length) {
  console.error(`Release readiness failed (${problems.length} problem(s)):`);
  for (const problem of problems) console.error(`  ${problem}`);
  process.exit(1);
}

console.log(`Release readiness OK: v${version}, ${catalogIds.size} catalog entries, ${path.basename(impactPath)}.`);
