import { describe, it, expect } from "vitest";
import { SEARCH_SYNTAX, canonicalFold, parseSearchQuery, matcherMatches, simpleTerm, matchHighlight, matchHighlights, friendlySearchToDsl, friendlySearchToSavedDsl, savedDslToFriendlySearch } from "./searchQuery";

// Mirrors crates/tine-core/src/search_query.rs tests — keep the two in sync.
const hit = (q: string, text: string) =>
  matcherMatches(parseSearchQuery(q), canonicalFold(text), text);

describe("searchQuery parser (#44)", () => {
  it("executes every example displayed by Ctrl K syntax help", () => {
    for (const rule of SEARCH_SYNTAX) {
      expect(hit(rule.example, rule.match), rule.example).toBe(true);
      expect(hit(rule.example, rule.miss), rule.example).toBe(false);
    }
  });
  it("single bare term is simple", () => {
    expect(simpleTerm(parseSearchQuery("hello"))).toBe("hello");
    expect(hit("hello", "well HELLO there")).toBe(true);
    expect(hit("hello", "goodbye")).toBe(false);
  });

  it("whitespace is order-independent AND", () => {
    expect(simpleTerm(parseSearchQuery("foo bar"))).toBeNull();
    expect(hit("foo bar", "bar then foo")).toBe(true);
    expect(hit("foo bar", "only foo")).toBe(false);
  });

  it("OR keyword splits groups; AND binds tighter", () => {
    expect(hit("cat OR dog", "i have a dog")).toBe(true);
    expect(hit("cat OR dog", "i have a fish")).toBe(false);
    expect(hit("apple pie OR cake", "cake")).toBe(true);
    expect(hit("apple pie OR cake", "apple pie")).toBe(true);
    expect(hit("apple pie OR cake", "apple tart")).toBe(false);
  });

  it("negation excludes; pure negation matches nothing", () => {
    expect(hit("foo -bar", "foo only")).toBe(true);
    expect(hit("foo -bar", "foo and bar")).toBe(false);
    expect(parseSearchQuery("-bar").kind).toBe("empty");
    expect(hit("-bar", "anything")).toBe(false);
  });

  it("quoted phrase is contiguous and not simple", () => {
    expect(hit('"foo bar"', "a foo bar b")).toBe(true);
    expect(hit('"foo bar"', "foo x bar")).toBe(false);
    expect(simpleTerm(parseSearchQuery('"foo"'))).toBeNull();
    expect(hit('keep -"foo bar"', "keep foo x bar")).toBe(true);
    expect(hit('-"foo bar"', "foo bar here")).toBe(false);
  });

  it("whole-query regex is case-sensitive", () => {
    expect(hit("/[A-Z]{3}/", "abc ABC def")).toBe(true);
    expect(hit("/[A-Z]{3}/", "abc def")).toBe(false);
    expect(hit("/^start/", "start of line")).toBe(true);
    expect(hit("/^start/", "not at start")).toBe(false);
  });

  it("invalid regex reports an error and matches nothing", () => {
    const m = parseSearchQuery("/(unclosed/");
    expect(m.kind).toBe("invalid");
    expect(hit("/(unclosed/", "(unclosed")).toBe(false);
  });

  it("`//` is a literal term, not a regex", () => {
    expect(parseSearchQuery("//").kind).toBe("boolean");
    expect(hit("//", "a // b")).toBe(true);
  });

  it("highlight picks the earliest positive term / regex match", () => {
    expect(matchHighlight(parseSearchQuery("bar"), "foo bar baz")).toEqual({ start: 4, len: 3 });
    // earliest of two AND terms
    expect(matchHighlight(parseSearchQuery("baz foo"), "foo bar baz")).toEqual({ start: 0, len: 3 });
    expect(matchHighlight(parseSearchQuery("/b.z/"), "foo bar baz")).toEqual({ start: 8, len: 3 });
    // negated terms are never highlighted
    expect(matchHighlight(parseSearchQuery("foo -bar"), "foo bar")).toEqual({ start: 0, len: 3 });
  });

  it("mock presentation evidence includes every positive term and repeated regex hit", () => {
    expect(matchHighlights(parseSearchQuery("alpha beta -draft"), "beta alpha alpha")).toEqual([
      { start: 0, end: 4 },
      { start: 5, end: 10 },
      { start: 11, end: 16 },
    ]);
    expect(matchHighlights(parseSearchQuery("/a./"), "ab ac")).toEqual([
      { start: 0, end: 2 },
      { start: 3, end: 5 },
    ]);
  });

  it("matches canonical Unicode forms and maps highlights to original UTF-16 spans", () => {
    expect(hit("Café", "Cafe\u0301")).toBe(true);
    expect(hit("Cafe\u0301", "Café")).toBe(true);
    expect(hit("가", "\u1100\u1161")).toBe(true);
    expect(hit("i\u0307", "İ")).toBe(true);
    expect(hit("cafe", "café")).toBe(false);
    expect(hit("/Café/", "Cafe\u0301")).toBe(false);
    expect(matchHighlight(parseSearchQuery("Résumé"), "\u{1F9E0} Re\u0301sume\u0301"))
      .toEqual({ start: 3, len: 8 });
    expect(matchHighlights(parseSearchQuery("è\u0315"), "e\u0315\u0300"))
      .toEqual([{ start: 0, end: 3 }]);
    expect(matchHighlights(parseSearchQuery("i\u0307"), "İ"))
      .toEqual([{ start: 0, end: 1 }]);
  });

  it("compiles friendly search to ordinary query DSL and preserves saved source losslessly", () => {
    expect(friendlySearchToDsl('foo -draft OR "exact phrase"')).toEqual({
      dsl: '(or (and "foo" (not "draft")) "exact phrase")',
      error: null,
    });
    expect(friendlySearchToDsl("/[A-Z]{3}/")).toEqual({
      dsl: '(content-regex "[A-Z]{3}")',
      error: null,
    });
    expect(friendlySearchToSavedDsl('foo "bar"')).toBe('(search "foo \\"bar\\"")');
    expect(savedDslToFriendlySearch('(search "foo \\"bar\\"")')).toBe('foo "bar"');
    expect(savedDslToFriendlySearch('(and "foo" "bar")')).toBeNull();
  });
});
