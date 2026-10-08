import { execFileSync } from "node:child_process";
import { describe, it } from "vitest";

describe("OG-R6 preview release boundary", () => {
  it("validates versions, routes candidate assets, and rejects stable publication", () => {
    execFileSync(process.execPath, ["scripts/test-beta-release.mjs"], { stdio: "pipe" });
  });
});

it("accepts the Beta version suffix at the release policy entry point", () => {
  execFileSync(process.execPath, ["--input-type=module", "-e", 'import { releaseVersion } from "./scripts/release-policy.mjs"; releaseVersion("0.7.0-beta.1");'], { stdio: "pipe" });
});
