#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const checkedIn = path.join(root, "website/demo");
const check = process.argv.includes("--check");
const temp = check ? fs.mkdtempSync(path.join(os.tmpdir(), "tine-guide-demo-")) : null;
const output = check ? path.join(temp, "demo") : checkedIn;

const built = spawnSync(
  "cargo",
  ["run", "--quiet", "-p", "tine-core", "--example", "build-demo-site", "--", output],
  { cwd: root, stdio: "inherit" },
);
if (built.status !== 0) process.exit(built.status ?? 1);

function filesUnder(dir, prefix = "") {
  const files = [];
  for (const entry of fs.readdirSync(dir, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name))) {
    const relative = path.join(prefix, entry.name);
    if (entry.isDirectory()) files.push(...filesUnder(path.join(dir, entry.name), relative));
    else files.push(relative);
  }
  return files;
}

function validateLinks(dir) {
  const failures = [];
  for (const relative of filesUnder(dir).filter((file) => file.endsWith(".html"))) {
    const html = fs.readFileSync(path.join(dir, relative), "utf8");
    for (const match of html.matchAll(/<[^>]+?(?:href|src)="([^"]+)"[^>]*>/g)) {
      const tag = match[0];
      const target = match[1];
      if (/^(?:[a-z]+:|#|\/\/)/i.test(target)) continue;
      const withoutFragment = target.split("#", 1)[0].split("?", 1)[0];
      if (!withoutFragment) continue;
      const resolved = path.resolve(path.dirname(path.join(dir, relative)), decodeURIComponent(withoutFragment));
      // Deliberate empty-page refs are useful in the editable graph (click to
      // create) but have no generated static page. Other links and every asset
      // must resolve inside the generated site.
      if (/class="[^"]*\b(?:ref|tag)\b/.test(tag) && !fs.existsSync(resolved)) continue;
      if (!resolved.startsWith(path.resolve(dir) + path.sep) || !fs.existsSync(resolved)) {
        failures.push(`${relative}: missing local target ${target}`);
      }
    }
  }
  if (failures.length) throw new Error(`Guide/demo link validation failed:\n${failures.join("\n")}`);
}

try {
  validateLinks(output);
  if (check) {
    const expected = filesUnder(checkedIn);
    const actual = filesUnder(output);
    const names = new Set([...expected, ...actual]);
    const stale = [];
    for (const relative of [...names].sort()) {
      const left = path.join(checkedIn, relative);
      const right = path.join(output, relative);
      if (!fs.existsSync(left)) stale.push(`missing from website/demo: ${relative}`);
      else if (!fs.existsSync(right)) stale.push(`extra in website/demo: ${relative}`);
      else if (!fs.readFileSync(left).equals(fs.readFileSync(right))) stale.push(`content differs: ${relative}`);
    }
    if (stale.length) throw new Error(`website/demo is stale; run npm run docs:build\n${stale.join("\n")}`);
    console.log(`Guide/demo OK: ${actual.length} generated files match website/demo`);
  } else {
    console.log(`Guide/demo rebuilt: ${filesUnder(output).length} files`);
  }
} finally {
  if (temp) fs.rmSync(temp, { recursive: true, force: true });
}
