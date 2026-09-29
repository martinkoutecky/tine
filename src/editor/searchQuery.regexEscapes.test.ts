// Regex class escapes and the shared Unicode subset (master f02af3eff6; og-G2 row 26).
import { describe, it, expect } from "vitest";
import { parseSearchQuery, matcherMatches } from "./searchQuery";
import { searchFold } from "./searchFold";
const hit = (q: string, text: string) => matcherMatches(parseSearchQuery(q), searchFold(text), text);

describe("og-G2 f02af3ef: shared Unicode regex subset (master test)", () => {
  it("uses the shared Unicode regex subset", () => {
    expect(hit("/\\p{L}+/", "café")).toBe(true);
    expect(hit("/\\p{L}+/", "123")).toBe(false);
    expect(hit("/[(?]+/", "(?")).toBe(true);
    for (const query of ["/foo(?=bar)/", "/(a)\\1/", "/(?i)abc/"]) expect(parseSearchQuery(query).kind, query).toBe("invalid");
  });
  it("master accepts class escapes \\d \\w \\s \\b as regex", () => {
    for (const query of ["/\\d{3}/", "/\\w+/", "/a\\sb/", "/\\bfoo/"]) expect(parseSearchQuery(query).kind, query).toBe("regex");
  });
  it("unicode whitespace splits terms (NBSP/ideographic space)", () => {
    expect(hit("foo　bar", "bar then foo")).toBe(true);
    expect(hit("﻿foo﻿", "FOO")).toBe(true);
  });
});
