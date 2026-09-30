import { describe, it, expect } from "vitest";
import fixtures from "../../tests/fixtures/search-regex-unicode.json";
import { parseSearchQuery, matcherMatches, matchHighlights } from "./searchQuery";
import { searchFold } from "./searchFold";
import { __tineReinstantiate } from "../render/wasm/lsdoc_wasm.js";

describe("OG-R1 native/browser Unicode regex contract", () => {
  it.each(fixtures)("$query on $text", (row) => {
    const matcher = parseSearchQuery(row.query);
    expect(matcher.kind).toBe("regex");
    expect(matcherMatches(matcher, searchFold(row.text), row.text)).toBe(row.match);
    expect(matchHighlights(matcher, row.text)).toEqual(row.spans);
  });
  it("refuses a regex program above the shared 1 MiB cap", () => {
    expect(parseSearchQuery("/a{1000000}/").kind).toBe("invalid");
  });
  it("keeps a retained matcher valid after bounded cache eviction and parser recovery", () => {
    const matcher = parseSearchQuery("/\\d+/", false);
    for (let at = 0; at < 10; at++) parseSearchQuery(`/other${at}/`);
    expect(matcherMatches(matcher, "", "١٢")).toBe(true);
    __tineReinstantiate();
    expect(matcherMatches(matcher, "", "١٢")).toBe(true);
    expect(matchHighlights(matcher, "🧠 ١٢")).toEqual([{ start: 3, end: 5 }]);
  });
  it("caps evidence and preserves zero-width first hits without drawing empty highlights", () => {
    const matcher = parseSearchQuery("/^|./");
    expect(matchHighlights(matcher, "🧠 café", 0)).toEqual([]);
    expect(matchHighlights(matcher, "🧠 café", 1)).toEqual([{ start: 2, end: 3 }]);
  });
});
