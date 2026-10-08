import { execFileSync, spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
import { releaseTag } from "../scripts/release-policy.mjs";

it("preflights the OG v0.6.5 performance baseline, including the release tag context", () => {
  const tag = releaseTag(JSON.parse(readFileSync("src-tauri/tauri.conf.json", "utf8")));
  for (const ref of [undefined, `refs/tags/${tag}`]) {
    const result = spawnSync(process.execPath, ["scripts/check-release-preflight.mjs"], {
      encoding: "utf8", env: { ...process.env, GITHUB_REF: ref },
    });
    expect(result.status, result.stderr).toBe(0);
  }
  expect(JSON.parse(execFileSync("git", ["show", "6e0e69e34:src-tauri/tauri.conf.json"], { encoding: "utf8" })).version).toBe("0.6.5");
});

it("keeps every inherited performance budget and the immutable anchor", () => {
  const before = JSON.parse(execFileSync("git", ["show", "224aa18e4:scripts/bench-policy.json"], { encoding: "utf8" }));
  const after = JSON.parse(readFileSync("scripts/bench-policy.json", "utf8"));
  delete before.previousRelease;
  delete after.previousRelease;
  // The only added term: scrollBig's one-frame ADR 0072 allowance (Martin
  // 2026-10-05). Anchors and percentage budgets stay exactly as inherited.
  expect(after.metrics.scrollBig.allowanceMs).toBe(17);
  delete after.metrics.scrollBig.allowanceMs;
  delete after.metrics.scrollBig.allowanceReason;
  expect(after).toEqual(before);
});
