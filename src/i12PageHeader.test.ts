import { readFileSync } from "node:fs";
import { beforeAll, beforeEach, describe, expect, it } from "vitest";
import { initParser } from "./render/parse";
import { loadSingle, pageToDto, resetStore } from "./document";
import type { PageDto } from "./types";

interface GoldenCase {
  input: PageDto;
  expected: { pre_block: string | null; blocks: string[] };
}

const cases = (JSON.parse(readFileSync("tests/fixtures/i12-page-header-golden.json", "utf8")) as { cases: GoldenCase[] }).cases;

function assertDifferential(expected: unknown, js: unknown): void {
  try { expect(js).toEqual(expected); }
  catch { throw new Error(`I-12: JS pageToDto must match the shared Rust save golden; exemplar tine_core::model::first_root_is_promotable_page_header. Expected=${JSON.stringify(expected)} JS=${JSON.stringify(js)}`); }
}

describe("I-12 JS and Rust page-header save boundary", () => {
  beforeAll(async () => { await initParser(); });
  beforeEach(() => resetStore());

  it("matches the shared Rust save golden for page-header fixtures", () => {
    expect(cases.length, "I-12: page-header golden needs save fixtures").toBeGreaterThan(0);
    for (const { input, expected } of cases) {
      resetStore();
      loadSingle(input);
      const dto = pageToDto(input.name)!;
      assertDifferential(expected, { pre_block: dto.pre_block ?? null, blocks: dto.blocks.map((block) => block.raw) });
    }
  });

  it("fails a planted divergent header answer", () => {
    const expected = { pre_block: "tags:: books", blocks: [] };
    const planted = { pre_block: null, blocks: ["tags:: books"] };
    expect(() => assertDifferential(expected, planted)).toThrow(/I-12:.*exemplar tine_core::model/s);
  });
});
