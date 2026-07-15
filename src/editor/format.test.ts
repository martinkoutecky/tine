import { describe, it, expect } from "vitest";
import {
  toggleWrap,
  insertLink,
  killLineBefore,
  killLineAfter,
  wordForward,
  wordBackward,
  killWordForward,
  killWordBackward,
  setPriority,
  trimBlockTrailingSpace,
  wrapLink,
  isPasteableUrl,
} from "./format";

describe("toggleWrap", () => {
  it("wraps a selection", () => {
    expect(toggleWrap("hello world", 0, 5, "**")).toEqual({ text: "**hello** world", start: 2, end: 7 });
  });
  it("unwraps when markers are just outside the selection", () => {
    expect(toggleWrap("**hello** world", 2, 7, "**")).toEqual({ text: "hello world", start: 0, end: 5 });
  });
  it("unwraps when markers are inside the selection", () => {
    expect(toggleWrap("**hello** world", 0, 9, "**")).toEqual({ text: "hello world", start: 0, end: 5 });
  });
  it("inserts an empty pair with caret between on no selection", () => {
    expect(toggleWrap("ab", 1, 1, "**")).toEqual({ text: "a****b", start: 3, end: 3 });
  });
  it("supports asymmetric wraps (underline)", () => {
    expect(toggleWrap("x", 0, 1, "<ins>", "</ins>")).toEqual({
      text: "<ins>x</ins>",
      start: 5,
      end: 6,
    });
  });
});

describe("insertLink", () => {
  it("turns a selection into the label, caret in ()", () => {
    expect(insertLink("see docs", 4, 8)).toEqual({ text: "see [docs]()", start: 11, end: 11 });
  });
  it("inserts empty link with caret in [] on no selection", () => {
    expect(insertLink("a b", 2, 2)).toEqual({ text: "a []()b", start: 3, end: 3 });
  });

  it("uses the page format and parser-recognized selected links rather than a URL regex", () => {
    const formatAware = insertLink as (text: string, start: number, end: number, format: "md" | "org") => ReturnType<typeof insertLink>;
    expect(formatAware("Label", 0, 5, "org")).toEqual({ text: "[[][Label]]", start: 2, end: 2 });
    expect(formatAware("https://example.com", 0, 19, "md")).toEqual({ text: "[](https://example.com)", start: 1, end: 1 });
    expect(formatAware("[[Page]]", 0, 8, "org")).toEqual({ text: "[[[[Page]]][]]", start: 12, end: 12 });
  });

  it("preserves surrounding text and recognizes block refs and already formatted links", () => {
    const formatAware = insertLink as (text: string, start: number, end: number, format: "md" | "org") => ReturnType<typeof insertLink>;
    expect(formatAware("before Label after", 7, 12, "org")).toEqual({
      text: "before [[][Label]] after", start: 9, end: 9,
    });
    expect(formatAware("((abc-123))", 0, 11, "md")).toEqual({
      text: "[](((abc-123)))", start: 1, end: 1,
    });
    expect(formatAware("[label](https://example.com)", 0, 28, "md")).toEqual({
      text: "[]([label](https://example.com))", start: 1, end: 1,
    });
  });
});

describe("isPasteableUrl", () => {
  it("accepts http(s) and mailto single tokens", () => {
    expect(isPasteableUrl("https://www.github.com")).toBe(true);
    expect(isPasteableUrl("http://example.com/a?b=c#d")).toBe(true);
    expect(isPasteableUrl("mailto:x@y.com")).toBe(true);
    expect(isPasteableUrl("  https://x.com  ")).toBe(true); // trims
  });
  it("rejects non-URLs, scheme-less, and multi-token text", () => {
    expect(isPasteableUrl("just text")).toBe(false);
    expect(isPasteableUrl("www.github.com")).toBe(false); // no scheme
    expect(isPasteableUrl("see https://x.com now")).toBe(false); // spaces
    expect(isPasteableUrl("")).toBe(false);
  });
});

describe("wrapLink", () => {
  it("wraps a selection as a markdown link, caret after", () => {
    // "Link" selected at 0..4, paste https://x.com
    expect(wrapLink("Link", 0, 4, "https://x.com", "md")).toEqual({
      text: "[Link](https://x.com)",
      start: 21,
      end: 21,
    });
  });
  it("wraps a selection as an org link (target first)", () => {
    expect(wrapLink("Link", 0, 4, "https://x.com", "org")).toEqual({
      text: "[[https://x.com][Link]]",
      start: 23,
      end: 23,
    });
  });
  it("preserves surrounding text", () => {
    expect(wrapLink("a Link b", 2, 6, "https://x.com", "md").text).toBe("a [Link](https://x.com) b");
  });
});

