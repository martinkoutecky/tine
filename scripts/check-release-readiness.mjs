#!/usr/bin/env node

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  auditableSourceFingerprint,
  changelogItems,
  normalizeItemText,
  releaseSection,
  validateDisposition,
} from "./release-readiness-lib.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const version = JSON.parse(fs.readFileSync(path.join(root, "src-tauri/tauri.conf.json"), "utf8")).version;
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

if (!section) problems.push(`CHANGELOG.md has no released section for ${version}`);
if (!fs.existsSync(impactPath)) problems.push(`missing docs/releases/v${version}-impact.json`);

if (section && fs.existsSync(impactPath)) {
  const impact = JSON.parse(fs.readFileSync(impactPath, "utf8"));
  if (impact.schemaVersion !== 1) problems.push("impact schemaVersion must be 1");
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
      validateDisposition(`${owner} docs`, item.docs, problems);
      validateDisposition(`${owner} website`, item.website, problems);
      validateDisposition(`${owner} blog`, item.blog, problems);
      if ([item.docs, item.website, item.blog].some((value) => value?.status === "consult")) {
        problems.push(`${owner}: unresolved consult disposition blocks release`);
      }
    }
  }
  if (version.startsWith("0.") && version.endsWith(".0")) {
    if (!impact.focusedAudit || typeof impact.focusedAudit.required !== "boolean" || impact.focusedAudit.reason?.length < 10) {
      problems.push("minor release impact needs a focusedAudit decision and reason");
    }
    const auditPath = path.join(root, `docs/releases/v${version}-audit-attestation.json`);
    if (!fs.existsSync(auditPath)) problems.push(`missing minor audit attestation ${path.basename(auditPath)}`);
    else {
      const attestation = JSON.parse(fs.readFileSync(auditPath, "utf8"));
      if (attestation.version !== version) problems.push("audit attestation version mismatch");
      if (attestation.sourceFingerprint !== auditableSourceFingerprint(root)) problems.push("audit attestation is stale for the current source tree");
      const required = new Set(["data-safety-security-privacy", "behavior-compatibility", "performance-resources"]);
      if (impact.focusedAudit.required) required.add("focused-change-cluster");
      for (const id of required) {
        const area = attestation.areas?.find((value) => value.id === id);
        if (!area) problems.push(`audit attestation missing ${id}`);
        else if (area.critical !== 0 || area.high !== 0 || !/^[0-9a-f]{64}$/.test(area.reportSha256 ?? "")) {
          problems.push(`audit area ${id} is not clean or lacks report digest`);
        }
      }
    }
    const redditPath = path.join(root, `docs/releases/v${version}-reddit.json`);
    if (!fs.existsSync(redditPath)) problems.push(`missing minor Reddit/blog evidence ${path.basename(redditPath)}`);
    else {
      const reddit = JSON.parse(fs.readFileSync(redditPath, "utf8"));
      if (reddit.version !== version || reddit.schemaVersion !== 1) problems.push("Reddit/blog evidence version or schema mismatch");
      if (reddit.author !== "al-Quaknaa") problems.push("Reddit/blog evidence has unexpected author");
      if (!Array.isArray(reddit.unprocessed) || reddit.unprocessed.length) problems.push("Reddit/blog evidence has unprocessed author posts");
      if (!Array.isArray(reddit.failedThreads) || reddit.failedThreads.length) problems.push("Reddit/blog evidence has discussion refresh failures");
      if (!Array.isArray(reddit.threadSnapshots) || reddit.threadSnapshots.some((item) => !/^[0-9a-f]{64}$/.test(item.sha256 ?? ""))) {
        problems.push("Reddit/blog evidence lacks valid discussion snapshots");
      }
    }
  }
}

if (problems.length) {
  console.error(`Release readiness failed (${problems.length} problem(s)):`);
  for (const problem of problems) console.error(`  ${problem}`);
  process.exit(1);
}

console.log(`Release readiness OK: v${version}, ${catalogIds.size} catalog entries, ${path.basename(impactPath)}.`);
