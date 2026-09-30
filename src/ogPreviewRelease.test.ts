import { execFileSync } from "node:child_process";
import { describe, it } from "vitest";

describe("OG-R6 preview release boundary", () => {
  it("validates versions, routes candidate assets, and rejects stable publication", () => {
    execFileSync(process.execPath, ["scripts/test-og-preview-release.mjs"], { stdio: "pipe" });
  });
});
