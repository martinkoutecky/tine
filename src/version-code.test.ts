// Guard: the Android versionCode in tauri.conf.json must stay in lockstep with
// the semver `version`, and must stay readable by F-Droid's autoupdate checker.
//
// F-Droid (metadata/dev.tine.app.yml, UpdateCheckMode: Tags) reads BOTH the
// versionName and the integer versionCode straight out of this file at every
// git tag, via UpdateCheckData regexes. Tauri derives the same versionCode by
// `major*1e6 + minor*1e3 + patch` when it's not set explicitly, so if someone
// bumps `version` and forgets `bundle.android.versionCode`, the APK and the
// F-Droid metadata would silently disagree. This test turns that into a CI
// failure instead of a broken release.

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const confPath = fileURLToPath(new URL("../src-tauri/tauri.conf.json", import.meta.url));
const confText = readFileSync(confPath, "utf8");
const conf = JSON.parse(confText) as {
  version: string;
  bundle?: { android?: { versionCode?: number } };
};

function deriveVersionCode(version: string): number {
  const m = /^(\d+)\.(\d+)\.(\d+)$/.exec(version);
  if (!m) throw new Error(`unexpected version string: ${version}`);
  const [, major, minor, patch] = m;
  return Number(major) * 1_000_000 + Number(minor) * 1_000 + Number(patch);
}

describe("Android versionCode (F-Droid autoupdate)", () => {
  it("matches Tauri's semver-derived versionCode", () => {
    const explicit = conf.bundle?.android?.versionCode;
    expect(explicit, "bundle.android.versionCode must be set for F-Droid").toBeTypeOf("number");
    expect(explicit).toBe(deriveVersionCode(conf.version));
  });

  // These are the exact regexes in metadata/dev.tine.app.yml's UpdateCheckData
  // field. If the JSON shape changes so they stop matching, F-Droid autoupdate
  // silently stops detecting new releases — catch it here.
  it("stays readable by the F-Droid UpdateCheckData regexes", () => {
    const codeMatch = /"versionCode":\s*([0-9]+)/.exec(confText);
    const verMatch = /"version":\s*"([0-9.]+)"/.exec(confText);
    expect(codeMatch?.[1]).toBe(String(conf.bundle?.android?.versionCode));
    expect(verMatch?.[1]).toBe(conf.version);
  });
});
