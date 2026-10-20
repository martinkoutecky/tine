import { execFileSync, spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
import { releaseTag } from "../scripts/release-policy.mjs";

it("preflights the performance baseline policy, including the release tag context", () => {
  const tauri = JSON.parse(readFileSync("src-tauri/tauri.conf.json", "utf8"));
  const tag = releaseTag(tauri);
  const run = (env: Record<string, string | undefined>) => spawnSync(process.execPath, ["scripts/check-release-preflight.mjs"], {
    encoding: "utf8", env: { ...process.env, TINE_PREFLIGHT_DEVELOPMENT: undefined, ...env },
  });
  // Off a tag, a development tree passes: og carries the next, undated version
  // between cuts (D-20261020-BZ7).
  const dev = run({ GITHUB_REF: undefined, TINE_PREFLIGHT_DEVELOPMENT: "1" });
  expect(dev.status, dev.stderr).toBe(0);
  // A tag never takes the development allowance: it passes exactly when the
  // version has its dated heading, and otherwise fails on that one problem.
  const released = new RegExp(`^## \\[${tauri.version.replaceAll(".", "\\.")}\\] - \\d{4}-\\d{2}-\\d{2}$`, "m")
    .test(readFileSync("CHANGELOG.md", "utf8"));
  const atTag = run({ GITHUB_REF: `refs/tags/${tag}`, TINE_PREFLIGHT_DEVELOPMENT: "1" });
  if (released) {
    expect(atTag.status, atTag.stderr).toBe(0);
  } else {
    expect(atTag.status).toBe(1);
    expect(atTag.stderr).toContain(`CHANGELOG.md has no dated [${tauri.version}] release heading`);
    expect(atTag.stderr).toContain("(1 problem(s))");
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
