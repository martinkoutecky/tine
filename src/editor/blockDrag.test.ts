import { describe, expect, it } from "vitest";
import { blockDropPosition } from "./blockDrag";
import { readFileSync } from "node:fs";

it("the Guide explains moving query results and the single-block Alt reference gesture", () => {
  const guide = readFileSync("crates/tine-core/src/templates/queries.md", "utf8");
  for (const detail of ["List result's bullet", "**Alt**", "leaving the source in place", "only after the source's ID has been saved", "several blocks still moves together"]) expect(guide).toContain(detail);
});

describe("blockDropPosition (OG outline drag parity, GH #326)", () => {
  const block = { left: 100, top: 20, height: 30 };
  const main = { left: 120, top: 20, height: 30 };

  it("uses the shallow target strip for sibling insertion", () => {
    expect(blockDropPosition(135, 25, block, main)).toBe("before");
    expect(blockDropPosition(135, 45, block, main)).toBe("after");
  });

  it("uses OG's deeper-than-50px target zone for child insertion", () => {
    expect(blockDropPosition(151, 22, block, main)).toBe("child");
    expect(blockDropPosition(190, 48, block, main)).toBe("child");
  });
});
