import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { createHash } from "node:crypto";
import { pathToFileURL } from "node:url";
import { spawnSync } from "node:child_process";
import { assembleCandidate } from "./assemble-release-candidate.mjs";
import { candidateProblems } from "./release-layout.mjs";
import { releaseLayout } from "./release-layout.mjs";
import { updaterAssetUrl, releaseVersion, packagingProblems, publicationPlan, BETA_ENDPOINT } from "./release-policy.mjs";

const conf = { version: "0.7.0-beta.1", bundle: { targets: ["nsis", "dmg", "appimage"], android: { versionCode: 7001 } }, plugins: { updater: { endpoints: [BETA_ENDPOINT] } } };
assert.equal(releaseVersion("0.6.987").androidCode, 6987);
assert.equal(releaseVersion(conf.version).androidCode, 7001);
assert.equal(releaseVersion("0.7.0-beta.2").androidCode, 7002);
assert.ok(releaseVersion("0.8.0-beta.1").androidCode > releaseVersion("0.7.0-beta.999").androidCode);
for (const invalid of ["0.7.1-beta.1", "0.7.0-beta.0", "0.7.0-beta.01", "0.7.0-og.1", "0.7.0-beta.1000", "0.7.1000", "01.7.0"]) {
  assert.throws(() => releaseVersion(invalid));
}
assert.deepEqual(packagingProblems(conf, "experiment"), []);
assert.match(packagingProblems(conf, "release").join(), /separate experiment Android/);
assert.match(packagingProblems({ ...conf, bundle: { ...conf.bundle, targets: ["msi"] } }, "experiment").join(), /MSI cannot express/);
assert.equal(publicationPlan({ conf, mode: "build", publish: false }).publish, false);
assert.deepEqual(publicationPlan({ conf, mode: "build", publish: true, tag: "beta" }), { channel: "beta", publish: true, prerelease: true, latest: false });
for (const tag of ["latest", "v0.7.0", "v0.7.0-beta.1", undefined]) assert.throws(() => publicationPlan({ conf, mode: "build", publish: true, tag }));
assert.throws(() => publicationPlan({ conf, mode: "promote", publish: false }));
assert.throws(() => publicationPlan({ conf: { ...conf, plugins: { updater: { endpoints: ["https://github.com/martinkoutecky/tine/releases/latest/download/latest.json"] } } }, mode: "build", publish: false }));
assert.equal(updaterAssetUrl("owner/repo", "app.sig", "beta"), "https://github.com/owner/repo/releases/download/beta/app.sig");
assert.ok(releaseLayout(conf.version).allAssets.includes(`Tine_${conf.version}_android-arm64.apk`));
const workflow = fs.readFileSync(".github/workflows/release.yml", "utf8");
assert.match(workflow, /mode:[\s\S]*?options: \[build\]/);
assert.match(workflow, /publish:[\s\S]*?default: false/);
assert.doesNotMatch(workflow, /\n  push:/);
assert.match(workflow, /if: inputs.publish/);
assert.match(workflow, /publish-release-candidate.mjs beta/);
assert.match(workflow, /build-release-bundles.mjs/);
assert.match(workflow, /group: release-beta/);
assert.doesNotMatch(workflow, /gh-releases-zsync\|[^"\n]+\|latest\|/);

// Drive the actual assembler and publisher, with synthetic signed artifacts and
// a local gh executable. No network call or release mutation is possible here.
const temp = fs.mkdtempSync(path.join(os.tmpdir(), "tine-beta-policy-"));
try {
  const version = JSON.parse(fs.readFileSync("src-tauri/tauri.conf.json", "utf8")).version;
  const commit = "a".repeat(40);
  const input = path.join(temp, "input");
  const output = path.join(temp, "candidate");
  for (const [lane, spec] of Object.entries(releaseLayout(version).lanes)) {
    const dir = path.join(input, lane);
    fs.mkdirSync(dir, { recursive: true });
    const assets = spec.assets.map((name) => {
      const bytes = Buffer.from(`synthetic ${name}`);
      fs.writeFileSync(path.join(dir, name), bytes);
      return { name, size: bytes.length, sha256: createHash("sha256").update(bytes).digest("hex") };
    });
    const platforms = Object.fromEntries(Object.entries(spec.platforms).map(([platform, [asset, signatureAsset]]) => [platform, { asset, signature: fs.readFileSync(path.join(dir, signatureAsset), "utf8").trim() }]));
    fs.writeFileSync(path.join(dir, "release-fragment.json"), JSON.stringify({ version, commit, channel: "beta", lane, assets, platforms }));
  }
  assembleCandidate({ input, output, version, commit, repository: "owner/repo", channel: "beta" });
  assert.deepEqual(candidateProblems(output, version, "beta"), []);
  const updater = JSON.parse(fs.readFileSync(path.join(output, "latest.json"), "utf8"));
  for (const entry of Object.values(updater.platforms)) assert.match(entry.url, /\/releases\/download\/beta\//);

  const calls = path.join(temp, "calls.jsonl");
  const state = path.join(temp, "release-created");
  const loader = path.join(temp, "mock-gh.mjs");
  fs.writeFileSync(loader, `
import fs from 'node:fs';
import path from 'node:path';
import childProcess from 'node:child_process';
import { syncBuiltinESMExports } from 'node:module';
const originalExec = childProcess.execFileSync;
const originalSpawn = childProcess.spawnSync;
function gh(args) {
  fs.appendFileSync(process.env.MOCK_CALLS, JSON.stringify(args) + '\\n');
  let status = 0, stdout = '';
  if (args[0] === 'api') stdout = process.env.GITHUB_SHA;
  else if (args[1] === 'create') fs.writeFileSync(process.env.MOCK_STATE, 'draft');
  else if (args[1] === 'view') {
    if (!fs.existsSync(process.env.MOCK_STATE)) status = 1;
    else stdout = JSON.stringify({isDraft:true,tagName:'beta',assets:fs.readdirSync(process.env.MOCK_CANDIDATE).map(name=>({name}))});
  } else if (args[1] === 'download') {
    const dest = args[args.indexOf('--dir')+1];
    fs.copyFileSync(path.join(process.env.MOCK_CANDIDATE,'latest.json'), path.join(dest,'latest.json'));
  }
  return {status, stdout, stderr:''};
}
childProcess.execFileSync = (command, args, options) => {
  if (command !== 'gh') return originalExec(command, args, options);
  const result = gh(args);
  if (result.status) throw new Error('mock gh failed');
  return result.stdout;
};
childProcess.spawnSync = (command, args, options) => {
  if (command === 'gh') return gh(args);
  if (command === 'npm' || command === 'npm.cmd') {
    fs.writeFileSync(process.env.MOCK_BUILD, JSON.stringify({args, env: options.env}));
    return {status:0, stdout:'', stderr:''};
  }
  return originalSpawn(command, args, options);
};
syncBuiltinESMExports();
`);
  const env = { ...process.env, NODE_OPTIONS: `${process.env.NODE_OPTIONS ?? ""} --import=${pathToFileURL(loader).href}`, GITHUB_REPOSITORY: "owner/repo", GITHUB_SHA: commit, RELEASE_MODE: "build", MOCK_CALLS: calls, MOCK_STATE: state, MOCK_CANDIDATE: output };
  const buildRecord = path.join(temp, "build.json");
  const built = spawnSync(process.execPath, ["scripts/build-release-bundles.mjs", "--", "--features", "custom-protocol"], {
    env: { ...env, MOCK_BUILD: buildRecord, UPDATE_INFORMATION: "gh-releases-zsync|owner|repo|latest|Tine_*.AppImage.zsync", LDAI_UPDATE_INFORMATION: "gh-releases-zsync|owner|repo|latest|Tine_*.AppImage.zsync" }, encoding: "utf8",
  });
  assert.equal(built.status, 0, built.stderr);
  const build = JSON.parse(fs.readFileSync(buildRecord, "utf8"));
  assert.deepEqual(build.args, ["run", "tauri", "build", "--", "--features", "custom-protocol"]);
  for (const key of ["UPDATE_INFORMATION", "LDAI_UPDATE_INFORMATION"]) assert.equal(build.env[key], "gh-releases-zsync|owner|repo|beta|Tine_*.AppImage.zsync");
  const publish = (tag, value) => spawnSync(process.execPath, ["scripts/publish-release-candidate.mjs", tag, output], { env: { ...env, RELEASE_PUBLISH: value }, encoding: "utf8" });
  assert.equal(publish("beta", "false").status, 0);
  assert.equal(fs.existsSync(calls), false, "publish=false made a gh call");
  assert.notEqual(publish(`v${version}`, "true").status, 0);
  assert.equal(fs.existsSync(calls), false, "stable rejection happened after a remote call");
  const published = publish("beta", "true");
  assert.equal(published.status, 0, published.stderr);
  const commands = fs.readFileSync(calls, "utf8").trim().split("\n").map(JSON.parse);
  for (const args of commands.filter((args) => args[0] === "release")) assert.equal(args[2], "beta");
  for (const verb of ["create", "edit"]) {
    const args = commands.find((args) => args[1] === verb);
    assert.ok(args.includes("--latest=false"));
    assert.ok(args.includes(verb === "create" ? "--prerelease" : "--prerelease=true"));
  }
  const firstPlatform = Object.keys(updater.platforms)[0];
  updater.platforms[firstPlatform].url = updater.platforms[firstPlatform].url.replace("download/beta", "latest/download");
  fs.writeFileSync(path.join(output, "latest.json"), JSON.stringify(updater));
  assert.match(candidateProblems(output, version, "beta").join(), /escapes beta/);
  const count = commands.length;
  assert.notEqual(publish("beta", "true").status, 0);
  assert.equal(fs.readFileSync(calls, "utf8").trim().split("\n").length, count, "bad updater map reached gh");
} finally {
  fs.rmSync(temp, { recursive: true, force: true });
}
console.log("Beta release policy OK");
