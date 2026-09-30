import { spawnSync } from "node:child_process";
import { readFileSync, mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { expect, it } from "vitest";

it("hosted OG performance keeps its campaign anchor when master publishes a release", () => {
  const result = spawnSync(process.execPath, ["scripts/check-bench-policy.mjs", "--expected-previous", "v0.6.987"], { encoding: "utf8" });
  expect(result.status, result.stderr).toBe(0);
  expect(result.stdout).toContain("previous v0.6.5");
});

it("the campaign selector refuses a moved anchor or a stable application", () => {
  const root = mkdtempSync(join(tmpdir(), "tine-og-policy-"));
  try {
    mkdirSync(join(root, "scripts"));
    mkdirSync(join(root, "src-tauri"));
    writeFileSync(join(root, "scripts/check-bench-policy.mjs"), readFileSync("scripts/check-bench-policy.mjs"));
    const policy = JSON.parse(readFileSync("scripts/bench-policy.json", "utf8"));
    const config = JSON.parse(readFileSync("src-tauri/tauri.conf.json", "utf8"));
    const identity = JSON.parse(readFileSync("src-tauri/app-identity.json", "utf8"));
    writeFileSync(join(root, "src-tauri/app-identity.json"), JSON.stringify(identity));
    policy.previousRelease.selection = "og-campaign";
    const run = () => {
      writeFileSync(join(root, "scripts/bench-policy.json"), JSON.stringify(policy));
      writeFileSync(join(root, "src-tauri/tauri.conf.json"), JSON.stringify(config));
      return spawnSync(process.execPath, [join(root, "scripts/check-bench-policy.mjs"), "--expected-previous", "v0.6.987"], { encoding: "utf8" });
    };
    policy.previousRelease.ref = "v0.6.987";
    expect(run().status).toBe(1);
    policy.previousRelease.ref = "v0.6.5";
    config.identifier = identity.identities.release.identifier;
    expect(run().status).toBe(1);
    config.identifier = identity.identities.experiment.identifier;
    policy.previousRelease.selection = "unknown";
    expect(run().status).toBe(1);
    delete policy.previousRelease.selection;
    config.identifier = identity.identities.release.identifier;
    policy.previousRelease.ref = "v0.6.987";
    expect(run().status).toBe(0);
    policy.previousRelease.ref = "v0.6.5";
    expect(run().status).toBe(1);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
