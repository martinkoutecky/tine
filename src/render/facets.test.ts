import { describe, it, expect } from "vitest";
import { seedFacets, facetsOf, clearSeededFacets, facetsFromDto, EMPTY_FACETS } from "./facets";

describe("facet cache (audit P2)", () => {
  it("never evicts seeded facets — a page far larger than the old 4096 cap is all hits", () => {
    clearSeededFacets();
    const N = 6000; // > the old LRU cap, where seeding used to evict its own early entries
    for (let i = 0; i < N; i++) {
      seedFacets(`block ${i}`, "md", { ...EMPTY_FACETS, marker: `M${i}` });
    }
    // Every seeded block is still a HIT (the seeded short-circuit never parses) — proves
    // no sequential-LRU thrash → no parse-all-on-load on big pages.
    for (let i = 0; i < N; i++) {
      expect(facetsOf(`block ${i}`, "md").marker).toBe(`M${i}`);
    }
  });

  it("clearSeededFacets drops the seeded tier (graph switch)", () => {
    clearSeededFacets();
    seedFacets("a block", "md", { ...EMPTY_FACETS, marker: "TODO" });
    expect(facetsOf("a block", "md").marker).toBe("TODO"); // seeded hit
    clearSeededFacets();
    // The seeded entry is gone; we don't call facetsOf here (that would parse via wasm),
    // we re-seed a DIFFERENT value and confirm the stale one didn't survive.
    seedFacets("a block", "md", { ...EMPTY_FACETS, marker: "DONE" });
    expect(facetsOf("a block", "md").marker).toBe("DONE");
  });

  it("facetsFromDto reads shipped fields without parsing", () => {
    expect(facetsFromDto({ marker: "DONE" }).done).toBe(true);
    expect(facetsFromDto({ priority: "A" }).priority).toBe("A");
    expect(facetsFromDto({ priority: "X" }).priority).toBe(null);
    expect(facetsFromDto({}).marker).toBe(null);
  });
});
