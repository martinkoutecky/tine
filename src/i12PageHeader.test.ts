import { readFileSync } from "node:fs";
import { beforeAll, beforeEach, describe, expect, it } from "vitest";
import { initParser } from "./render/parse";
import { resetStore, setRaw } from "./document";
import { doc } from "./document/model";
import { loadSingle } from "./document/workingSet";
import { BLOCK_SCOPED_PROPERTY_KEYS, pageToDto } from "./document/convert";
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

  it("documents the two deliberate JS-side normalisations the shared golden excludes", () => {
    // Enter leaves trailing newlines in the live header editor, and CRLF is only
    // normalised by the Rust save. JS folds a flagless "tags:: books\n" root into
    // pre_block before the DTO leaves the browser; Rust alone would keep the
    // trailing-newline root as a bullet. The golden therefore holds neither shape.
    resetStore();
    loadSingle({ name: "NL", kind: "page", title: "NL", pre_block: null, format: "md",
      blocks: [{ id: "b1", raw: "tags:: books\n", collapsed: false, children: [] }] } as PageDto);
    const dto = pageToDto("NL")!;
    expect({ pre_block: dto.pre_block ?? null, blocks: dto.blocks.map((b) => b.raw) }).toEqual({ pre_block: "tags:: books", blocks: [] });
  });

  it("fails a planted divergent header answer", () => {
    const expected = { pre_block: "tags:: books", blocks: [] };
    const planted = { pre_block: null, blocks: ["tags:: books"] };
    expect(() => assertDifferential(expected, planted)).toThrow(/I-12:.*exemplar tine_core::model/s);
  });

  it("keeps the block-scoped property list identical to the Rust promotion rule (GH #540)", () => {
    const rust = readFileSync("crates/tine-store/src/model.rs", "utf8");
    const body = /BLOCK_SCOPED_PROPERTY_KEYS: &\[&str\] = &\[([^\]]*)\]/.exec(rust)?.[1] ?? "";
    const rustKeys = [...body.matchAll(/"([^"]+)"/g)].map((m) => m[1]);
    expect(rustKeys.length).toBeGreaterThan(0);
    expect([...BLOCK_SCOPED_PROPERTY_KEYS]).toEqual(rustKeys);
  });

  it("keeps an empty numbered first bullet a list item once its text is typed (GH #540)", () => {
    loadSingle({ name: "Dosa", kind: "page", title: "Dosa", pre_block: null, format: "md",
      blocks: [{ id: "b1", raw: "logseq.order-list-type:: number", collapsed: false, children: [] }] } as PageDto);
    expect(pageToDto("Dosa")!.pre_block ?? null).toBeNull();
    setRaw(doc.pages[0].roots[0], "Dosa\nlogseq.order-list-type:: number");
    const dto = pageToDto("Dosa")!;
    expect(dto.pre_block ?? null).toBeNull();
    expect(dto.blocks.map((b) => b.raw)).toEqual(["Dosa\nlogseq.order-list-type:: number"]);
  });
});
