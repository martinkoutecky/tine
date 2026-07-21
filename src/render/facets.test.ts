import { beforeAll, describe, it, expect } from "vitest";
import {
  seedFacets,
  facetsOf,
  clearSeededFacets,
  facetsFromDto,
  EMPTY_FACETS,
  inlineText,
  parseBody,
  stripPlanningLines,
} from "./facets";
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

  it("keeps line-leading planning facets when same-line body text follows", () => {
    const bodyText = (raw: string) =>
      stripPlanningLines(parseBody(raw, "md"), raw)
        .map((block) => ("inline" in block ? inlineText(block.inline) : ""))
        .join("\n");

    for (const [tag, field, date] of [
      ["DEADLINE", "deadline", "2026-07-30 Thu"],
      ["SCHEDULED", "scheduled", "2026-07-29 Wed"],
    ] as const) {
      const raw = `TODO x\n${tag}: <${date}>tail`;
      expect(facetsOf(raw, "md")[field]).toBe(date);
      expect(bodyText(raw)).toContain("x");
      expect(bodyText(raw)).toContain("tail");
      expect(JSON.stringify(stripPlanningLines(parseBody(raw, "md"), raw))).not.toContain(date);
      expect(raw).toBe(`TODO x\n${tag}: <${date}>tail`);
    }

    expect(facetsOf("TODO x\n  DEADLINE: <2026-07-30 Thu>tail", "md").deadline).toBe(
      "2026-07-30 Thu"
    );

    // Deliberate OG divergence: only a line-leading timestamp is planning
    // chrome; a mid-text timestamp remains ordinary body content in Tine.
    const mid = "Discuss DEADLINE: <2026-07-30 Thu> inline";
    expect(facetsOf(mid, "md").deadline).toBeNull();
    expect(JSON.stringify(stripPlanningLines(parseBody(mid, "md"), mid))).toContain('"ts":"Deadline"');
    expect(facetsOf("`DEADLINE: <2026-07-30 Thu>`", "md").deadline).toBeNull();
    expect(facetsOf("```\nDEADLINE: <2026-07-30 Thu>\n```", "md").deadline).toBeNull();
  });

  it("uses Logseq's heading property for Org blocks", () => {
    const raw = "Org heading\n:PROPERTIES:\n:heading: 3\n:END:";
    expect(facetsOf(raw, "org").headingLevel).toBe(3);
  });
});
