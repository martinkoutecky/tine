// Guard release metadata through the shared release policy. Stable versions stay
// readable by F-Droid's tag checker; Beta uses its separate Android identity and
// must not match that stable-only versionName expression.

import { execFileSync } from "node:child_process";
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
  return Number(execFileSync(process.execPath, [
    "--input-type=module", "-e",
    "import { releaseVersion } from './scripts/release-policy.mjs'; console.log(releaseVersion(process.argv[1]).androidCode);",
    version,
  ], { encoding: "utf8" }).trim());
}

describe("Android versionCode (F-Droid autoupdate)", () => {
  it("preview names advance Android codes only under the separate app identity and isolate the updater channel", () => {
    execFileSync(process.execPath, ["scripts/test-beta-release.mjs"], { stdio: "pipe" });
  });
  it("matches the shared release policy versionCode", () => {
    const explicit = conf.bundle?.android?.versionCode;
    expect(explicit, "bundle.android.versionCode must be set for F-Droid").toBeTypeOf("number");
    expect(explicit).toBe(deriveVersionCode(conf.version));
  });

  // These are the exact regexes in metadata/page.tine.app.yml's UpdateCheckData
  // field. If the JSON shape changes so they stop matching, F-Droid autoupdate
  // silently stops detecting new releases — catch it here.
  it("stays readable by the F-Droid UpdateCheckData regexes", () => {
    const codeMatch = /"versionCode":\s*([0-9]+)/.exec(confText);
    const verMatch = /"version":\s*"([0-9.]+)"/.exec(confText);
    expect(codeMatch?.[1]).toBe(String(conf.bundle?.android?.versionCode));
    if (conf.version.includes("-beta.")) {
      expect(verMatch).toBeNull();
      const identity = JSON.parse(readFileSync(new URL("../src-tauri/app-identity.json", import.meta.url), "utf8"));
      expect(identity.ship).toBe("experiment");
    } else {
      expect(verMatch?.[1]).toBe(conf.version);
    }
    // Retain the stable checker contract even while this checkout ships Beta.
    expect(/"version":\s*"([0-9.]+)"/.exec('{"version": "0.6.5"}')?.[1]).toBe("0.6.5");
  });
});
