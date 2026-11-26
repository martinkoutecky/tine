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

describe("autoPairInsertOnInput — a quote after a word closes (GH #1020)", () => {
  // `type(v, c, ch)` inserts `ch` at `c` first, as the browser does.
  it("adds no second quote after a letter or a number", () => {
    expect(type('{{y "hello}}', 10, '"')).toEqual({ value: '{{y "hello"}}', caret: 11 }); // the report
    expect(type('say "hello', 10, '"')).toEqual({ value: 'say "hello"', caret: 11 }); // end of the block
    expect(type('"hello now', 6, '"')).toEqual({ value: '"hello" now', caret: 7 }); // before a space
    expect(type('"ano', 4, '"')).toEqual({ value: '"ano"', caret: 5 });
    expect(type('"žluťoučký', 10, '"')).toEqual({ value: '"žluťoučký"', caret: 11 }); // not only ASCII
    expect(type('"2026', 5, '"')).toEqual({ value: '"2026"', caret: 6 });
    expect(type("`code", 5, "`")).toEqual({ value: "`code`", caret: 6 });
  });

  it("knows letters outside the BMP, combining marks and number signs", () => {
    // `before` ends in the word; the quote is typed at its end.
    const closes = (before: string) => expect(type(before, before.length, '"')).toEqual({ value: before + '"', caret: before.length + 1 });
    closes('"\u{20BB7}'); // a CJK letter stored as a surrogate pair
    closes('"\u{10437}'); // Deseret
    closes('"\u{1D7E0}'); // a mathematical digit
    closes('"cafe\u0301'); // e + combining acute
    closes('"\u0915\u0940'); // Devanagari letter + vowel sign
    closes('"\u00BD'); // one half
    expect(type("say \u0301", 5, '"')).toEqual({ value: 'say \u0301""', caret: 6 }); // a mark with no letter opens
  });

  it("still pairs a quote that opens: at the start, after a space, an opener or punctuation", () => {
    expect(type("say ", 4, '"')).toEqual({ value: 'say ""', caret: 5 });
    expect(type("{{y }}", 4, '"')).toEqual({ value: '{{y ""}}', caret: 5 });
    expect(type("()", 1, '"')).toEqual({ value: '("")', caret: 2 });
    expect(type("a: ", 3, "`")).toEqual({ value: "a: ``", caret: 4 });
    expect(type("_", 1, '"')).toEqual({ value: '_""', caret: 2 }); // an emphasis mark is not a word
  });

  it("adds nothing to a second or third closing quote after a word", () => {
    const keys = (value: string, caret: number, text: string) => {
      let state = { value, caret };
      for (const ch of text) state = type(state.value, state.caret, ch);
      return state;
    };
    // A double-backtick code span: two closing backticks stay two.
    expect(keys("``a`b", 5, "``")).toEqual({ value: "``a`b``", caret: 7 }); // end of the block
    expect(keys("``a`b now", 5, "``")).toEqual({ value: "``a`b`` now", caret: 7 }); // before a space
    expect(keys("(``a`b)", 6, "``")).toEqual({ value: "(``a`b``)", caret: 8 }); // before a closer
    expect(keys('""word', 6, '""')).toEqual({ value: '""word""', caret: 8 });
    expect(keys('""word then', 6, '""')).toEqual({ value: '""word"" then', caret: 8 });
    expect(keys('(""word)', 7, '""')).toEqual({ value: '(""word"")', caret: 9 });
    expect(keys("```word", 7, "```")).toEqual({ value: "```word```", caret: 10 });
    // A closing quote already there, one more typed after it.
    expect(type('"word"', 6, '"')).toEqual({ value: '"word""', caret: 7 });
    // Stepping over an existing quote, then typing one more.
    expect(keys('word"', 4, '""')).toEqual({ value: 'word""', caret: 6 });
  });

  it("after a word, every typed quote inserts exactly itself", () => {
    for (const q of ['"', "`"]) for (const run of ["", q, q + q, q + q + q]) for (const next of ["", " ", ")", "]", "}"]) {
      const before = "word" + run;
      expect(type(before + next, before.length, q)).toEqual({ value: before + q + next, caret: before.length + 1 });
    }
  });

  it("a run of quotes with no word before it still pairs and steps over", () => {
    expect(type('"', 1, '"')).toEqual({ value: '"""', caret: 2 }); // one already there, no word
    expect(type('say "', 5, '"')).toEqual({ value: 'say """', caret: 6 });
    expect(type('word"', 4, '"')).toEqual({ value: 'word"', caret: 5 }); // steps over, right after a word
    expect(type("word", 0, '"')).toEqual({ value: '"word', caret: 1 }); // before a word: nothing more
  });

  it("leaves brackets alone: they open after a word as before", () => {
    expect(type("f", 1, "(")).toEqual({ value: "f()", caret: 2 });
    expect(type("see", 3, "[")).toEqual({ value: "see[]", caret: 4 });
  });

  it("typing the text of the report key by key gives it back", () => {
    let state = { value: "", caret: 0 };
    for (const ch of '{{y "hello"}}') state = type(state.value, state.caret, ch);
    expect(state).toEqual({ value: '{{y "hello"}}', caret: 13 });
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
  it("wraps full-width selection openers with ASCII brackets and parentheses", () => {
    expect(wrapSelectionEdit("hello", 1, 4, "（")).toEqual({ text: "h(ell)o", start: 2, end: 5 });
    expect(wrapSelectionEdit("hello", 1, 4, "【")).toEqual({ text: "h[ell]o", start: 2, end: 5 });
  });
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
