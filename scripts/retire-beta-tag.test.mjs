import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { IDENTITY } from "./lib/app-identity.mjs";
import { ZSYNC_ARCHES, zsyncAssetName } from "./pages-beta-manifest.mjs";
import { releaseLayout } from "./release-layout.mjs";
import { bridgeNotes } from "./retire-beta-tag.mjs";

test("the bridge page links the new Beta's release and its direct downloads", () => {
  const notes = bridgeNotes("owner/repo", "v0.8.0-beta.3", releaseLayout("0.8.0-beta.3"));
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

function legacyState() {
  const layout = releaseLayout(target.slice(1));
  const base = `https://github.com/${repository}/releases/download/${target}/`;
  const assets = Object.fromEntries(layout.platformAssets.map((name) => [name, "app"]));
  assets["latest.json"] = JSON.stringify({ version: target.slice(1), platforms: Object.fromEntries(
    Object.entries(layout.updaterPlatforms).map(([platform, [asset]]) => [platform, { url: `${base}${asset}`, signature: "sig" }])) });
  for (const arch of ZSYNC_ARCHES) {
    const appImage = zsyncAssetName(target, arch).replace(/\.zsync$/, "");
    assets[zsyncAssetName(target, arch)] = `zsync: 0.6.2\nFilename: ${appImage}\nURL: ${base}${appImage}\nSHA-1: 00\n\n\u0001`;
  }
  return {
    mutations: 0,
    pages: target.slice(1),
    tags: { beta: "c2", "v0.8.0-beta.2": "c2", [target]: "c3" },
    releases: [
      { tagName: "beta", name: "Tine Beta 0.8.0-beta.2", isDraft: false, isPrerelease: true, target: "c2", assets: { "latest.json": JSON.stringify({ version: "0.8.0-beta.2" }) } },
      { tagName: target, name: `Tine Beta ${target.slice(1)}`, isDraft: false, isPrerelease: true, target: "c3", assets },
    ],
  };
}

function harness() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "retire-beta-test-"));
  for (const tool of ["gh", "curl"]) {
    fs.writeFileSync(path.join(dir, tool), `#!/bin/sh\nexec "${process.execPath}" "${fake}" ${tool} "$@"\n`, { mode: 0o755 });
  }
  const file = path.join(dir, "state.json");
  fs.writeFileSync(file, JSON.stringify(legacyState()));
  const run = (args, failAt) => spawnSync(process.execPath, ["scripts/retire-beta-tag.mjs", target, ...args], {
    encoding: "utf8",
    env: { ...process.env, PATH: `${dir}:${process.env.PATH}`, FAKE_GITHUB_STATE: file, FAKE_GITHUB_FAIL_AT: failAt ?? "", GITHUB_REPOSITORY: repository },
  });
  return { run, state: () => JSON.parse(fs.readFileSync(file, "utf8")), done: () => fs.rmSync(dir, { recursive: true, force: true }) };
}

function assertBridged(state) {
  const bridge = state.releases.find((release) => release.tagName === "beta");
  assert.equal(bridge.name, `${IDENTITY.productName} update bridge`);
  assert.equal(bridge.isDraft, false);
  assert.equal(state.tags.beta, "c3");
  assert.deepEqual(Object.keys(bridge.assets).sort(), ["latest.json", ...ZSYNC_ARCHES.map((arch) => zsyncAssetName(target, arch))].sort());
  assert.equal(JSON.parse(bridge.assets["latest.json"]).version, target.slice(1));
  assert.ok(state.releases.some((release) => release.tagName === "v0.8.0-beta.2" && release.name === "Tine Beta 0.8.0-beta.2"));
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
  for (let failAt = 1; failAt <= 5; failAt++) {
    const github = harness();
    try {
      const first = github.run(["--execute"], String(failAt));
      assert.notEqual(first.status, 0, `failure at mutation ${failAt} was not reported`);
      const rerun = github.run(["--execute"]);
      assert.equal(rerun.status, 0, `rerun after failure at mutation ${failAt}:\n${rerun.stderr}`);
      assertBridged(github.state());
      const before = github.state().mutations;
      const again = github.run(["--execute"]);
      assert.equal(again.status, 0, again.stderr);
      assert.equal(github.state().mutations, before, `a completed bridge was changed again (failure at ${failAt})`);
    } finally {
      github.done();
    }
  }
});
