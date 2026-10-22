import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

it("the troubleshooting Guide explains reproducing a typing freeze before exporting memory-only stalls", () => {
  const guide = readFileSync("crates/tine-core/src/templates/troubleshooting-recovery.md", "utf8");
  for (const meaning of ["reproduce it before creating the report", "worst renderer stalls while typing", "stay in memory", "no typed text or key values"]) expect(guide).toContain(meaning);
});