describe("kill motions", () => {
  it("killLineBefore deletes from line start to caret", () => {
    expect(killLineBefore("foo bar", 4)).toEqual({ text: "bar", start: 0, end: 0 });
  });
  it("killLineBefore respects newlines", () => {
    expect(killLineBefore("a\nfoo bar", 6)).toEqual({ text: "a\nbar", start: 2, end: 2 });
  });
  it("killLineAfter deletes from caret to line end", () => {
    expect(killLineAfter("foo bar", 3)).toEqual({ text: "foo", start: 3, end: 3 });
  });
  it("killLineAfter respects newlines", () => {
    expect(killLineAfter("foo\nbar", 1)).toEqual({ text: "f\nbar", start: 1, end: 1 });
  });
});

describe("word motions", () => {
  it("wordForward jumps over the next word", () => {
    expect(wordForward("foo bar", 0)).toBe(3);
    expect(wordForward("foo bar", 3)).toBe(7);
  });
  it("wordBackward jumps to the previous word start", () => {
    expect(wordBackward("foo bar", 7)).toBe(4);
    expect(wordBackward("foo bar", 4)).toBe(0);
  });
  it("killWordForward / Backward delete a word", () => {
    expect(killWordForward("foo bar", 0)).toEqual({ text: " bar", start: 0, end: 0 });
    expect(killWordBackward("foo bar", 3)).toEqual({ text: " bar", start: 0, end: 0 });
  });
});

describe("setPriority", () => {
  it("adds priority to a plain block", () => {
    expect(setPriority("buy milk", "A")).toBe("[#A] buy milk");
  });
  it("places priority after a task marker", () => {
    expect(setPriority("TODO buy milk", "B")).toBe("TODO [#B] buy milk");
  });
  it("replaces an existing priority", () => {
    expect(setPriority("TODO [#A] buy milk", "C")).toBe("TODO [#C] buy milk");
    expect(setPriority("[#A] buy milk", "B")).toBe("[#B] buy milk");
  });
  it("handles an empty body", () => {
    expect(setPriority("TODO", "A")).toBe("TODO [#A]");
  });
});

describe("trimBlockTrailingSpace", () => {
  it("drops trailing spaces/tabs at the very end (the /priority convenience space)", () => {
    expect(trimBlockTrailingSpace("TODO [#A] ")).toBe("TODO [#A]");
    expect(trimBlockTrailingSpace("hello \t ")).toBe("hello");
  });
  it("leaves a block with no trailing whitespace unchanged", () => {
    expect(trimBlockTrailingSpace("TODO [#A]")).toBe("TODO [#A]");
    expect(trimBlockTrailingSpace("")).toBe("");
  });
  it("preserves leading indent, internal spaces, and a trailing newline", () => {
    // Only the absolute end is trimmed — list continuation lines, internal
    // spacing, and a trailing newline are untouched.
    expect(trimBlockTrailingSpace("  - item")).toBe("  - item");
    expect(trimBlockTrailingSpace("a  b")).toBe("a  b");
    expect(trimBlockTrailingSpace("a \nb")).toBe("a \nb"); // space before \n is internal
    expect(trimBlockTrailingSpace("line\n")).toBe("line\n");
  });
  it("keeps the required space on an empty trailing list/checkbox item", () => {
    // The list/checkbox renderers need whitespace after the marker, so a bare
    // trailing marker must NOT lose its space.
    expect(trimBlockTrailingSpace("* ")).toBe("* ");
    expect(trimBlockTrailingSpace("text\n+ ")).toBe("text\n+ ");
    expect(trimBlockTrailingSpace("1. ")).toBe("1. ");
    expect(trimBlockTrailingSpace("  - ")).toBe("  - ");
    expect(trimBlockTrailingSpace("* [ ] ")).toBe("* [ ] ");
    expect(trimBlockTrailingSpace("* [x] ")).toBe("* [x] ");
    // …but a list item WITH content still loses its trailing space.
    expect(trimBlockTrailingSpace("* item ")).toBe("* item");
  });
});
