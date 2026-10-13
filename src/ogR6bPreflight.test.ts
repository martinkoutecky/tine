import { execFileSync, spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
import { releaseTag } from "../scripts/release-policy.mjs";

it("preflights the performance baseline policy, including the release tag context", () => {
  const tag = releaseTag(JSON.parse(readFileSync("src-tauri/tauri.conf.json", "utf8")));
  for (const ref of [undefined, `refs/tags/${tag}`]) {
    const result = spawnSync(process.execPath, ["scripts/check-release-preflight.mjs"], {
      encoding: "utf8", env: { ...process.env, GITHUB_REF: ref },
    });
    expect(result.status, result.stderr).toBe(0);
  }
  expect(JSON.parse(execFileSync("git", ["show", "6e0e69e34:src-tauri/tauri.conf.json"], { encoding: "utf8" })).version).toBe("0.6.5");
});

it("keeps every inherited performance budget; only Martin's anchor move and the bounded rerun differ", () => {
  const before = JSON.parse(execFileSync("git", ["show", "224aa18e4:scripts/bench-policy.json"], { encoding: "utf8" }));
  const after = JSON.parse(readFileSync("scripts/bench-policy.json", "utf8"));
  delete before.previousRelease;
  delete after.previousRelease;
  // Martin 2026-10-13: the immutable anchor moved v0.4.7 -> v0.7.2, one
  // disturbed round may be re-measured, and the ADR 0072 allowance went with
  // the pre-windowing anchors. Every percentage budget stays as inherited.
  expect(after.immutableBaseline.ref).toBe("v0.7.2");
  expect(after.reliability.maxDisturbedRoundReruns).toBe(1);
  expect(after.metrics.scrollBig.allowanceMs).toBeUndefined();
  delete before.immutableBaseline;
  delete after.immutableBaseline;
  delete after.reliability.maxDisturbedRoundReruns;
  expect(after).toEqual(before);
});
