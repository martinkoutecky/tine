import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { IDENTITIES } from "./lib/app-identity.mjs";
// The bridge serves Tine Beta installs whichever identity this tree ships.
const IDENTITY = IDENTITIES.experiment;
import { ZSYNC_ARCHES, zsyncAssetName } from "./pages-beta-manifest.mjs";
import { releaseLayout } from "./release-layout.mjs";
import { bridgeNotes } from "./retire-beta-tag.mjs";

test("the bridge page links the new Beta's release and its direct downloads", () => {
  const notes = bridgeNotes("owner/repo", "v0.8.0-beta.3", releaseLayout("0.8.0-beta.3", IDENTITY));
  assert.match(notes, /\[v0\.8\.0-beta\.3\]\(https:\/\/github\.com\/owner\/repo\/releases\/tag\/v0\.8\.0-beta\.3\)/);
  for (const suffix of ["_universal.dmg", "_x64-setup.exe", "_arm64-setup.exe", "_x86-setup.exe", "_amd64.AppImage", "_aarch64.AppImage"]) {
    assert.match(notes, new RegExp(`\\(https://github\\.com/owner/repo/releases/download/v0\\.8\\.0-beta\\.3/[^)]*${suffix.replaceAll(".", "\\.")}\\)`), suffix);
  }
});

test("refuses tags before the first fixed-manifest Beta without touching GitHub", () => {
  for (const tag of ["v0.8.0-beta.2", "v0.7.9-beta.9", "beta", "v0.8.0"]) {
    const run = spawnSync(process.execPath, ["scripts/retire-beta-tag.mjs", tag], { encoding: "utf8", env: { ...process.env, PATH: "/nonexistent" } });
    assert.notEqual(run.status, 0, tag);
    assert.match(run.stderr, /usage: retire-beta-tag/, tag);
  }
});

// GitHub as retire-beta-tag.mjs finds it right after beta.3 is published: the
// beta.2 release still at the moving tag, beta.3 published and served.
const fake = path.join(path.dirname(fileURLToPath(import.meta.url)), "fixtures", "fake-github.mjs");
const repository = "martinkoutecky/tine";
const target = "v0.8.0-beta.3";
const bridgeFiles = ["latest.json", ...ZSYNC_ARCHES.map((arch) => zsyncAssetName(target, arch))];
const uploaded = (data, id) => ({ id, state: "uploaded", data });

function legacyState() {
  const layout = releaseLayout(target.slice(1), IDENTITY);
  const base = `https://github.com/${repository}/releases/download/${target}/`;
  const files = Object.fromEntries(layout.platformAssets.map((name) => [name, "app"]));
  files["latest.json"] = JSON.stringify({ version: target.slice(1), platforms: Object.fromEntries(
    Object.entries(layout.updaterPlatforms).map(([platform, [asset]]) => [platform, { url: `${base}${asset}`, signature: "sig" }])) });
  for (const arch of ZSYNC_ARCHES) {
    const appImage = zsyncAssetName(target, arch).replace(/\.zsync$/, "");
    files[zsyncAssetName(target, arch)] = `zsync: 0.6.2\nFilename: ${appImage}\nURL: ${base}${appImage}\nSHA-1: 00\n\n\u0001`;
  }
  let id = 0;
  const assets = Object.fromEntries(Object.entries(files).map(([name, data]) => [name, uploaded(data, ++id)]));
  return {
    mutations: 0,
    nextAsset: 1000,
    pages: target.slice(1),
    tags: { beta: "c2", "v0.8.0-beta.2": "c2", [target]: "c3" },
    releases: [
      { tagName: "beta", name: `${IDENTITY.productName} 0.8.0-beta.2`, isDraft: false, isPrerelease: true, target: "c2",
        assets: { "latest.json": uploaded(JSON.stringify({ version: "0.8.0-beta.2" }), 900) } },
      { tagName: target, name: `${IDENTITY.productName} ${target.slice(1)}`, isDraft: false, isPrerelease: true, target: "c3", assets },
    ],
  };
}

function harness(initial = legacyState()) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "retire-beta-test-"));
  for (const tool of ["gh", "curl"]) {
    fs.writeFileSync(path.join(dir, tool), `#!/bin/sh\nexec "${process.execPath}" "${fake}" ${tool} "$@"\n`, { mode: 0o755 });
  }
  const file = path.join(dir, "state.json");
  fs.writeFileSync(file, JSON.stringify(initial));
  const run = (args, faults = {}) => spawnSync(process.execPath, ["scripts/retire-beta-tag.mjs", target, ...args], {
    encoding: "utf8",
    env: { ...process.env, PATH: `${dir}:${process.env.PATH}`, FAKE_GITHUB_STATE: file, GITHUB_REPOSITORY: repository,
      FAKE_GITHUB_FAIL_AT: faults.failAt ?? "", FAKE_GITHUB_PARTIAL: faults.partial ?? "", FAKE_GITHUB_FAIL_READ: faults.failRead ?? "" },
  });
  return { run, state: () => JSON.parse(fs.readFileSync(file, "utf8")), done: () => fs.rmSync(dir, { recursive: true, force: true }) };
}

