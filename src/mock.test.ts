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
});
