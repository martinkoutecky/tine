import { expect, it, vi } from "vitest";
import { blockRegions, initParser } from "./render/parse";

it("the hosted Big fixture carries the same identity absence as native parser projections", async () => {
  await initParser();
  vi.stubGlobal("location", { search: "?big" });
  try {
    const { mockBackend } = await import("./mock");
    const page = await mockBackend().getPage("Big", "page");
    expect(page?.blocks).toHaveLength(2000);
    for (const block of page!.blocks) {
      expect(block.has_id).toBe(blockRegions(block.raw, "md").id !== null);
    }
  } finally { vi.unstubAllGlobals(); }
});
