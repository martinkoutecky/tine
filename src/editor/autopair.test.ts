import { describe, it, expect } from "vitest";
import {
  autoPairInsertOnInput,
  wrapSelectionEdit,
  backspacePairEdit,
} from "./autopair";

// Simulate a single keystroke the way the browser + onInput see it: the browser
// first inserts `ch` at `caret`, THEN autoPairInsertOnInput runs on that state.
function type(value: string, caret: number, ch: string): { value: string; caret: number } {
  const v = value.slice(0, caret) + ch + value.slice(caret);
  const c = caret + 1;
  return autoPairInsertOnInput(v, c, ch) ?? { value: v, caret: c };
}

describe("autoPairInsertOnInput — single pair insert", () => {
  it("inserts the matching closer, caret between", () => {
    expect(type("", 0, "(")).toEqual({ value: "()", caret: 1 });
    expect(type("", 0, "[")).toEqual({ value: "[]", caret: 1 });
    expect(type("", 0, "{")).toEqual({ value: "{}", caret: 1 });
    expect(type("", 0, '"')).toEqual({ value: '""', caret: 1 });
    expect(type("", 0, "`")).toEqual({ value: "``", caret: 1 });
  });

  it("auto-closes before whitespace / a closer / end-of-text, but not mid-word", () => {
    expect(type(" hi", 0, "(")).toEqual({ value: "() hi", caret: 1 }); // before space
    expect(type("word", 0, "(")).toEqual({ value: "(word", caret: 1 }); // mid-word → no closer
    expect(type("[]", 1, "(")).toEqual({ value: "[()]", caret: 2 }); // nests inside a closer
  });
});

describe("autoPairInsertOnInput — skip-over", () => {
  it("types through a closer that already follows the caret", () => {
    // In `(|)`, typing `)` steps over instead of stacking `())`.
    expect(type("()", 1, ")")).toEqual({ value: "()", caret: 2 });
    expect(type('""', 1, '"')).toEqual({ value: '""', caret: 2 });
  });
});

describe("autoPairInsertOnInput — double-open composes with [[ / (( / {{", () => {
  it("`[` then `[` yields [[|]] (not [[]| ] )", () => {
    const first = type("", 0, "["); // → [|]
    expect(first).toEqual({ value: "[]", caret: 1 });
    const second = type(first.value, first.caret, "["); // → [[|]]
    expect(second).toEqual({ value: "[[]]", caret: 2 });
  });

  it("`(` then `(` yields (( |)) for block refs", () => {
    const first = type("", 0, "(");
    const second = type(first.value, first.caret, "(");
    expect(second).toEqual({ value: "(())", caret: 2 });
  });

  it("`{` then `{` yields {{|}}", () => {
    const first = type("", 0, "{");
    const second = type(first.value, first.caret, "{");
    expect(second).toEqual({ value: "{{}}", caret: 2 });
  });

  it("double-open with no pre-existing closer still inserts the pair", () => {
    // e.g. the first `[` didn't pair (typed mid-word), then a second `[` arrives.
    expect(autoPairInsertOnInput("[[", 2, "[")).toEqual({ value: "[[]]", caret: 2 });
  });
});

describe("wrapSelectionEdit", () => {
  it("wraps a non-empty selection, keeping it around the inner text", () => {
    expect(wrapSelectionEdit("hello", 1, 4, "(")).toEqual({ text: "h(ell)o", start: 2, end: 5 });
    expect(wrapSelectionEdit("x", 0, 1, '"')).toEqual({ text: '"x"', start: 1, end: 2 });
  });
  it("returns null for an empty selection or a non-opener", () => {
    expect(wrapSelectionEdit("hi", 1, 1, "(")).toBeNull();
    expect(wrapSelectionEdit("hi", 0, 2, ")")).toBeNull();
    expect(wrapSelectionEdit("hi", 0, 2, "a")).toBeNull();
  });
});

describe("backspacePairEdit", () => {
  it("deletes both chars of an empty pair", () => {
    expect(backspacePairEdit("()", 1)).toEqual({ text: "", start: 0, end: 0 });
    expect(backspacePairEdit("a()b", 2)).toEqual({ text: "ab", start: 1, end: 1 });
    expect(backspacePairEdit('""', 1)).toEqual({ text: "", start: 0, end: 0 });
  });
  it("returns null when the caret is not between a matching empty pair", () => {
    expect(backspacePairEdit("ab", 1)).toBeNull();
    expect(backspacePairEdit("(x)", 1)).toBeNull(); // non-empty
    expect(backspacePairEdit("()", 0)).toBeNull(); // caret before the pair
  });
});
