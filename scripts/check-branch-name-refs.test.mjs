import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { checkFiles, historicalFile, legacyBranchReferences } from "./check-branch-name-refs.mjs";

test("rejects executable old branch references, including workflow lists", () => {
  for (const text of ["origin/og", "refs/heads/og", "--base og", "--base='og'", "--line og",
    "default: og", 'default: "og"', "branches: [master, og]", "branches: ['og']",
    "branches:\n  - master\n  - og", 'branches-ignore:\n  - "og"',
    "branches: [\n  master,\n  og\n]", "branches:\n  # development\n  - master\n\n  - og",
    "git fetch -q origin og", "git switch og", 'rtk_git(repo, "fetch", "origin", "og")',
    '["--base", "og"]', 'prepare.add_argument("--base", default="og")',
    'LINE_BRANCHES = ("og", "master")', 'target: str = "og"', 'if target != "og":']) {
    assert.ok(legacyBranchReferences(text).length, text);
  }
  assert.deepEqual(legacyBranchReferences("name: workflow\n  default: og").map(hit => hit.line), [2]);
});

test("preserves identity, parity, historical branch prefixes and new branch", () => {
  for (const text of ["origin/develop", "origin/og-topic", "refs/heads/og-topic", "--base og-topic",
    "default: og-topic", "branches: [og-topic]", "branches: [tine-og, catalog]", "git fetch origin og-topic", "tine-og", "batch/og-topic",
    "og/PARITY-CAMPAIGN.md", "OG-RULES.md", 'writer: "og"', "og-parity-references"])
    assert.deepEqual(legacyBranchReferences(text), [], text);
});

test("history allowlist does not exempt live workflows, scripts or normative docs", () => {
  for (const file of ["CHANGELOG.md", "docs/adr/0001.md", "docs/releases/v0.8.0.json",
    "docs/notes/2026-10-26.md", "RECEIPT-lane.md", "tests/ui-regressions/catalog.json"])
    assert.equal(historicalFile(file), true, file);
  for (const file of [".github/workflows/release.yml", "scripts/deploy.sh", "docs/VERSIONING.md"])
    assert.equal(historicalFile(file), false, file);
});

test("scan fails before a branch edit and passes after it, while preserving history", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "tine-branch-guard-"));
  try {
    fs.writeFileSync(path.join(root, "workflow.yml"), "default: og\n");
    fs.writeFileSync(path.join(root, "CHANGELOG.md"), "origin/og\n");
    assert.equal(checkFiles(root, ["workflow.yml", "CHANGELOG.md"]).length, 1);
    fs.writeFileSync(path.join(root, "workflow.yml"), "default: develop\n");
    assert.deepEqual(checkFiles(root, ["workflow.yml", "CHANGELOG.md"]), []);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});
