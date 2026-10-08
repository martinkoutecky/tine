#!/usr/bin/env node

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { buildInputState, deriveTauriManifest, normalizedBuildInputState } from "./build-e2e-inputs.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const helper = path.join(root, "scripts/build-e2e-receipt.mjs");
const runner = path.join(root, "scripts/run-e2e.mjs");
const inputHelper = path.join(root, "scripts/build-e2e-inputs.mjs");
const capabilities = path.join(root, "scripts/e2e-capabilities.mjs");
const contracts = path.join(root, "tests/ui-regressions/e2e-contracts.json");
const index = path.join(root, "dist/index.html");
const asset = fs.readFileSync(index, "utf8").match(/[A-Za-z0-9_]+-[A-Za-z0-9_-]+\.(?:js|css)/)?.[0];
if (!asset) throw new Error(`could not find a current frontend asset in ${index}`);

function runNode(args, options = {}) {
  return spawnSync(process.execPath, args, { encoding: "utf8", ...options });
}

function runChecked(command, args, options = {}) {
  const result = spawnSync(command, args, { encoding: "utf8", ...options });
  if (result.status !== 0) throw result.error || new Error(`${command} ${args.join(" ")} failed: ${(result.stderr || "").trim()}`);
  return result.stdout;
}

function git(cwd, args) {
  return runChecked("git", args, { cwd }).trim();
}

