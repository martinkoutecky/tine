import { describe, it, expect } from "vitest";
import { quoteEdnString, unquoteEdnString, splitTrailingMap } from "./edn";

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
  });
});
