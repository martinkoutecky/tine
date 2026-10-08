import { describe, expect, it } from "vitest";
import type { BlockDto } from "../types";
import { capBlockTree } from "./PeekPopup";

describe("PeekPopup imported tree bound", () => {
  it("caps a deep tree without recursing, while keeping an ordinary tree intact", () => {
    const ordinary: BlockDto = { id: "ordinary", raw: "ordinary", collapsed: false, children: [] };
    expect(capBlockTree([ordinary], 10)).toEqual({ blocks: [ordinary], truncated: 0 });
    let realistic = ordinary;
    for (let i = 0; i < 63; i++) realistic = { id: `realistic-${i}`, raw: "nested", collapsed: false, children: [realistic] };
    expect(capBlockTree([realistic], 100).truncated).toBe(0);
    let deep = ordinary;
    for (let i = 0; i < 15_000; i++) {
      deep = { id: String(i), raw: "nested", collapsed: false, children: [deep] };
    }
    const capped = capBlockTree([deep], 10);
    expect(capped.truncated).toBeGreaterThan(0);
    expect(capped.blocks).toHaveLength(1);
  });
  it("I-22: stops counting a very broad truncated page", () => {
    const blocks: BlockDto[] = Array.from({ length: 10_000 }, (_, i) =>
      ({ id: String(i), raw: "item", collapsed: false, children: [] }));
    const capped = capBlockTree(blocks, 10);
    expect(capped.blocks).toHaveLength(10);
    expect(capped.truncated, "I-22: capBlockTree must bound visits after the excerpt cap")
      .toBeLessThanOrEqual(2_001);
  });
});
