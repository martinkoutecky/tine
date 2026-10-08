import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
const source = (path: string) => readFileSync(path, "utf8");
describe("OG-DUPBL1 one answerer", () => {
  it("I-12: ordered glyphs belong to tine-core ordinal, with a thin wasm adapter", () => {
    expect(source("src/document/edits/properties.ts")).toContain("ordered_list_glyph(idx, depth)");
    expect(source("src/document/edits/properties.ts")).not.toMatch(/function toLetters|function toRoman/);
    expect(source("crates/tine-graph-features/src/render_facets.rs")).toContain("tine_core::ordinal::glyph(");
    expect(source("crates/tine-graph-features/src/render_facets.rs")).not.toMatch(/fn letters|fn roman/);
    expect(source("crates/lsdoc-wasm/src/lib.rs")).toContain('tine-core/src/ordinal.rs');
  });
  it("I-4/I-12: PDF identity sanitization belongs to tine-core pdf_key, never a frontend twin", () => {
    expect(source("src/pdf.ts")).toContain("pdf_asset_key(filename, true)");
    expect(source("src/pdf.ts")).not.toMatch(/function sanitize|codePointAt\(|replace\(/);
    expect(source("crates/tine-core/src/pdf.rs")).toContain("crate::pdf_key::asset_key(");
    expect(source("crates/tine-core/src/pdf.rs")).not.toContain("fn sanitize_filename");
    expect(source("crates/lsdoc-wasm/src/lib.rs")).toContain('tine-core/src/pdf_key.rs');
  });
  it("I-4/I-12: favorites arrangement metadata uses the document page-property door", () => {
    expect(source("src/favorites.ts")).toContain("pageHeaderProperties(disk)");
    expect(source("src/favorites.ts")).toContain('await import("./document")');
    expect(source("src/favorites.ts")).not.toContain("pagePropertyEntries");
    expect(source("src/favorites.ts")).not.toContain("IS_ARRANGEMENT_PAGE");
  });

});
