import { beforeAll, describe, expect, it } from "vitest";
import { initParser } from "../render/parse";
import { block_reference_names, linkable_property_names, page_alias_spellings } from "../render/wasm/lsdoc_wasm.js";

describe("parser-owned linkable property values", () => {
  beforeAll(initParser);
  it("splits only top-level plain tokens and keeps reference interiors", () => {
    expect(linkable_property_names("a, b，[[c, d]] #[[multi word]]", false)).toEqual(["a", "b", "c, d", "multi word"]);
  });
  it("drops empty members and deduplicates references", () => {
    expect(linkable_property_names(",a,, #a, [[b]]", false)).toEqual(["a", "b"]);
  });
  it("does not turn quoted values or code into implicit references", () => {
    expect(linkable_property_names('"#a #b"', false)).toEqual([]);
    expect(linkable_property_names("`hidden` [[shown]]", false)).toEqual(["shown"]);
  });
  it("alias labels use the same token boundaries as tags", () => {
    expect(page_alias_spellings("aliases:: [[First alias]] [[Second alias]]", undefined, false)).toEqual(["First alias", "Second alias"]);
  });
  it("reference candidates do not invent a combined name from mixed property text", () => {
    const refs = block_reference_names("tags:: plain text #a #[[multi word]]", false);
    expect(new Set(refs)).toEqual(new Set(["plain text", "a", "multi word"]));
  });
});