const temporary = fs.mkdtempSync(path.join(os.tmpdir(), "tine-e2e-provenance-test-"));
try {
  const fixture = path.join(temporary, "fixture");
  fs.mkdirSync(path.join(fixture, "dist"), { recursive: true });
  fs.mkdirSync(path.join(fixture, "scripts"), { recursive: true });
  fs.mkdirSync(path.join(fixture, "src-tauri/gen/schemas"), { recursive: true });
  fs.mkdirSync(path.join(fixture, "node_modules/@tauri-apps/cli"), { recursive: true });
  fs.mkdirSync(path.join(fixture, "tests/ui-regressions"), { recursive: true });
  fs.copyFileSync(helper, path.join(fixture, "scripts/build-e2e-receipt.mjs"));
  fs.copyFileSync(inputHelper, path.join(fixture, "scripts/build-e2e-inputs.mjs"));
  fs.copyFileSync(runner, path.join(fixture, "scripts/run-e2e.mjs"));
  fs.copyFileSync(capabilities, path.join(fixture, "scripts/e2e-capabilities.mjs"));
  fs.copyFileSync(contracts, path.join(fixture, "tests/ui-regressions/e2e-contracts.json"));
  fs.writeFileSync(path.join(fixture, "scripts/e2e-multigraph.mjs"), "// Provenance validation reached the selected scenario.\n");
  fs.writeFileSync(path.join(fixture, "source.txt"), "before\n");
  const fixtureManifest = path.join(fixture, "src-tauri/Cargo.toml");
  const originalManifest = "[build-dependencies]\ntauri-build = { version = \"2\", features = [] }\n\n[dependencies]\ntauri = { version = \"2\", features = [\"image-png\"] }\n";
  const normalizedManifest = originalManifest.replace('features = []', 'features = ["isolation"]');
  fs.writeFileSync(fixtureManifest, originalManifest);
  fs.writeFileSync(path.join(fixture, "node_modules/@tauri-apps/cli/tauri.js"), [
    "const fs = require('node:fs');",
    "const path = require('node:path');",
    "const manifest = path.join(process.cwd(), 'src-tauri', 'Cargo.toml');",
    "fs.writeFileSync(manifest, fs.readFileSync(manifest, 'utf8').replace('features = []', 'features = [\\\"isolation\\\"]'));",
  ].join("\n"));
  fs.writeFileSync(path.join(fixture, "src-tauri/gen/schemas/desktop-schema.json"), "{\"schema\":\"before\"}\n");
  fs.writeFileSync(path.join(fixture, "dist", "index.html"), `<script src="${asset}"></script>\n`);
  const fixtureApp = path.join(fixture, process.platform === "win32" ? "tine.exe" : "tine");
  const launchProbe = path.join(temporary, "app-launched");
  if (process.platform === "win32") {
    fs.writeFileSync(fixtureApp, `@echo off\r\necho launched > "${launchProbe}"\r\nrem ${asset}\r\n`);
  } else {
    fs.writeFileSync(fixtureApp, `#!/usr/bin/env node\nrequire("node:fs").writeFileSync(process.env.TINE_E2E_LAUNCH_PROBE, "launched");\n// ${asset}\n`);
    fs.chmodSync(fixtureApp, 0o755);
  }
  git(fixture, ["init"]);
  git(fixture, ["add", "."]);
  git(fixture, ["-c", "user.email=tine-test@example.invalid", "-c", "user.name=Tine test", "commit", "-m", "fixture"]);

  const snapshot = path.join(temporary, "fixture-before.json");
  const receipt = path.join(temporary, "fixture-receipt.json");
  runChecked(process.execPath, [helper, "before", "--snapshot", snapshot], { cwd: fixture });
  const beforeGeneratedSchemas = buildInputState(fixture);
  fs.writeFileSync(path.join(fixture, "src-tauri/gen/schemas/desktop-schema.json"), "{\"schema\":\"after\"}\n");
  fs.writeFileSync(path.join(fixture, "src-tauri/gen/schemas/x86_64-pc-windows-msvc-schema.json"), "{\"schema\":\"created\"}\n");
  assert.deepEqual(buildInputState(fixture), beforeGeneratedSchemas, "Tauri-generated schema output must not change the build-input state");
  runChecked(process.execPath, [helper, "after", "--snapshot", snapshot, "--app", fixtureApp, "--receipt", receipt], { cwd: fixture });
  const writtenReceipt = JSON.parse(fs.readFileSync(receipt, "utf8"));
  assert.equal(writtenReceipt.sourceRevision, git(fixture, ["rev-parse", "HEAD"]));
  assert.equal(writtenReceipt.frontendAsset, asset);
  assert.equal(writtenReceipt.appSha256, crypto.createHash("sha256").update(fs.readFileSync(fixtureApp)).digest("hex"));
  assert.match(writtenReceipt.buildInputDigest, /^[a-f0-9]{64}$/);
  assert.equal(writtenReceipt.buildInputsDirty, false);

  const normalizedSnapshot = path.join(temporary, "fixture-tauri-normalized-before.json");
  const normalizedReceipt = path.join(temporary, "fixture-tauri-normalized-receipt.json");
  runChecked(process.execPath, [helper, "before", "--tauri-manifest-normalization", "--snapshot", normalizedSnapshot], { cwd: fixture });
  assert.equal(fs.readFileSync(fixtureManifest, "utf8"), originalManifest, "the Tauri manifest probe must restore the checkout before building");
  const normalization = JSON.parse(fs.readFileSync(normalizedSnapshot, "utf8")).tauriManifestNormalization;
  assert.equal(normalization.path, "src-tauri/Cargo.toml");
  assert.equal(Buffer.from(normalization.normalizedContentBase64, "base64").toString("utf8"), normalizedManifest);
  fs.writeFileSync(fixtureManifest, normalizedManifest);
  runChecked(process.execPath, [helper, "after", "--snapshot", normalizedSnapshot, "--app", fixtureApp, "--receipt", normalizedReceipt], { cwd: fixture });
  assert.equal(fs.readFileSync(fixtureManifest, "utf8"), normalizedManifest, "the receipt must preserve the target-native Tauri rewrite it hashes");
  const normalizedWrittenReceipt = JSON.parse(fs.readFileSync(normalizedReceipt, "utf8"));
  assert.deepEqual(normalizedWrittenReceipt.tauriManifestNormalization, normalization, "the receipt must carry the build-proven canonical manifest");

  const expectedNormalizedManifest = Buffer.from(normalization.normalizedContentBase64, "base64");
  assert.deepEqual(deriveTauriManifest(fixture), expectedNormalizedManifest, "normalizing an already-normalized manifest must be idempotent");
  fs.writeFileSync(fixtureManifest, originalManifest);
  assert.deepEqual(deriveTauriManifest(fixture), expectedNormalizedManifest, "a pristine checkout must derive the build-produced manifest");
  assert.equal(
    normalizedBuildInputState(fixture, expectedNormalizedManifest).digest,
    normalizedWrittenReceipt.buildInputDigest,
    "producer and consumer must compute the same shared normalized digest",
  );

  const pristineArtifacts = path.join(temporary, "pristine-normalized-receipt-artifacts");
  const pristineConsumer = runNode([path.join(fixture, "scripts/run-e2e.mjs"), "linux-smoke", "--scenario=multigraph", "--validate-build-receipt-only"], {
    cwd: fixture,
    env: {
      ...process.env,
      TINE_APP: fixtureApp,
      TINE_E2E_BUILD_RECEIPT: normalizedReceipt,
      TINE_E2E_LAUNCH_PROBE: launchProbe,
      E2E_ARTIFACT_DIR: pristineArtifacts,
    },
  });
  assert.equal(pristineConsumer.status, 0, pristineConsumer.stderr || pristineConsumer.stdout);
  assert.equal(fs.existsSync(pristineArtifacts), false, "receipt-only validation must not start E2E artifact work");

  fs.writeFileSync(fixtureManifest, originalManifest.replace('version = "2"', 'version = "999"'));
  const changedCargoArtifacts = path.join(temporary, "changed-cargo-artifacts");
  const changedCargoConsumer = runNode([path.join(fixture, "scripts/run-e2e.mjs"), "linux-smoke", "--scenario=multigraph", "--validate-build-receipt-only"], {
    cwd: fixture,
    env: {
      ...process.env,
      TINE_APP: fixtureApp,
      TINE_E2E_BUILD_RECEIPT: normalizedReceipt,
      TINE_E2E_LAUNCH_PROBE: launchProbe,
      E2E_ARTIFACT_DIR: changedCargoArtifacts,
    },
  });
  assert.notEqual(changedCargoConsumer.status, 0);
  assert.match(changedCargoConsumer.stderr, /built from different build inputs than the current checkout/);
  assert.equal(fs.existsSync(changedCargoArtifacts), false, "run-e2e started artifact work before rejecting changed Cargo.toml");
  const changedCargo = runNode([helper, "after", "--snapshot", normalizedSnapshot, "--app", fixtureApp, "--receipt", path.join(temporary, "changed-cargo-receipt.json")], { cwd: fixture });
  assert.notEqual(changedCargo.status, 0);
  assert.match(changedCargo.stderr, /refusing receipt: src-tauri\/Cargo\.toml diverged from the exact target-native Tauri manifest normalization/);
  fs.writeFileSync(fixtureManifest, originalManifest);

  fs.writeFileSync(fixtureManifest, normalizedManifest);
  fs.writeFileSync(path.join(fixture, "source.txt"), "changed alongside the Tauri rewrite\n");
  const changedSourceAlongsideNormalization = runNode([helper, "after", "--snapshot", normalizedSnapshot, "--app", fixtureApp, "--receipt", path.join(temporary, "changed-source-alongside-normalization-receipt.json")], { cwd: fixture });
  assert.notEqual(changedSourceAlongsideNormalization.status, 0);
  assert.match(changedSourceAlongsideNormalization.stderr, /refusing receipt: build-input state changed while building/);
  assert.equal(fs.readFileSync(fixtureManifest, "utf8"), normalizedManifest, "receipt validation must preserve the build-produced Tauri rewrite");
  fs.writeFileSync(fixtureManifest, originalManifest);
  fs.writeFileSync(path.join(fixture, "source.txt"), "before\n");

  fs.writeFileSync(path.join(fixture, "source.txt"), "after\n");
  const changedSourceState = buildInputState(fixture);
  assert.notEqual(changedSourceState.digest, beforeGeneratedSchemas.digest, "ordinary tracked source changes must alter the build-input state");
  assert.equal(changedSourceState.dirty, true);
  assert.ok(changedSourceState.changes.some((change) => change.endsWith("source.txt")));
  const changedReceipt = runNode([helper, "after", "--snapshot", snapshot, "--app", fixtureApp, "--receipt", path.join(temporary, "changed-input-receipt.json")], { cwd: fixture });
  assert.notEqual(changedReceipt.status, 0);
  assert.match(changedReceipt.stderr, /refusing receipt: build-input state changed while building\n?build-input status delta:\n  \+  M source\.txt/);
  const artifacts = path.join(temporary, "changed-input-artifacts");
  const changed = runNode([path.join(fixture, "scripts/run-e2e.mjs"), "linux-smoke", "--scenario=multigraph"], {
    cwd: fixture,
    env: {
      ...process.env,
      TINE_APP: fixtureApp,
      TINE_E2E_BUILD_RECEIPT: receipt,
      TINE_E2E_LAUNCH_PROBE: launchProbe,
      E2E_ARTIFACT_DIR: artifacts,
    },
  });
  assert.notEqual(changed.status, 0);
  assert.match(changed.stderr, /built from different build inputs than the current checkout/);
  assert.equal(fs.existsSync(launchProbe), false, "run-e2e launched an app before rejecting changed build inputs");
  assert.equal(fs.existsSync(artifacts), false, "run-e2e started E2E artifact work before checking changed build inputs");

  const fakeApp = path.join(temporary, process.platform === "win32" ? "unreceipted.cmd" : "unreceipted-app");
  if (process.platform === "win32") {
    fs.writeFileSync(fakeApp, `@echo off\r\necho launched > "${launchProbe}"\r\nrem ${asset}\r\n`);
  } else {
    fs.writeFileSync(fakeApp, `#!/usr/bin/env node\nrequire("node:fs").writeFileSync(process.env.TINE_E2E_LAUNCH_PROBE, "launched");\n// ${asset}\n`);
    fs.chmodSync(fakeApp, 0o755);
  }
  const unreceiptedArtifacts = path.join(temporary, "unreceipted-artifacts");
  const unreceipted = runNode([runner, "linux-smoke", "--scenario=multigraph"], {
    cwd: root,
    env: {
      ...process.env,
      TINE_APP: fakeApp,
      TINE_E2E_BUILD_RECEIPT: path.join(temporary, "missing-receipt.json"),
      TINE_E2E_LAUNCH_PROBE: launchProbe,
      E2E_ARTIFACT_DIR: unreceiptedArtifacts,
    },
  });
  assert.notEqual(unreceipted.status, 0);
  assert.match(unreceipted.stderr, /build receipt is required at/);
  assert.equal(fs.existsSync(launchProbe), false, "run-e2e launched an app before rejecting its missing receipt");
  assert.equal(fs.existsSync(unreceiptedArtifacts), false, "run-e2e started E2E artifact work before provenance validation");
} finally {
  fs.rmSync(temporary, { recursive: true, force: true });
}

console.log("E2E provenance tests passed (receipt input digest + no-launch receipt rejection).");
