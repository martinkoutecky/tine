import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { IDENTITIES } from "./lib/app-identity.mjs";
import { releaseLayout } from "./release-layout.mjs";
import { workflowInputs } from "./release-workflow-inputs.mjs";
import { BETA_ENDPOINT } from "./release-policy.mjs";

// Exercise actual staging/assembly CLI with Tauri's space-bearing filenames,
// signed byte payloads and binary zsync checksums, under both identity switches.
const root = fs.mkdtempSync(path.join(os.tmpdir(), "tine-release-identity-"));
const commit = "a".repeat(40);
const version = "0.5.6";
const payload = Buffer.from([0, 128, 255, 10, 13]);
try {
  for (const file of ["release-layout.mjs", "release-policy.mjs", "stage-release-lane.mjs",
    "assemble-release-candidate.mjs", "release-workflow-inputs.mjs", "lib/app-identity.mjs"]) {
    const dest = path.join(root, "scripts", file);
    fs.mkdirSync(path.dirname(dest), { recursive: true });
    fs.copyFileSync(path.join("scripts", file), dest);
  }
  fs.mkdirSync(path.join(root, "src-tauri/gen/android/app"), { recursive: true });
  fs.writeFileSync(path.join(root, "CHANGELOG.md"), `## [${version}]\nFixture notes\n`);
  const run = (file, ...args) => {
    const result = spawnSync(process.execPath, [path.join(root, "scripts", file), ...args], {
      cwd: root, encoding: "utf8", timeout: 30000,
      env: { ...process.env, GITHUB_SHA: commit, GITHUB_REPOSITORY: "owner/repo" },
    });
    assert.equal(result.status, 0, result.stderr);
    return result.stdout;
  };
  for (const [ship, identity] of Object.entries(IDENTITIES)) {
    fs.writeFileSync(path.join(root, "src-tauri/app-identity.json"), JSON.stringify({ ship, identities: IDENTITIES }));
    const conf = { version, identifier: identity.identifier, productName: identity.productName,
      plugins: { updater: { endpoints: [BETA_ENDPOINT] } } };
    fs.writeFileSync(path.join(root, "src-tauri/tauri.conf.json"), JSON.stringify(conf));
    const layout = releaseLayout(version, identity);
    for (const [lane, spec] of Object.entries(layout.lanes)) {
      const bundle = path.join(root, "target", lane);
      fs.mkdirSync(bundle, { recursive: true });
      for (const name of spec.assets) {
        const source = spec.sourceAssets[name];
        const bytes = name.endsWith(".zsync") ? Buffer.concat([
          Buffer.from(`zsync: 0.6.2\nFilename: ${source.slice(0, -6)}\nURL: ${source.slice(0, -6)}\n\n`), payload,
        ]) : payload;
        fs.writeFileSync(path.join(bundle, source), bytes);
      }
      run("stage-release-lane.mjs", lane, "candidate-input", commit);
      for (const name of spec.assets) {
        const bytes = fs.readFileSync(path.join(root, "candidate-input", lane, name));
        if (name.endsWith(".zsync")) {
          const appimage = name.slice(0, -6);
          assert.ok(bytes.includes(Buffer.from(`Filename: ${appimage}\nURL: ${appimage}\n\n`)));
          assert.deepEqual(bytes.subarray(-payload.length), payload, "zsync binary payload changed");
        } else assert.deepEqual(bytes, payload, "signed artifact changed while renaming");
      }
      const inputs = workflowInputs(lane, conf, "owner/repo", identity);
      if (lane.startsWith("linux")) {
        assert.ok(inputs.UPDATE_INFORMATION.includes("|beta|"));
        assert.ok(!/\s/.test(inputs.UPDATE_INFORMATION));
        assert.equal(inputs.RELEASE_APPIMAGE, spec.sourceAssets[spec.assets[0]]);
      }
    }
    run("assemble-release-candidate.mjs", "candidate-input", "assembled");
    assert.deepEqual(fs.readdirSync(path.join(root, "assembled")).sort(), layout.allAssets.slice().sort());
    const updater = JSON.parse(fs.readFileSync(path.join(root, "assembled/latest.json"), "utf8"));
    for (const [platform, entry] of Object.entries(updater.platforms)) {
      assert.equal(entry.url, `https://github.com/owner/repo/releases/download/beta/${layout.updaterPlatforms[platform][0]}`);
    }
    // Stable layout preserves every previous asset name, including RPM separators.
    if (ship === "release") {
      assert.ok(layout.platformAssets.every((name) => name.startsWith(`Tine_${version}_`) || name.startsWith(`Tine-${version}-1.`)));
    }
    const gradle = `android {\n namespace = "fixture.kotlin.package"\n applicationId = "${identity.androidApplicationId}"\n}\n`;
    fs.writeFileSync(path.join(root, "src-tauri/gen/android/app/build.gradle.kts"), gradle);
    run("release-workflow-inputs.mjs", "android-config");
    const androidConf = JSON.parse(fs.readFileSync(path.join(root, "src-tauri/tauri.conf.json"), "utf8"));
    assert.equal(androidConf.identifier, "fixture.kotlin.package");
    assert.equal(androidConf.productName, identity.productName);
    assert.equal(fs.readFileSync(path.join(root, "src-tauri/gen/android/app/build.gradle.kts"), "utf8"), gradle);
    if (ship === "experiment") {
      fs.writeFileSync(path.join(root, "src-tauri/tauri.conf.json"), JSON.stringify({ ...conf, version: "0.7.0-beta.1" }));
      run("release-workflow-inputs.mjs", "android-config");
      assert.match(fs.readFileSync(path.join(root, "src-tauri/gen/android/app/tauri.properties"), "utf8"), /versionCode=7001\n/);
    }
    fs.writeFileSync(path.join(root, "src-tauri/gen/android/app/build.gradle.kts"),
      gradle.replace(identity.androidApplicationId, "wrong.application.id"));
    const rejected = spawnSync(process.execPath, [path.join(root, "scripts/release-workflow-inputs.mjs"), "android-config"],
      { cwd: root, encoding: "utf8", timeout: 30000 });
    assert.notEqual(rejected.status, 0, "Android preparation accepted applicationId drift");
    fs.rmSync(path.join(root, "target"), { recursive: true });
  }
} finally { fs.rmSync(root, { recursive: true, force: true }); }
console.log("Release identity CLI tests passed for both ships (staging, zsync, assembly, Android namespace).");
