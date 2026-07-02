import { describe, it, expect } from "vitest";
import {
  autoPairInsertOnInput,
  wrapSelectionEdit,
  doubleRefKind,
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

describe("select-text → page/block ref (#18)", () => {
  // Two `[` presses on a selection compose into `[[sel]]`, keeping the selection
  // around the inner text each time — the mechanism Block.tsx drives always-on.
  it("`[` twice around a selection builds `[[sel]]`, then reports a page ref", () => {
    const first = wrapSelectionEdit("say foo bar", 4, 7, "[")!; // select "foo"
    expect(first).toEqual({ text: "say [foo] bar", start: 5, end: 8 });
    expect(doubleRefKind(first.text, first.start, first.end)).toBeNull(); // first bracket: no search yet

    const second = wrapSelectionEdit(first.text, first.start, first.end, "[")!;
    expect(second).toEqual({ text: "say [[foo]] bar", start: 6, end: 9 });
    expect(second.text.slice(second.start, second.end)).toBe("foo"); // selection still on the inner text
    expect(doubleRefKind(second.text, second.start, second.end)).toBe("page"); // now open page search
  });

  it("`(` twice around a selection builds `((sel))` and reports a block ref", () => {
    const first = wrapSelectionEdit("foo", 0, 3, "(")!;
    const second = wrapSelectionEdit(first.text, first.start, first.end, "(")!;
    expect(second.text).toBe("((foo))");
    expect(doubleRefKind(second.text, second.start, second.end)).toBe("block");
  });

  it("doubleRefKind is null for a single bracket, a non-ref pair, or out-of-bounds", () => {
    expect(doubleRefKind("[foo]", 1, 4)).toBeNull(); // single `[`
    expect(doubleRefKind("{{foo}}", 2, 5)).toBeNull(); // `{{` is not a page/block ref
    expect(doubleRefKind('"foo"', 1, 4)).toBeNull();
    expect(doubleRefKind("[[foo]]", 1, 6)).toBeNull(); // innerStart<2 guard / mismatched bounds
  });
});

describe("select-text → emphasis wrap (OG parity, always-on)", () => {
  it("wraps a selection with emphasis marks; doubling gives **/~~/==", () => {
    expect(wrapSelectionEdit("a foo b", 2, 5, "*")).toEqual({ text: "a *foo* b", start: 3, end: 6 });
    const one = wrapSelectionEdit("foo", 0, 3, "*")!;
    const two = wrapSelectionEdit(one.text, one.start, one.end, "*")!;
    expect(two.text).toBe("**foo**"); // second press → bold
    expect(two.text.slice(two.start, two.end)).toBe("foo"); // selection preserved
    expect(wrapSelectionEdit("foo", 0, 3, "~")!.text).toBe("~foo~"); // → ~~strike~~ on doubling
    expect(wrapSelectionEdit("foo", 0, 3, "=")!.text).toBe("=foo="); // → ==highlight==
    expect(wrapSelectionEdit("foo", 0, 3, "_")!.text).toBe("_foo_");
    expect(doubleRefKind(two.text, two.start, two.end)).toBeNull(); // emphasis opens no search
  });

  it("wraps Org emphasis markers `/` and `+` too, but not a plain letter or empty selection", () => {
    expect(wrapSelectionEdit("foo", 0, 3, "/")!.text).toBe("/foo/");
    expect(wrapSelectionEdit("foo", 0, 3, "+")!.text).toBe("+foo+");
    expect(wrapSelectionEdit("foo", 0, 3, "a")).toBeNull();
    expect(wrapSelectionEdit("foo", 1, 1, "*")).toBeNull();
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
