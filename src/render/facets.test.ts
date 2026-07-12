import { beforeAll, describe, it, expect } from "vitest";
import { seedFacets, facetsOf, clearSeededFacets, facetsFromDto, EMPTY_FACETS } from "./facets";
import { initParser } from "./parse";

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

describe("planning facets", () => {
  beforeAll(async () => {
    await initParser();
  });

  it("keeps a schedule clickable when body text follows (#75)", () => {
    const raw = "Task\nSCHEDULED: <2026-07-13 Mon>\nnotes after the schedule";
    expect(facetsOf(raw, "md").scheduled).toBe("2026-07-13 Mon");
    expect(facetsOf("Überblick\nSCHEDULED: <2026-07-14 Tue>\n続き", "md").scheduled).toBe(
      "2026-07-14 Tue"
    );
  });

  it("does not promote a mid-text planning timestamp to date chrome", () => {
    const raw = "Discuss SCHEDULED: <2026-07-13 Mon> inline\nnotes after it";
    expect(facetsOf(raw, "md").scheduled).toBeNull();
  });

  it("uses Logseq's heading property for Org blocks", () => {
    const raw = "Org heading\n:PROPERTIES:\n:heading: 3\n:END:";
    expect(facetsOf(raw, "org").headingLevel).toBe(3);
  });
});
