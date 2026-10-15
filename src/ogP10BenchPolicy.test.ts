import { spawnSync } from "node:child_process";
import { readFileSync, mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { expect, it } from "vitest";

// The immutable anchor is pinned at v0.7.2 (Martin 2026-10-13); the previous
// one follows GitHub's latest published release (v0.7.3 since 2026-10-15).
it("the benchmark policy pins v0.7.2 and follows the latest published release", () => {
  const result = spawnSync(process.execPath, ["scripts/check-bench-policy.mjs", "--expected-previous", "v0.7.3"], { encoding: "utf8" });
  expect(result.status, result.stderr).toBe(0);
  expect(result.stdout).toContain("immutable v0.7.2, previous v0.7.3");
});

it("the policy checker refuses a stale previous anchor, a moved immutable anchor, and unbounded reruns", () => {
  const root = mkdtempSync(join(tmpdir(), "tine-bench-policy-"));
  try {
    mkdirSync(join(root, "scripts"));
    mkdirSync(join(root, "src-tauri"));
    writeFileSync(join(root, "scripts/check-bench-policy.mjs"), readFileSync("scripts/check-bench-policy.mjs"));
    writeFileSync(join(root, "src-tauri/tauri.conf.json"), readFileSync("src-tauri/tauri.conf.json"));
    const policy = JSON.parse(readFileSync("scripts/bench-policy.json", "utf8"));
    const run = (latest = "v0.7.3") => {
      writeFileSync(join(root, "scripts/bench-policy.json"), JSON.stringify(policy));
      return spawnSync(process.execPath, [join(root, "scripts/check-bench-policy.mjs"), "--expected-previous", latest], { encoding: "utf8" });
    };
    expect(run().status).toBe(0);
    // A newer stable release makes the previous anchor stale.
    expect(run("v0.8.0").status).toBe(1);
    policy.immutableBaseline.ref = "v0.4.7";
    expect(run().status).toBe(1);
    policy.immutableBaseline.ref = "v0.7.2";
    policy.reliability.maxDisturbedRoundReruns = 2;
    expect(run().status).toBe(1);
    policy.reliability.maxDisturbedRoundReruns = 1;
    policy.previousRelease.selection = "og-campaign";
    expect(run().status).toBe(1);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
