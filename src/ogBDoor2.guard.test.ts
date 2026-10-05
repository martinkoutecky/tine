import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const read = (path: string) => readFileSync(new URL(`../${path}`, import.meta.url), "utf8");

describe("OG-B-DOOR2 shared-answer boundaries", () => {
  it("keeps page properties and icons behind parser-owned regions (I-12)", () => {
    // page_property_lines lives in tine-core (one pure answer); the store's
    // page_properties.rs re-exports it and adds page facets.
    const coreProps = read("crates/tine-core/src/page_properties.rs");
    const props = read("crates/tine-store/src/query/page_properties.rs");
    const icons = read("crates/tine-store/src/model/page_icons.rs");
    expect(coreProps, "I-12: imitate block_regions::parse_document; no page-property raw selector").toContain("block_regions::parse_document");
    expect(props, "I-12: the store asks tine_core::page_properties, it does not re-derive").toContain("tine_core::page_properties::");
    expect(icons, "I-12: icons use page_property_lines with the actual file format").toContain("page_property_lines");
    for (const source of [coreProps, props, icons]) expect(source).not.toMatch(/\.lines\(|parse_property_line|strip_prefix\(/);
    expect(read("crates/tine-store/src/query.rs")).not.toContain("fn org_property_line");
    expect(read("crates/tine-core/src/reference_evidence.rs")).not.toContain("fn property_values");
    const regions = read("crates/tine-core/src/block_regions.rs");
    expect(regions, "I-15: imitate the accepted-key map; do not search a folded property list per entry").not.toContain("props.iter().find");
    expect(regions, "I-15: whole-file literal exclusion uses exclude_literals' monotone cursor").toContain("exclude_literals(&mut result.properties");
  });

  it("uses the same format-aware literal door for VCS detection and resolution (I-12)", () => {
    const source = read("crates/tine-core/src/concord_queue.rs");
    expect(source, "I-12: literal ownership comes from block_regions::parse_document").toContain("block_regions::parse_document");
    expect(source).not.toContain("let mut fence");
    for (const name of ["vcs_conflict_markers", "scan_vcs_conflict_markers", "parse_vcs_marker_sides"])
      expect(source).toMatch(new RegExp(`pub fn ${name}\\([^)]*format: Format`));
  });

  it("constructs n-ary SQL booleans before visits and bounds actual nesting (I-22)", () => {
    const source = read("crates/tine-core/src/query/tql.rs");
    expect(source, "I-22: imitate tql/boolean.rs; flat chains are not recursive binary trees").toContain("boolean::BooleanDialect");
    expect(source.indexOf("boolean::admit_tokens")).toBeLessThan(source.indexOf("parser.parse_expr()"));
    expect(source).toContain("boolean::admit_ast(&expr)");
    expect(source).not.toMatch(/fn (?:and|or)_items/);
    expect(read("crates/tine-core/src/query/tql/boolean.rs")).toContain("parser.parse_subexpr(precedence)");
  });

  it("uses Rust property separators and one grapheme-safe plain matcher (I-4/I-12)", () => {
    expect(read("src/render/pageRefs.ts")).toContain("splitLinkableProperty(value)");
    expect(read("src/render/block.ts")).not.toContain(".split(/[,，]/)");
    expect(read("src/mock.ts")).not.toContain(".split(/[,，]/)");
    const source = read("crates/tine-core/src/reference_evidence/plain_match.rs");
    expect(source, "I-4: imitate visit_plain_matches; never end a match inside a grapheme").toContain("end == boundary");
    expect(source).toContain(".grapheme_indices(true)");
    expect(source.match(/fn visit_plain_matches\(/g)).toHaveLength(1);
  });
});
