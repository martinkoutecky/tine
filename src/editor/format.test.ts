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
