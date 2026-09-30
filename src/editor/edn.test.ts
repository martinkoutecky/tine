import { beforeAll, describe, it, expect } from "vitest";
import { quoteEdnString, unquoteEdnString, splitTrailingMap, editEdnTitle, readEdnOptions } from "./edn";

import { initParser } from "../render/parse";
beforeAll(initParser);

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
});


it("the shipped wasm uses discard-aware title spans and real EDN strings", () => {
  const source = '{:x #_ :title [:title "keep"] :title #_ "discard" "é\\n"}';
  expect(readEdnOptions(source)?.title).toBe("é\n");
  expect(editEdnTitle(source, '😀"\\')).toBe('{:x #_ :title [:title "keep"] :title #_ "discard" "😀\\\"\\\\"}');
  expect(readEdnOptions('{:x [:title "keep" :collapsed? true :table-view? true]}')).toEqual({title: null, collapsed: false, table: false});
});
it("unreadable options raise the existing typed refusal", () => {
  for (const source of ['{:x}', '{:title "\\q"}', '{:title 1 :title 2}', '{:x #_}']) {
    expect(() => editEdnTitle(source, "new")).toThrow(/Unreadable EDN options/);
  }
});
