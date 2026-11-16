import { describe, it, expect } from "vitest";
import { mockBackend } from "./mock";
import type { PageDto } from "./types";

describe("mock backend", () => {
  it("accepts the caller's exact asset cap and refuses one byte over (L-1)", async () => {
    const api = mockBackend();
    const TEST_ASSET_MAX_BYTES = 5;
    const TOO_SMALL_MAX_BYTES = TEST_ASSET_MAX_BYTES - 1;
    const name = await api.saveAsset("l1-limit.bin", new Uint8Array(TEST_ASSET_MAX_BYTES), 0);
    expect((await api.readAsset(name, TEST_ASSET_MAX_BYTES)).byteLength).toBe(TEST_ASSET_MAX_BYTES);
    await expect(api.readAsset(name, TOO_SMALL_MAX_BYTES)).rejects.toBe("asset-too-large");
  });

  it("keeps distinct page owners and scopes while search names fold", async () => {
    const pages: PageDto[] = ["Ａ", "A", "Café", "Cafe"].map((name) => ({
      name, title: name, kind: "page", pre_block: null,
      blocks: [{ id: `id-${name}`, raw: `shared needle ${name}`, collapsed: false, children: [] }],
      format: "md",
    }));
    const b = mockBackend(pages);
    for (const name of ["A", "Cafe"]) {
      const scope = { name, pageKind: "page" as const, path: `pages/${name}.md` };
      const result = await b.runGraphSearch("needle", 0, 20, undefined, false, scope);
      expect(result.hits.filter((hit) => hit.entity === "block").map((hit) => hit.page)).toEqual([name]);
      expect(result.hits.filter((hit) => hit.entity === "block").map((hit) => hit.path)).toEqual([scope.path]);
      const byName = await b.runGraphSearch("needle", 0, 20, undefined, false, { name, pageKind: "page" });
      expect(byName.hits.filter((hit) => hit.entity === "block").map((hit) => hit.page)).toEqual([name]);
    }
  });

  it("uses the mock graph's explicit accent policy for every search entry", async () => {
    const page: PageDto = { name: "Café", title: "Café", kind: "page", pre_block: null,
      blocks: [{ id: "cafe-id", raw: "café body", collapsed: false, children: [] }], format: "md" };
    const b = mockBackend([page], false);
    expect((await b.search("cafe", 20)).flatMap((group) => group.blocks).map((block) => block.id)).not.toContain("cafe-id");
    expect((await b.runGraphSearch("cafe", 20, 20)).hits.some((hit) => hit.entity === "block" && hit.block.id === "cafe-id")).toBe(false);
    expect((await b.quickSwitch("cafe", 20)).map((entry) => entry.name)).not.toContain("Café");
    expect((await b.captureQuickSwitch("cafe", 20)).map((entry) => entry.name)).not.toContain("Café");
  });
  it("block picker search matches whitespace-separated fragments in order, like OG's LIKE '%a%b%'", async () => {
    const page: PageDto = { name: "P", title: "P", kind: "page", pre_block: null,
      blocks: [{ id: "a", raw: "qzfoo qzdraft", collapsed: false, children: [] },
        { id: "b", raw: "qzready only", collapsed: false, children: [] }], format: "md" };
    const b = mockBackend([page]);
    const ids = async (q: string) => (await b.search(q, 20)).flatMap((group) => group.blocks).map((block) => block.id);
    expect(await ids("qzfo qzdr")).toEqual(["a"]);
    expect(await ids("ZFOO  QZDRA")).toEqual(["a"]);
    expect(await ids("qzdra qzfoo")).toEqual([]); // not in order, and `qzdra` is not a whole word
    expect(await ids("qzfdr")).toEqual([]);
    expect(await ids("   ")).toEqual([]);
  });

  it("block picker search adds OG's full-text leg: whole words in any order, listed before fragment-only blocks", async () => {
    const page: PageDto = { name: "P", title: "P", kind: "page", pre_block: null,
      blocks: [{ id: "a", raw: "qzfoo qzbarx tail", collapsed: false, children: [] },
        { id: "b", raw: "xx qzbar yy qzfoo", collapsed: false, children: [] },
        { id: "c", raw: "qzfoo qzdraft", collapsed: false, children: [] }], format: "md" };
    const b = mockBackend([page]);
    const ids = async (q: string, limit = 20) => (await b.search(q, limit)).flatMap((group) => group.blocks).map((block) => block.id);
    expect(await ids("qzdraft qzfoo")).toEqual(["c"]);
    expect(await ids("qzfoo qzbar")).toEqual(["b", "a"]);
    expect(await ids("qzfoo qzbar", 1)).toEqual(["b"]);
  });

  it("query (todo TODO DOING) matches both open tasks", async () => {
    const b = mockBackend();
    const groups = await b.runQuery("(todo TODO DOING)");
    const raws = groups.flatMap((g) => g.blocks.map((bl) => bl.raw));
    expect(raws.some((r) => r.startsWith("TODO ") && r.includes("Ship the M0"))).toBe(true);
    expect(raws.some((r) => r.startsWith("DOING Wire"))).toBe(true);
  });

  it("backlinks to Tine include the journal", async () => {
    const b = mockBackend();
    const groups = await b.getBacklinks("Tine");
    expect(groups.some((g) => g.page === "Jun 14th, 2026")).toBe(true);
  });

  it("query (tag name) matches hashtag references", async () => {
    const b = mockBackend();
    const groups = await b.runQuery('(tag "sheets")');
    const raws = groups.flatMap((g) => g.blocks.map((bl) => bl.raw));
    expect(raws.some((r) => r.includes("#sheets"))).toBe(true);
  });

  it("block ref counts cover bare + labeled forms", async () => {
    const b = mockBackend();
    const counts = await b.getBlockRefCounts();
    // kitchen-sink target 64b9c0e2… is referenced by a bare ref, a labeled ref,
    // AND an {{embed}} (the embed arg is a block ref too, like OG) → 3.
    expect(counts["64b9c0e2-0000-0000-0000-000000000000"]).toBe(3);
    // 58900000-0000-4000-8000-0000000000b1 is referenced once from the Jun 14th journal.
    expect(counts["58900000-0000-4000-8000-0000000000b1"]).toBe(1);
  });

  it("block referrers list the referencing blocks (same page included)", async () => {
    const b = mockBackend();
    const groups = await b.getBlockReferrers("64b9c0e2-0000-0000-0000-000000000000");
    const raws = groups.flatMap((g) => g.blocks.map((bl) => bl.raw));
    expect(raws.some((r) => r.includes("Block reference (bare)"))).toBe(true);
    expect(raws.some((r) => r.includes("Labeled block reference"))).toBe(true);
  });
});
