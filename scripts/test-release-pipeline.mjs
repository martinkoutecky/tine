#!/usr/bin/env node

import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { assembleCandidate } from "./assemble-release-candidate.mjs";
import { candidateProblems, releaseLayout, RELEASE_LANES } from "./release-layout.mjs";

const version = "0.5.6";
const commit = "a".repeat(40);
const repository = "martinkoutecky/tine";
const layout = releaseLayout(version);

function makeInput(base) {
  const input = path.join(base, "input");
  fs.mkdirSync(input, { recursive: true });
  for (const lane of RELEASE_LANES) {
    const directory = path.join(input, `release-${lane}`);
    fs.mkdirSync(directory, { recursive: true });
    const assets = [];
    for (const name of layout.lanes[lane].assets) {
      const contents = name.endsWith(".sig") ? `signature-${name}\n` : `fixture-${name}\n`;
      fs.writeFileSync(path.join(directory, name), contents);
      const bytes = Buffer.from(contents);
      assets.push({ name, size: bytes.length, sha256: createHash("sha256").update(bytes).digest("hex") });
    }
    const platforms = {};
    for (const [platform, [asset, signatureAsset]] of Object.entries(layout.lanes[lane].platforms)) {
      platforms[platform] = {
        asset,
        signature: fs.readFileSync(path.join(directory, signatureAsset), "utf8").trim(),
      };
    }
    fs.writeFileSync(
      path.join(directory, "release-fragment.json"),
      `${JSON.stringify({ version, commit, lane, assets, platforms }, null, 2)}\n`
    );
  }
  return input;
}

function assemble(input, output) {
  assembleCandidate({
    input,
    output,
    version,
    commit,
    repository,
    pubDate: "2026-07-11T00:00:00.000Z",
  });
}

const temporary = fs.mkdtempSync(path.join(os.tmpdir(), "tine-release-pipeline-test-"));
try {
  {
    const base = path.join(temporary, "valid");
    const input = makeInput(base);
    const output = path.join(base, "output");
    assemble(input, output);
    assert.deepEqual(candidateProblems(output, version), []);
  }
  {
    const base = path.join(temporary, "missing-android");
    const input = makeInput(base);
    fs.rmSync(path.join(input, "release-android"), { recursive: true });
    assert.throws(() => assemble(input, path.join(base, "output")), /missing release lanes: android/);
  }
  {
    const base = path.join(temporary, "missing-signature");
    const input = makeInput(base);
    fs.rmSync(path.join(input, "release-windows-x64", `Tine_${version}_x64-setup.exe.sig`));
    assert.throws(() => assemble(input, path.join(base, "output")), /ENOENT/);
  }
  {
    const base = path.join(temporary, "wrong-version");
    const input = makeInput(base);
    const fragmentPath = path.join(input, "release-macos-universal", "release-fragment.json");
    const fragment = JSON.parse(fs.readFileSync(fragmentPath, "utf8"));
    fragment.version = "0.5.7";
    fs.writeFileSync(fragmentPath, JSON.stringify(fragment));
    assert.throws(() => assemble(input, path.join(base, "output")), /version 0\.5\.7, expected 0\.5\.6/);
  }
  {
    const base = path.join(temporary, "duplicate-platform");
    const input = makeInput(base);
    const fragmentPath = path.join(input, "release-windows-x64", "release-fragment.json");
    const fragment = JSON.parse(fs.readFileSync(fragmentPath, "utf8"));
    fragment.platforms["linux-x86_64"] = fragment.platforms["windows-x86_64"];
    fs.writeFileSync(fragmentPath, JSON.stringify(fragment));
    assert.throws(() => assemble(input, path.join(base, "output")), /updater platform contract mismatch/);
  }
  {
    const base = path.join(temporary, "incomplete-updater");
    const input = makeInput(base);
    const output = path.join(base, "output");
    assemble(input, output);
    const updaterPath = path.join(output, "latest.json");
    const updater = JSON.parse(fs.readFileSync(updaterPath, "utf8"));
    delete updater.platforms["windows-aarch64"];
    fs.writeFileSync(updaterPath, JSON.stringify(updater));
    assert(candidateProblems(output, version).some((problem) => problem.includes("windows-aarch64")));
  }
} finally {
  fs.rmSync(temporary, { recursive: true, force: true });
}

console.log("Release pipeline fixture tests passed (valid + 5 fail-closed cases).");
