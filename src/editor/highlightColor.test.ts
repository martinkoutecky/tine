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
    ["a tag opener", "x a < b", 2, 7],
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
});
