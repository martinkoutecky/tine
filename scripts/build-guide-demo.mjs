#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
// og (the beta channel): the public demo at website/demo is master's and is
// published from master only (Martin, 2026-10-03). og never regenerates or
// checks it; this script builds og's Guide site into the untracked
// target/guide/demo for og's own publish journeys
// (e2e-og-published-guide, check-publish-outline-geometry).
if (process.argv.includes("--check")) {
  console.error("build-guide-demo --check: website/demo follows master on og; there is nothing to check here.");
  process.exit(1);
}
const checkedIn = path.join(root, "target/guide/demo");
const check = false;
const temp = null;
const output = checkedIn;

// The checked-in public demo is a reproducible artifact, independent of the
// local build clock and the commit which happens to run this check.
const guideBuildEnv = { ...process.env, SOURCE_DATE_EPOCH: "1790640000", TINE_BUILD_COMMIT: "" };
const frontend = spawnSync("npx", ["--no-install", "vite", "build"],
  { cwd: root, stdio: "inherit", env: guideBuildEnv });
if (frontend.status !== 0) process.exit(frontend.status ?? 1);
if (!check) fs.rmSync(output, { recursive: true, force: true });
fs.mkdirSync(path.dirname(output), { recursive: true });

// Sheets are computed by the app's own TS evaluator, never by the Rust publisher
// (I-12): dump the Guide's sheet blocks, compute them with src/sheet/staticExport.ts
// under vite-node, then publish with the exports.
const scratch = fs.mkdtempSync(path.join(os.tmpdir(), "tine-guide-sheets-"));
const inputsFile = path.join(scratch, "inputs.json");
const exportsFile = path.join(scratch, "exports.json");
const cargoEnv = { ...guideBuildEnv, CARGO_INCREMENTAL: "0" };
const example = (args) => spawnSync(
  "cargo",
  ["run", "--quiet", "-p", "tine-store", "--example", "build-demo-site", "--", ...args],
  { cwd: root, stdio: "inherit", env: cargoEnv },
);
const dumped = example(["--dump-sheet-inputs", inputsFile]);
if (dumped.status !== 0) process.exit(dumped.status ?? 1);
const computed = spawnSync("npx", ["--no-install", "vite-node", "scripts/sheet-export-cli.ts", inputsFile, exportsFile],
  { cwd: root, stdio: "inherit", env: guideBuildEnv });
if (computed.status !== 0) process.exit(computed.status ?? 1);
const built = example([output, "--sheets", exportsFile]);
fs.rmSync(scratch, { recursive: true, force: true });
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
