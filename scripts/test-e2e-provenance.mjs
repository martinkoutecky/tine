#!/usr/bin/env node

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

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
  fs.mkdirSync(path.join(fixture, "tests/ui-regressions"), { recursive: true });
  fs.copyFileSync(helper, path.join(fixture, "scripts/build-e2e-receipt.mjs"));
  fs.copyFileSync(inputHelper, path.join(fixture, "scripts/build-e2e-inputs.mjs"));
  fs.copyFileSync(runner, path.join(fixture, "scripts/run-e2e.mjs"));
  fs.copyFileSync(capabilities, path.join(fixture, "scripts/e2e-capabilities.mjs"));
  fs.copyFileSync(contracts, path.join(fixture, "tests/ui-regressions/e2e-contracts.json"));
  fs.writeFileSync(path.join(fixture, "source.txt"), "before\n");
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
  runChecked(process.execPath, [helper, "after", "--snapshot", snapshot, "--app", fixtureApp, "--receipt", receipt], { cwd: fixture });
  const writtenReceipt = JSON.parse(fs.readFileSync(receipt, "utf8"));
  assert.equal(writtenReceipt.sourceRevision, git(fixture, ["rev-parse", "HEAD"]));
  assert.equal(writtenReceipt.frontendAsset, asset);
  assert.equal(writtenReceipt.appSha256, crypto.createHash("sha256").update(fs.readFileSync(fixtureApp)).digest("hex"));
  assert.match(writtenReceipt.buildInputDigest, /^[a-f0-9]{64}$/);
  assert.equal(writtenReceipt.buildInputsDirty, false);

  fs.writeFileSync(path.join(fixture, "source.txt"), "after\n");
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
