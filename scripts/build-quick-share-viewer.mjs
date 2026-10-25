#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { fileHashes, sha256, VIEWER_ZIP, fixtureFingerprint, packViewer } from "./quick-share-viewer-lib.mjs";

const root = path.resolve(import.meta.dirname, "..");
const artifacts = path.join(root, "artifacts/quick-share-viewer");
const output = path.join(artifacts, "build");
const commit = execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim();
const dirty = execFileSync("git", ["status", "--porcelain"], { cwd: root, encoding: "utf8" }).trim();
const builtFrom = process.env.VIEWER_BUILT_FROM || `${commit}${dirty ? "+dirty" : ""}`;
execFileSync(process.execPath, ["scripts/check-wasm-pin.mjs"], { cwd: root, stdio: "inherit" });
execFileSync("npx", ["--no-install", "vite", "build", "--mode", "quick-share"], { cwd: root, stdio: "inherit", env: { ...process.env, CARGO_BUILD_JOBS: "4" } });
fs.renameSync(path.join(output, "quick-share.html"), path.join(output, "index.html"));
const manifest = { generation: 1, link: 1, envelope: 1, document: 1, snapshot_schema: 1, built_from: builtFrom, fixture_sha256: fixtureFingerprint(root), files: fileHashes(output) };
fs.writeFileSync(path.join(output, "viewer.json"), JSON.stringify(manifest, null, 2) + "\n");
// Compatibility is checked against the production files before creating an artifact.
execFileSync(process.execPath, ["scripts/check-quick-share-viewer.mjs", output], { cwd: root, stdio: "inherit" });
const zip = path.join(artifacts, VIEWER_ZIP);
// zip updates an existing archive; use a fresh name so stale files cannot survive.
const fresh = path.join(artifacts, `viewer-${process.pid}.zip`);
packViewer(output, fresh);
fs.renameSync(fresh, zip);
const digest = sha256(fs.readFileSync(zip));
fs.writeFileSync(`${zip}.sha256`, `${digest}  ${VIEWER_ZIP}\n`);
console.log(`${VIEWER_ZIP}: ${digest}; built_from=${builtFrom}`);
