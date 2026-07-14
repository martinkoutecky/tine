#!/usr/bin/env node

import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const uiCheck = spawnSync(process.execPath, [path.join(root, "scripts/check-ui-regression-catalog.mjs")], {
  cwd: root,
  encoding: "utf8",
});
process.stdout.write(uiCheck.stdout);
process.stderr.write(uiCheck.stderr);
if (uiCheck.status !== 0) process.exit(uiCheck.status ?? 1);

const taoTitlebarCheck = spawnSync(process.execPath, [path.join(root, "scripts/check-tao-native-titlebar.mjs")], {
  cwd: root,
  encoding: "utf8",
});
process.stdout.write(taoTitlebarCheck.stdout);
process.stderr.write(taoTitlebarCheck.stderr);
if (taoTitlebarCheck.status !== 0) process.exit(taoTitlebarCheck.status ?? 1);

const index = JSON.parse(fs.readFileSync(path.join(root, "tests/regressions/catalog.json"), "utf8"));
const problems = [];
const inventoryIds = new Set();
const allowedStatuses = new Set(["reported", "reproduced", "fixing", "covered", "released", "closed", "exempt"]);

if (index.schemaVersion !== 1 || !Array.isArray(index.inventories) || index.inventories.length < 2) {
  problems.push("regression index must have schemaVersion 1 and at least two inventories");
} else {
  for (const inventory of index.inventories) {
    if (!inventory.id || inventoryIds.has(inventory.id)) problems.push(`duplicate or missing inventory id ${inventory.id}`);
    inventoryIds.add(inventory.id);
    const inventoryPath = path.join(root, inventory.path ?? "");
    if (!fs.existsSync(inventoryPath)) {
      problems.push(`${inventory.id}: missing ${inventory.path}`);
      continue;
    }
    const data = JSON.parse(fs.readFileSync(inventoryPath, "utf8"));
    if (data.schemaVersion !== 1 || !Array.isArray(data.entries)) {
      problems.push(`${inventory.id}: catalog must have schemaVersion 1 and entries`);
      continue;
    }
    for (const entry of data.entries) {
      if (!entry.id?.startsWith(inventory.idPrefix)) problems.push(`${inventory.id}: invalid entry id ${entry.id}`);
      if (!allowedStatuses.has(entry.status)) problems.push(`${entry.id}: invalid status ${entry.status}`);
      // A single issue can contain a later, separately testable regression after
      // an earlier fix shipped. Catalog IDs own behaviors; issue numbers only
      // preserve public provenance and therefore need not be unique.
      for (const issue of entry.sources?.issues ?? []) {
        if (!Number.isInteger(issue) || issue < 1) problems.push(`${entry.id}: invalid issue ${issue}`);
      }
      for (const test of entry.coverage?.tests ?? []) {
        const file = test.split("#", 1)[0];
        if (!fs.existsSync(path.join(root, file))) problems.push(`${entry.id}: missing test file ${file}`);
      }
    }
  }
}

if (problems.length) {
  console.error(`Regression catalog failed (${problems.length} problem(s)):`);
  for (const problem of problems) console.error(`  ${problem}`);
  process.exit(1);
}
console.log(`Regression catalog index OK: ${index.inventories.length} inventories.`);
