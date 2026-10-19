import { describe, expect, it } from "vitest";
import { blockPickerLeg, blockPickerWordGroups } from "./blockPickerMatch";

// OG's `((` search (electron/search.cljs search-blocks) lists FTS5 matches
// (whole words, any order) before `LIKE '%a%b%'` matches (literal pieces in
// order). Inputs here are already search-folded, as both backends fold them.
describe("blockPickerLeg", () => {
  it("full-text leg: whole words in any order", () => {
    expect(blockPickerLeg("foo draft", "draft foo")).toBe("words");
    expect(blockPickerLeg("foo draft", "foo draft")).toBe("words");
    expect(blockPickerLeg("a b c", "c a")).toBe("words");
  });
  it("words are whole tokens, not substrings", () => {
    expect(blockPickerLeg("foo draft", "dra foo")).toBeNull();
    expect(blockPickerLeg("foo draft", "fo dra")).toBe("fragments");
    expect(blockPickerLeg("foo draft", "draft ready")).toBeNull();
  });
  it("a fragment-only block is the second leg", () => {
    expect(blockPickerLeg("foobar tail", "foo bar")).toBe("fragments");
    expect(blockPickerLeg("xx bar yy foo", "foo bar")).toBe("words");
  });
  it("punctuation makes the FTS5 bareword query a syntax error: OG falls back to one adjacent phrase", () => {
    expect(blockPickerWordGroups("foo-bar draft")).toEqual([["foo", "bar", "draft"]]);
    expect(blockPickerWordGroups("foo_bar draft")).toEqual([["foo", "bar"], ["draft"]]);
    expect(blockPickerLeg("foo bar draft", "foo-bar draft")).toBe("words");
    expect(blockPickerLeg("draft foo-bar", "foo-bar draft")).toBeNull();
  });
  it("blank input matches nothing", () => {
    expect(blockPickerLeg("anything", "")).toBeNull();
    expect(blockPickerLeg("anything", "  ")).toBeNull();
  });
});
