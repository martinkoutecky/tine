// Checkpoint-5 packet R (I-12): `facetsOf` (the app, from one wasm parse) and the Rust block
// projection answer the same header facts. The expectations in the shared fixture are written by
// Rust (crates/tine-core/tests/og_r_facets_fixture.rs); Rust owns them, this half proves the TS twin.
import { beforeAll, describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { initParser } from "./parse";
import { clearSeededFacets, facetsOf } from "./facets";
import type { Format } from "./ast";

interface Case {
  format: Format;
  raw: string;
  expected: {
    marker: string | null;
    priority: string | null;
    heading: number | null;
    scheduled: string | null;
    deadline: string | null;
    tags: string[];
    properties: [string, string][];
  };
}
const cases: Case[] = JSON.parse(
  readFileSync(new URL("../../crates/tine-core/tests/fixtures/block-facets.json", import.meta.url), "utf8"),
);

describe("facetsOf agrees with the Rust projection on the shared fixture", () => {
  beforeAll(async () => {
    await initParser();
    clearSeededFacets();
  });
  for (const c of cases) {
    it(JSON.stringify(c.raw).slice(0, 70), () => {
      const f = facetsOf(c.raw, c.format);
      expect({
        marker: f.marker,
        priority: f.priority,
        heading: f.headingLevel,
        scheduled: f.scheduled,
        deadline: f.deadline,
        tags: f.tags,
        properties: f.properties,
      }).toEqual(c.expected);
    });
  }
});
