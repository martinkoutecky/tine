#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { buildInputState } from "./build-e2e-inputs.mjs";

// Keeps every native E2E candidate tied to the source state that built it.

function usage() {
  throw new Error("usage: node scripts/build-e2e-receipt.mjs <before|after> --snapshot <path> [--tauri-manifest-normalization] [--app <path>] [--receipt <path>] [--dist <path>]");
}

function parseArgs(argv) {
  const [phase, ...rest] = argv;
  if (phase !== "before" && phase !== "after") usage();
  const options = {};
  for (let index = 0; index < rest.length;) {
    const key = rest[index];
    if (key === "--tauri-manifest-normalization") {
      if (phase !== "before" || options.tauriManifestNormalization) usage();
      options.tauriManifestNormalization = true;
      index += 1;
      continue;
    }
    const value = rest[index + 1];
    if (!key?.startsWith("--") || value === undefined || !["--snapshot", "--app", "--receipt", "--dist"].includes(key)) usage();
    options[key.slice(2)] = value;
    index += 2;
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

function sha256(value) {
  return crypto.createHash("sha256").update(value).digest("hex");
}

function encoded(value) {
  return value.toString("base64");
}

function decoded(value, label) {
  if (typeof value !== "string") throw new Error(`invalid ${label}`);
  const bytes = Buffer.from(value, "base64");
  if (bytes.toString("base64") !== value) throw new Error(`invalid ${label}`);
  return bytes;
}

function tauriManifestPath(root) {
  return path.join(root, "src-tauri", "Cargo.toml");
}

// `tauri inspect wix-upgrade-code` resolves the Windows config and constructs
// the same Rust AppInterface as `tauri build`, including rewrite_manifest,
// without starting the frontend or Rust build. Record its exact byte result,
// but restore the checkout so the real build remains the only build input.
function probeTauriManifestNormalization(root, initialState) {
  const manifestPath = tauriManifestPath(root);
  const original = fs.readFileSync(manifestPath);
  const cli = path.join(root, "node_modules", "@tauri-apps", "cli", "tauri.js");
  let result;
  let normalized;
  try {
    result = spawnSync(process.execPath, [cli, "inspect", "wix-upgrade-code"], {
      cwd: root,
      encoding: "utf8",
      maxBuffer: 32 * 1024 * 1024,
    });
    normalized = fs.readFileSync(manifestPath);
  } finally {
    fs.writeFileSync(manifestPath, original);
  }
  if (result?.status !== 0) {
    throw result?.error || new Error(`Tauri manifest normalization probe failed: ${String(result?.stderr || "").trim()}`);
  }
  if (!normalized) throw new Error("Tauri manifest normalization probe did not produce Cargo.toml");
  const restoredState = buildInputState(root);
  if (restoredState.digest !== initialState.digest) {
    throw new Error("Tauri manifest normalization probe changed build inputs before the snapshot");
  }
  return {
    path: "src-tauri/Cargo.toml",
    originalContentBase64: encoded(original),
    normalizedContentBase64: encoded(normalized),
    originalSha256: sha256(original),
    normalizedSha256: sha256(normalized),
  };
}

function snapshot(root, tauriManifestNormalization) {
  const state = buildInputState(root);
  const normalization = tauriManifestNormalization ? probeTauriManifestNormalization(root, state) : undefined;
  return {
    schemaVersion: 1,
    kind: "tine-e2e-build-input-snapshot",
    repositoryRoot: root,
    sourceRevision: git(root, ["rev-parse", "HEAD"]).trim(),
    buildInputDigest: state.digest,
    buildInputsDirty: state.dirty,
    buildInputChanges: state.changes,
    capturedAt: new Date().toISOString(),
    ...(normalization ? { tauriManifestNormalization: normalization } : {}),
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
  if (value.tauriManifestNormalization !== undefined) {
    const normalization = value.tauriManifestNormalization;
    if (!normalization || Array.isArray(normalization) || normalization.path !== "src-tauri/Cargo.toml"
      || !/^[a-f0-9]{64}$/i.test(normalization.originalSha256 || "")
      || !/^[a-f0-9]{64}$/i.test(normalization.normalizedSha256 || "")) {
      throw new Error(`invalid build-input snapshot ${file}`);
    }
    const original = decoded(normalization.originalContentBase64, `build-input snapshot ${file}`);
    const normalized = decoded(normalization.normalizedContentBase64, `build-input snapshot ${file}`);
    if (sha256(original) !== normalization.originalSha256 || sha256(normalized) !== normalization.normalizedSha256) {
      throw new Error(`invalid build-input snapshot ${file}`);
    }
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

function describeBuildInputDelta(beforeChanges, afterChanges) {
  const before = new Set(beforeChanges);
  const after = new Set(afterChanges);
  const added = afterChanges.filter((change) => !before.has(change)).map((change) => `+ ${change}`);
  const removed = beforeChanges.filter((change) => !after.has(change)).map((change) => `- ${change}`);
  // A build can alter an input that was already dirty before its snapshot. In
  // that case Git's short-status line persists, but naming it is still the
  // smallest useful diagnostic for the digest mismatch.
  const unchanged = afterChanges.filter((change) => before.has(change)).map((change) => `~ ${change}`);
  const entries = [...added, ...removed, ...unchanged];
  const limit = 20;
  if (entries.length === 0) return "build-input status delta: no paths reported by Git";
  const remaining = entries.length - limit;
  return [
    "build-input status delta:",
    ...entries.slice(0, limit).map((entry) => `  ${entry}`),
    ...(remaining > 0 ? [`  ... ${remaining} more path${remaining === 1 ? "" : "s"}`] : []),
  ].join("\n");
}

function restoreExpectedTauriManifestNormalization(root, before) {
  const normalization = before.tauriManifestNormalization;
  if (!normalization) return;
  const manifestPath = tauriManifestPath(root);
  const original = decoded(normalization.originalContentBase64, "Tauri manifest normalization");
  const normalized = decoded(normalization.normalizedContentBase64, "Tauri manifest normalization");
  const actual = fs.readFileSync(manifestPath);
  if (actual.equals(original)) return;
  if (!actual.equals(normalized)) {
    throw new Error("refusing receipt: src-tauri/Cargo.toml diverged from the exact Tauri manifest normalization");
  }
  fs.writeFileSync(manifestPath, original);
}

function createReceipt(root, before, app, dist) {
  const afterRevision = git(root, ["rev-parse", "HEAD"]).trim();
  if (afterRevision !== before.sourceRevision) {
    throw new Error(`refusing receipt: HEAD changed while building (${before.sourceRevision} -> ${afterRevision})`);
  }
  restoreExpectedTauriManifestNormalization(root, before);
  const afterState = buildInputState(root);
  if (afterState.digest !== before.buildInputDigest) {
    throw new Error(`refusing receipt: build-input state changed while building\n${describeBuildInputDelta(before.buildInputChanges, afterState.changes)}`);
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
  writeJson(snapshotPath, snapshot(root, options.tauriManifestNormalization));
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
