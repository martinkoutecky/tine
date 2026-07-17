#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { buildInputState } from "./build-e2e-inputs.mjs";

// Keeps every native E2E candidate tied to the source state that built it.

function usage() {
  throw new Error("usage: node scripts/build-e2e-receipt.mjs <before|after> --snapshot <path> [--app <path>] [--receipt <path>] [--dist <path>]");
}

function parseArgs(argv) {
  const [phase, ...rest] = argv;
  if (phase !== "before" && phase !== "after") usage();
  const options = {};
  for (let index = 0; index < rest.length; index += 2) {
    const key = rest[index];
    const value = rest[index + 1];
    if (!key?.startsWith("--") || value === undefined || !["--snapshot", "--app", "--receipt", "--dist"].includes(key)) usage();
    options[key.slice(2)] = value;
  }
  if (!options.snapshot || (phase === "after" && !options.app)) usage();
  return { phase, options };
}

function git(root, args, encoding = "utf8") {
  const result = spawnSync("git", args, { cwd: root, encoding, maxBuffer: 32 * 1024 * 1024 });
  if (result.status !== 0) {
    throw result.error || new Error(`git ${args.join(" ")} failed: ${String(result.stderr || "").trim()}`);
  }
  return result.stdout;
}

function repoRoot() {
  const root = git(process.cwd(), ["rev-parse", "--show-toplevel"]).trim();
  if (!root) throw new Error("could not determine the Git worktree for the build receipt");
  return path.resolve(root);
}

function snapshot(root) {
  const state = buildInputState(root);
  return {
    schemaVersion: 1,
    kind: "tine-e2e-build-input-snapshot",
    repositoryRoot: root,
    sourceRevision: git(root, ["rev-parse", "HEAD"]).trim(),
    buildInputDigest: state.digest,
    buildInputsDirty: state.dirty,
    buildInputChanges: state.changes,
    capturedAt: new Date().toISOString(),
  };
}

function writeJson(file, value) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, `${JSON.stringify(value, null, 2)}\n`);
}

function readSnapshot(file) {
  let value;
  try {
    value = JSON.parse(fs.readFileSync(file, "utf8"));
  } catch (error) {
    throw new Error(`could not read build-input snapshot ${file}: ${error.message}`);
  }
  if (!value || Array.isArray(value) || value.schemaVersion !== 1 || value.kind !== "tine-e2e-build-input-snapshot"
    || typeof value.repositoryRoot !== "string" || typeof value.sourceRevision !== "string"
    || typeof value.buildInputDigest !== "string" || typeof value.buildInputsDirty !== "boolean"
    || !Array.isArray(value.buildInputChanges)) {
    throw new Error(`invalid build-input snapshot ${file}`);
  }
  return value;
}

function frontendAsset(dist) {
  const index = path.join(dist, "index.html");
  if (!fs.existsSync(index)) throw new Error(`dist/index.html is missing at ${index}`);
  const asset = fs.readFileSync(index, "utf8").match(/[A-Za-z0-9_]+-[A-Za-z0-9_-]+\.(?:js|css)/)?.[0];
  if (!asset) throw new Error(`could not identify a hashed frontend asset in ${index}`);
  return asset;
}

function sameFileStat(before, after) {
  return before.dev === after.dev && before.ino === after.ino && before.size === after.size && before.mtimeMs === after.mtimeMs;
}

function createReceipt(root, before, app, dist) {
  const afterRevision = git(root, ["rev-parse", "HEAD"]).trim();
  if (afterRevision !== before.sourceRevision) {
    throw new Error(`refusing receipt: HEAD changed while building (${before.sourceRevision} -> ${afterRevision})`);
  }
  const afterState = buildInputState(root);
  if (afterState.digest !== before.buildInputDigest) {
    throw new Error("refusing receipt: build-input state changed while building");
  }
  const beforeStat = fs.statSync(app);
  const bytes = fs.readFileSync(app);
  const asset = frontendAsset(dist);
  if (!bytes.includes(Buffer.from(asset))) {
    throw new Error(`binary does not embed current production frontend ${asset}`);
  }
  const afterStat = fs.statSync(app);
  if (!sameFileStat(beforeStat, afterStat)) throw new Error("refusing receipt: app binary changed while being verified");
  return {
    schemaVersion: 1,
    sourceRevision: before.sourceRevision,
    builtAt: new Date().toISOString(),
    frontendAsset: asset,
    appSha256: crypto.createHash("sha256").update(bytes).digest("hex"),
    buildInputDigest: before.buildInputDigest,
    buildInputsDirty: before.buildInputsDirty,
    buildInputChanges: before.buildInputChanges,
  };
}

const { phase, options } = parseArgs(process.argv.slice(2));
const root = repoRoot();
const snapshotPath = path.resolve(options.snapshot);

if (phase === "before") {
  writeJson(snapshotPath, snapshot(root));
  console.log(`build-input snapshot → ${snapshotPath}`);
} else {
  const before = readSnapshot(snapshotPath);
  if (path.resolve(before.repositoryRoot) !== root) throw new Error(`build-input snapshot belongs to ${before.repositoryRoot}, not ${root}`);
  const app = path.resolve(options.app);
  if (!fs.existsSync(app)) throw new Error(`app binary not found: ${app}`);
  const receiptPath = options.receipt ? path.resolve(options.receipt) : `${app}.build.json`;
  const receipt = createReceipt(root, before, app, path.resolve(options.dist || path.join(root, "dist")));
  writeJson(receiptPath, receipt);
  console.log(`build receipt → ${receiptPath}`);
}