function assertBridged(state) {
  const bridge = state.releases.find((release) => release.tagName === "beta");
  const source = state.releases.find((release) => release.tagName === target);
  assert.equal(bridge.name, `${IDENTITY.productName} update bridge`);
  assert.equal(bridge.isDraft, false);
  assert.equal(state.tags.beta, "c3", "the bridge tag is the new Beta's commit");
  assert.deepEqual(Object.keys(bridge.assets).sort(), [...bridgeFiles].sort());
  for (const name of bridgeFiles) {
    assert.equal(bridge.assets[name].state, "uploaded", name);
    assert.equal(bridge.assets[name].data, source.assets[name].data, name);
  }
  assert.ok(state.releases.some((release) => release.tagName === "v0.8.0-beta.2" && release.name === `${IDENTITY.productName} 0.8.0-beta.2`));
}

/** Run once with `faults` (expected to fail), then until done; a further run changes nothing. */
function recovers(faults, initial) {
  const github = harness(initial);
  try {
    const first = github.run(["--execute"], faults);
    assert.notEqual(first.status, 0, `${JSON.stringify(faults)} was not reported`);
    const rerun = github.run(["--execute"]);
    assert.equal(rerun.status, 0, `rerun after ${JSON.stringify(faults)}:\n${rerun.stderr}`);
    assertBridged(github.state());
    const before = github.state().mutations;
    const again = github.run(["--execute"]);
    assert.equal(again.status, 0, again.stderr);
    assert.equal(github.state().mutations, before, `a completed bridge was changed again after ${JSON.stringify(faults)}`);
  } finally {
    github.done();
  }
}

test("a dry run reports the plan and changes nothing", () => {
  const github = harness();
  try {
    const run = github.run([]);
    assert.equal(run.status, 0, run.stderr);
    assert.match(run.stdout, /PLAN move the 0\.8\.0-beta\.2 release/);
    assert.match(run.stdout, /Dry run only/);
    assert.deepEqual(github.state(), legacyState());
  } finally {
    github.done();
  }
});

test("an execute run that fails at any GitHub mutation completes on a rerun, and a further rerun changes nothing", () => {
  const clean = harness();
  try {
    const run = clean.run(["--execute"]);
    assert.equal(run.status, 0, run.stderr);
    assert.equal(clean.state().mutations, 5);
    assertBridged(clean.state());
  } finally {
    clean.done();
  }
  for (let failAt = 1; failAt <= 5; failAt++) recovers({ failAt: String(failAt) });
});

test("a failed upload that leaves any mix of complete, empty (starter) and missing files is repaired", () => {
  // Mutation 4 is the bridge upload of latest.json and both zsync files.
  for (const partial of ["complete,starter,absent", "starter,complete,complete", "starter,starter,starter", "absent,complete,starter"]) {
    recovers({ failAt: "4", partial });
  }
});

test("a failed read of the beta tag stops the run instead of leaving the bridge on the old commit", () => {
  // Reads of commits/beta: before the move, before re-creating, before publishing, in the final check.
  for (const nth of [1, 2, 3, 4]) recovers({ failRead: `commits/beta:${nth}` });
  // The release lookups at beta: an outage there must not read as "no release".
  for (const nth of [1, 2, 3]) recovers({ failRead: `releases/tags/beta:${nth}` });
  for (const nth of [1, 2]) recovers({ failRead: `releases?per_page:${nth}` });
  // Re-review S1: an outage at the first release lookup, then a failed create.
  recovers({ failRead: "releases/tags/beta:1", failAt: "2" });
});

test("a draft bridge left on the old beta tag is published at the new Beta's commit", () => {
  const state = legacyState();
  const legacy = state.releases.find((release) => release.tagName === "beta");
  legacy.tagName = "v0.8.0-beta.2";
  state.releases.push({ tagName: "beta", name: `${IDENTITY.productName} update bridge`, isDraft: true, isPrerelease: true, target: "c2", notes: "", assets: {} });
  const github = harness(state);
  try {
    const run = github.run(["--execute"]);
    assert.equal(run.status, 0, run.stderr);
    assertBridged(github.state());
  } finally {
    github.done();
  }
});

test("refuses, before any change, a target whose manual downloads are missing, and a bridge frozen at another Beta", () => {
  const missing = legacyState();
  const assets = missing.releases.find((release) => release.tagName === target).assets;
  for (const name of Object.keys(assets)) if (/_universal\.dmg$|_x86-setup\.exe$/.test(name)) delete assets[name];
  const github = harness(missing);
  try {
    const run = github.run(["--execute"]);
    assert.notEqual(run.status, 0);
    assert.match(run.stderr, /_universal\.dmg answers 404/);
    assert.match(run.stderr, /_x86-setup\.exe answers 404/);
    assert.equal(github.state().mutations, 0);
  } finally {
    github.done();
  }
  const frozen = legacyState();
  const legacy = frozen.releases.find((release) => release.tagName === "beta");
  legacy.tagName = "v0.8.0-beta.2";
  frozen.releases.push({ tagName: "beta", name: `${IDENTITY.productName} update bridge`, isDraft: false, isPrerelease: true, target: "c3", notes: "",
    assets: { "latest.json": uploaded(JSON.stringify({ version: "0.8.0-beta.4" }), 901) } });
  frozen.tags.beta = "c3";
  const bridged = harness(frozen);
  try {
    const run = bridged.run(["--execute"]);
    assert.notEqual(run.status, 0);
    assert.match(run.stderr, /frozen at 0\.8\.0-beta\.4/);
    assert.equal(bridged.state().mutations, 0);
  } finally {
    bridged.done();
  }
});
