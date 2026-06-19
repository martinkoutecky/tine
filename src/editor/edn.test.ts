import { describe, it, expect } from "vitest";
import { quoteEdnString, unquoteEdnString, splitTrailingMap, queryMacroExtent, queryMacroExtents } from "./edn";

describe("edn helpers", () => {
  it("quote/unquote round-trips quotes and backslashes", () => {
    for (const s of ["plain", 'a "b" c', "back\\slash", 'mix "x"\\y']) {
      expect(unquoteEdnString(quoteEdnString(s))).toBe(s);
    }
  });

  it("splits a trailing options map, ignoring braces inside strings", () => {
    expect(splitTrailingMap("(todo)")).toEqual({ form: "(todo)", opts: "" });
    expect(splitTrailingMap('(todo) {:title "A" :collapsed? true}')).toEqual({
      form: "(todo)",
      opts: '{:title "A" :collapsed? true}',
    });
    // Braces inside the title string must NOT confuse the split (the round-2 bug).
    expect(splitTrailingMap('(todo) {:title "A {B} C"}')).toEqual({
      form: "(todo)",
      opts: '{:title "A {B} C"}',
    });
    // A brace inside the form (no trailing map) → no opts.
    expect(splitTrailingMap('(todo "x}")')).toEqual({ form: '(todo "x}")', opts: "" });
    // A `}` inside a [[page ref]] in the form must NOT confuse the split.
    expect(splitTrailingMap('(page [[a}b]]) {:title "x"}')).toEqual({
      form: "(page [[a}b]])",
      opts: '{:title "x"}',
    });
    expect(splitTrailingMap("(page [[a}b]])")).toEqual({ form: "(page [[a}b]])", opts: "" });
  });

  it("finds the query macro extent, ignoring }} inside strings", () => {
    expect(queryMacroExtent("not a macro")).toBeNull();
    const simple = "{{query (todo)}}";
    expect(queryMacroExtent(simple)).toEqual({ start: 0, end: simple.length });
    // `}}` inside a :title string must NOT end the macro early (the round-3 bug).
    const tricky = '{{query (and (task TODO)) {:title "Sprint }} board"}}}';
    const e1 = queryMacroExtent(tricky)!;
    expect(tricky.slice(e1.start, e1.end)).toBe(tricky);
    // Trailing property lines after the macro are excluded (so a rewrite keeps them).
    const withProps = '{{query (todo) {:title "A"}}}\nid:: abc';
    const e2 = queryMacroExtent(withProps)!;
    expect(withProps.slice(e2.start, e2.end)).toBe('{{query (todo) {:title "A"}}}');
    // `}}` inside a [[page ref]] in the form must NOT end the macro early.
    const ref = '{{query (page [[A }} B]]) {:title "t"}}}\nid:: x';
    const e3 = queryMacroExtent(ref)!;
    expect(ref.slice(e3.start, e3.end)).toBe('{{query (page [[A }} B]]) {:title "t"}}}');
  });

  it("finds ALL query macros in a block, in order", () => {
    const raw = "A {{query (task TODO)}} B {{query (task DONE)}}";
    const exts = queryMacroExtents(raw);
    expect(exts.map((e) => raw.slice(e.start, e.end))).toEqual([
      "{{query (task TODO)}}",
      "{{query (task DONE)}}",
    ]);
    expect(queryMacroExtents("no queries here")).toEqual([]);
  });
});
