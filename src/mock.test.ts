import { describe, it, expect } from "vitest";
import { mockBackend } from "./mock";

describe("mock backend", () => {
  it("query (todo TODO DOING) matches both open tasks", async () => {
    const b = mockBackend();
    const groups = await b.runQuery("(todo TODO DOING)");
    const raws = groups.flatMap((g) => g.blocks.map((bl) => bl.raw));
    expect(raws.some((r) => r.startsWith("TODO ") && r.includes("Ship the M0"))).toBe(true);
    expect(raws.some((r) => r.startsWith("DOING Wire"))).toBe(true);
  });

  it("backlinks to logseq-claude include the journal", async () => {
    const b = mockBackend();
    const groups = await b.getBacklinks("logseq-claude");
    expect(groups.some((g) => g.page === "Jun 14th, 2026")).toBe(true);
  });

  it("block ref counts cover bare + labeled forms", async () => {
    const b = mockBackend();
    const counts = await b.getBlockRefCounts();
    // kitchen-sink target 64b9c0e2… is referenced by a bare ref, a labeled ref,
    // AND an {{embed}} (the embed arg is a block ref too, like OG) → 3.
    expect(counts["64b9c0e2-0000-0000-0000-000000000000"]).toBe(3);
    // arch-1 is referenced once from the Jun 14th journal.
    expect(counts["arch-1"]).toBe(1);
  });

  it("block referrers list the referencing blocks (same page included)", async () => {
    const b = mockBackend();
    const groups = await b.getBlockReferrers("64b9c0e2-0000-0000-0000-000000000000");
    const raws = groups.flatMap((g) => g.blocks.map((bl) => bl.raw));
    expect(raws.some((r) => r.includes("Block reference (bare)"))).toBe(true);
    expect(raws.some((r) => r.includes("Labeled block reference"))).toBe(true);
  });
});
