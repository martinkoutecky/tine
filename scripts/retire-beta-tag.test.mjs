import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import test from "node:test";
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
