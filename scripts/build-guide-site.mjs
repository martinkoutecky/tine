#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { filesUnder, validateGuideSiteLinks } from "./guide-site-validator.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const checkedIn = path.join(root, "website/guide");
const check = process.argv.includes("--check");
const temp = check ? fs.mkdtempSync(path.join(os.tmpdir(), "tine-guide-site-")) : null;
const output = check ? path.join(temp, "guide") : checkedIn;

const built = spawnSync(
  "cargo",
  ["run", "--quiet", "-p", "tine-core", "--example", "build-guide-site", "--", output],
  { cwd: root, stdio: "inherit" },
);
if (built.status !== 0) process.exit(built.status ?? 1);

try {
  validateGuideSiteLinks(output);
  if (check) {
    const expected = filesUnder(checkedIn);
    const actual = filesUnder(output);
    const names = new Set([...expected, ...actual]);
    const stale = [];
    for (const relative of [...names].sort()) {
      const left = path.join(checkedIn, relative);
      const right = path.join(output, relative);
      if (!fs.existsSync(left)) stale.push(`missing from website/guide: ${relative}`);
      else if (!fs.existsSync(right)) stale.push(`extra in website/guide: ${relative}`);
      else if (!fs.readFileSync(left).equals(fs.readFileSync(right))) stale.push(`content differs: ${relative}`);
    }
    if (stale.length) throw new Error(`website/guide is stale; run npm run docs:build\n${stale.join("\n")}`);
    console.log(`Guide OK: ${actual.length} generated files match website/guide`);
  } else {
    console.log(`Guide rebuilt: ${filesUnder(output).length} files`);
  }
} finally {
  if (temp) fs.rmSync(temp, { recursive: true, force: true });
}
