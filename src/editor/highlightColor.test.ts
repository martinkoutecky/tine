// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import { highlightMarkOpen, setHighlightColor } from "./format";
import { BLOCK_COLOR_NAMES } from "../blockColors";
import { colorOnlyStyles } from "../render/htmlSanitize";

// a colour swatch under the selection toolbar wraps the selection in
// Logseq's theme-aware highlight, replaces a colour already there, and "no colour" removes it.
const RED = '<mark style="background-color: var(--ls-highlight-color-red, #fdd8d8)">';
const BLUE = '<mark style="background-color: var(--ls-highlight-color-blue, #cee7fe)">';

describe("setHighlightColor", () => {
  it("wraps the selection in the exact Logseq-compatible markup and keeps the same text selected", () => {
    const text = "say hello world";
    const edit = setHighlightColor(text, 4, 9, "red", "forward")!;
    expect(edit.text).toBe(`say ${RED}hello</mark> world`);
    expect(edit.text.slice(edit.start, edit.end)).toBe("hello");
    expect(edit.direction).toBe("forward");
  });

  it("uses Logseq's seven built-in colour names, in Logseq's order", () => {
    expect(BLOCK_COLOR_NAMES).toEqual(["yellow", "red", "pink", "green", "blue", "purple", "gray"]);
  });

  it("writes, for every colour, a declaration the sanitizer keeps unchanged", () => {
    const styles = BLOCK_COLOR_NAMES.map((name) => highlightMarkOpen(name).match(/style="([^"]*)"/)![1]);
    expect(colorOnlyStyles(styles)).toEqual(styles);
  });

  it("replaces the colour around the selection with the second one", () => {
    const first = setHighlightColor("say hello world", 4, 9, "red")!;
    const second = setHighlightColor(first.text, first.start, first.end, "blue")!;
    expect(second.text).toBe(`say ${BLUE}hello</mark> world`);
    expect(second.text.slice(second.start, second.end)).toBe("hello");
    // Selecting the whole markup replaces it too.
    const whole = setHighlightColor(first.text, 4, first.end + "</mark>".length, "blue")!;
    expect(whole.text).toBe(second.text);
    expect(whole.text.slice(whole.start, whole.end)).toBe("hello");
  });

  it("removes the colour with no colour, and changes nothing when there is none", () => {
    const first = setHighlightColor("say hello world", 4, 9, "red")!;
    const removed = setHighlightColor(first.text, first.start, first.end, null)!;
    expect(removed).toEqual({ text: "say hello world", start: 4, end: 9 });
    expect(setHighlightColor("say hello world", 4, 9, null)).toBeNull();
    expect(setHighlightColor("say ==hello== world", 6, 11, null)).toBeNull();
  });

  it("turns a plain == highlight around the selection into the coloured one", () => {
    const edit = setHighlightColor("say ==hello== world", 6, 11, "red")!;
    expect(edit.text).toBe(`say ${RED}hello</mark> world`);
    expect(edit.text.slice(edit.start, edit.end)).toBe("hello");
  });

  it("leaves browser-selected outer whitespace outside the mark", () => {
    const edit = setHighlightColor("say hello world", 3, 10, "red")!;
    expect(edit.text).toBe(`say ${RED}hello</mark> world`);
    expect(edit.text.slice(edit.start, edit.end)).toBe("hello");
  });

  it.each([
    ["a page link", "see [[Some page]] now", 4, 17],
    ["text inside a page link", "see [[Some page]] now", 11, 15],
    ["bold text", "see **this** now", 4, 12],
    ["half of a bold span", "see **this one** now", 2, 10],
    ["inline code", "see `code` now", 4, 10],
    ["text inside inline code", "see `some code` now", 5, 9],
    ["a tag", "see #tag now", 4, 8],
    ["a tag", "x a <b>c</b>", 2, 12],
    ["a character reference", "x a &amp; b", 2, 11],
    ["an escape", "x a \\*b", 2, 7],
    ["two lines", "x one\ntwo", 2, 9],
    // mldoc reads no inline HTML inside emphasis: Logseq would show the markup.
    ["text inside bold", "**say hello world**", 6, 11],
    // At the start of a block's text mldoc reads `<mark …>` as a block of raw HTML and the
    // block's title becomes empty, in Logseq as in Tine.
    ["the first word of a block", "hello world", 0, 5],
    ["the first word after a task marker", "TODO hello world", 5, 10],
  ])("refuses %s, whose markup would stop working inside HTML", (_label, text, start, end) => {
    expect(setHighlightColor(text, start, end, "red")).toBeNull();
  });

  it("refuses an unknown colour and an empty selection", () => {
    expect(setHighlightColor("say hello", 4, 9, "orange")).toBeNull();
    expect(setHighlightColor("say hello", 4, 4, "red")).toBeNull();
    expect(setHighlightColor("say   hello", 3, 6, "red")).toBeNull();
  });

  it("colours text next to an identical highlight", () => {
    const first = setHighlightColor("x a b a", 2, 3, "red")!;
    const second = setHighlightColor(first.text, first.text.length - 1, first.text.length, "red")!;
    expect(second.text).toBe(`x ${RED}a</mark> b ${RED}a</mark>`);
  });

  // Merge review of b218d18e3, finding 2: the parser decides which highlight owns the selection.
  describe("the highlight that owns the selection", () => {
    const twoMarks = (a: string, b: string) => `say ${a}one</mark> and ${b}two</mark> now`;
    it.each([
      ["two marks of one colour, inner text selected", twoMarks(RED, RED), 0],
      ["two marks of two colours, inner text selected", twoMarks(RED, BLUE), 0],
      ["two marks with their tags selected", twoMarks(RED, BLUE), 1],
    ])("removes nothing across %s", (_label, text, withTags) => {
      const from = withTags ? 4 : text.indexOf("one");
      const to = withTags ? text.lastIndexOf("</mark>") + "</mark>".length : text.indexOf("two") + 3;
      expect(setHighlightColor(text, from, to, null)).toBeNull();
      expect(setHighlightColor(text, from, to, "green")).toBeNull();
    });

    it.each([
      ["inline code", `x \`${RED}hello</mark>\` y`],
      ["a code fence", `x\n\`\`\`\n${RED}hello</mark>\n\`\`\``],
      ["bold text", `x **${RED}hello</mark>** y`],
      ["a property value", `x\nnote:: ${RED}hello</mark>`],
    ])("leaves a literal copy of the markup in %s as it is", (_label, text) => {
      const from = text.indexOf("hello"), to = from + 5;
      expect(setHighlightColor(text, from, to, null)).toBeNull();
      const edit = setHighlightColor(text, from, to, "blue");
      expect(edit).toBeNull();
    });

    it("colours the whole text of a plain == highlight it is inside", () => {
      const text = "say ===hello=== now";
      const edit = setHighlightColor(text, 7, 12, "red")!;
      expect(edit.text).toBe(`say ${RED}=hello</mark>= now`);
      expect(edit.text.slice(edit.start, edit.end)).toBe("hello");
    });

    it("recolours and removes a mark from a selection inside it, keeping the same words selected", () => {
      const text = `say ${RED}hello world</mark> now`;
      const from = text.indexOf("world");
      const blue = setHighlightColor(text, from, from + 5, "blue")!;
      expect(blue.text).toBe(`say ${BLUE}hello world</mark> now`);
      expect(blue.text.slice(blue.start, blue.end)).toBe("world");
      const removed = setHighlightColor(blue.text, blue.start, blue.end, null)!;
      expect(removed.text).toBe("say hello world now");
      expect(removed.text.slice(removed.start, removed.end)).toBe("world");
    });

    it("changes nothing when the colour is already the one there", () => {
      const first = setHighlightColor("say hello world", 4, 9, "red")!;
      expect(setHighlightColor(first.text, first.start, first.end, "red")).toBeNull();
    });

    // Merge review of 75e6ab532, findings 1 and 5: only a mark the row wrote is rewritten. A mark
    // written any other way keeps every declaration and its spelling, whether the selection is
    // its text, the whole mark, or a span around it.
    it.each([
      ["a foreground", '<mark style="background-color: #000; color: #fff">'],
      ["a decoration colour", '<mark style="background-color: #fdd8d8; text-decoration-color: red">'],
      ["two spaces after the colon", '<mark style="background-color:  var(--ls-highlight-color-red, #fdd8d8)">'],
      ["another hex fallback", '<mark style="background-color: var(--ls-highlight-color-red, #ff0000)">'],
      ["an uppercase property", '<mark style="BACKGROUND-COLOR: var(--ls-highlight-color-red, #fdd8d8)">'],
      ["a single-quoted attribute", "<mark style='background-color: var(--ls-highlight-color-red, #fdd8d8)'>"],
      ["a literal hex background", '<mark style="background-color: #fdd8d8">'],
      ["an uppercase tag", '<MARK STYLE="background-color: #fdd8d8">', "</MARK>"],
      ["an uppercase open tag", '<MARK style="background-color: #000">'],
      ["a numeric reference in the style", '<mark style="background-color: &#35;000">'],
      ["a numeric reference in the property", '<mark style="background&#45;color: #000">'],
      ["a second attribute", '<mark class="x" style="background-color: #fdd8d8">'],
      ["a declaration the sanitizer drops", '<mark style="background-color: #fdd8d8; padding: 2px">'],
      ["no background", '<mark style="color: #c00">'],
      ["no style", "<mark>"],
    ])("leaves a mark written with %s as it is", (_label, open, close = "</mark>") => {
      const text = `say ${open}hello${close} now`;
      const from = text.indexOf("hello");
      const end = text.indexOf(" now");
      for (const [s, e] of [[from, from + 5], [from + 1, from + 3], [4, end], [0, text.length]]) {
        expect(setHighlightColor(text, s, e, "blue")).toBeNull();
        expect(setHighlightColor(text, s, e, null)).toBeNull();
      }
    });

    it("leaves a mark with spaces just inside its tags as it is", () => {
      const text = `say ${RED} hello </mark> now`;
      const from = text.indexOf("hello");
      expect(setHighlightColor(text, from, from + 5, "blue")).toBeNull();
      expect(setHighlightColor(text, from, from + 5, null)).toBeNull();
    });

    it("refuses a selection with a row's mark inside it", () => {
      const text = `say one ${RED}two</mark> three`;
      expect(setHighlightColor(text, 4, text.length, "blue")).toBeNull();
      expect(setHighlightColor(text, 4, text.length, null)).toBeNull();
    });
  });

  // Merge review of 75e6ab532, finding 4: a colour is refused exactly when the page's own HTML
  // parse would show other text inside the mark (it decodes character references).
  it.each(["R&D", "AT&T", "A&B", "A & B", "a < b", "x &; y", "fish & chips;"])("colours %s as written", (word) => {
    const text = `say ${word} now`;
    const edit = setHighlightColor(text, 4, 4 + word.length, "red")!;
    expect(edit.text).toBe(`say ${RED}${word}</mark> now`);
    expect(edit.text.slice(edit.start, edit.end)).toBe(word);
    const removed = setHighlightColor(edit.text, edit.start, edit.end, null)!;
    expect(removed.text).toBe(text);
  });

  // Finding 3: an HTML parser decodes character references, so a colour would change the words.
  it.each(["&copy", "&copycat", "&amp", "&AMP", "&nbsp", "&lt", "&gt", "&quot", "&notin", "&#35", "&#x41"])(
    "refuses a selection holding %s",
    (word) => {
      const text = `say ${word}; now`;
      expect(setHighlightColor(text, 4, 4 + word.length, "red")).toBeNull();
    },
  );
});
