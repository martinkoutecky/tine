import { describe, expect, it } from "vitest";
import {
  anchorQuote, authorOf, commentRaw, isCommentBlock, isCommentPresentationKey, normalizeQuoteText,
  quoteSelectorFor, quoteSelectorOf, QUOTE_CONTEXT_MAX,
} from "./comments";
import { facetsOf } from "./render/facets";
import { blockRegions } from "./render/parse";

const sliceOf = (raw: string, anchor: ReturnType<typeof anchorQuote>) =>
  anchor.kind === "range" ? raw.slice(anchor.start, anchor.end) : anchor.kind;

describe("margin comment anchor (vision §3.7)", () => {
  it("anchors a unique quote and needs no context", () => {
    const parent = "Agent's paragraph text that has a phrase worth disputing.\nauthor:: claude";
    const at = parent.indexOf("a phrase");
    const selector = quoteSelectorFor(parent, at, at + "a phrase worth disputing".length, parent);
    expect(selector).toEqual({ quote: "a phrase worth disputing", prefix: null, suffix: null });
    expect(sliceOf(parent, anchorQuote(parent, selector))).toBe("a phrase worth disputing");
  });

  it("tells repeated occurrences apart by prefix and suffix", () => {
    const parent = "the cat sat; then the cat ran; and the cat slept";
    const second = parent.indexOf("the cat", 10);
    const selector = quoteSelectorFor(parent, second, second + 7, parent);
    expect(selector.quote).toBe("the cat");
    expect(selector.prefix).toBe("the cat sat; then");
    expect(selector.suffix).toBe("ran; and the cat slept");
    const anchor = anchorQuote(parent, selector);
    expect(anchor).toEqual({ kind: "range", start: second, end: second + 7 });
    const third = parent.lastIndexOf("the cat");
    expect(anchorQuote(parent, quoteSelectorFor(parent, third, third + 7, parent)))
      .toEqual({ kind: "range", start: third, end: third + 7 });
    // An edit elsewhere keeps the anchor on the meant occurrence.
    const edited = "Intro. " + parent.replace("slept", "dozed");
    expect(anchorQuote(edited, selector)).toEqual({ kind: "range", start: second + 7, end: second + 14 });
  });

  it("caps context at 32 characters on whole code points", () => {
    const filler = "🎉".repeat(40);
    const parent = `${filler} one x ${filler} two x ${filler}`;
    const second = parent.lastIndexOf(" x ") + 1;
    const selector = quoteSelectorFor(parent, second, second + 1, parent);
    expect(selector.prefix!.length).toBeLessThanOrEqual(QUOTE_CONTEXT_MAX);
    expect(selector.suffix!.length).toBeLessThanOrEqual(QUOTE_CONTEXT_MAX);
    expect(selector.prefix!.endsWith("🎉 two")).toBe(true);
    expect(selector.prefix!.charCodeAt(0)).not.toBe(0xdc89); // no lone low surrogate at the cut
    expect(anchorQuote(parent, selector)).toEqual({ kind: "range", start: second, end: second + 1 });
  });

  it("normalises whitespace runs on both sides and maps back to raw offsets", () => {
    expect(normalizeQuoteText("  a\n\tb   c  ")).toBe("a b c");
    const parent = "first line ends\n  and the next   starts";
    const selector = quoteSelectorFor(parent, parent.indexOf("ends"), parent.indexOf("next") + 4, parent);
    expect(selector.quote).toBe("ends and the next");
    expect(sliceOf(parent, anchorQuote(parent, selector))).toBe("ends\n  and the next");
    expect(sliceOf("ends and the next", anchorQuote("ends and the next", { quote: "ends\n and  the next", prefix: null, suffix: null })))
      .toBe("ends and the next");
  });

  it("reports a stale quote and an empty quote as the whole block", () => {
    expect(anchorQuote("the text was rewritten", { quote: "a phrase", prefix: null, suffix: null })).toEqual({ kind: "stale" });
    expect(anchorQuote("anything", { quote: "", prefix: null, suffix: null })).toEqual({ kind: "whole" });
    expect(quoteSelectorFor("some text", 4, 4, "some text")).toEqual({ quote: "", prefix: null, suffix: null });
    expect(quoteSelectorFor("some   text", 4, 7, "some   text")).toEqual({ quote: "", prefix: null, suffix: null });
  });
});

describe("margin comment format", () => {
  it("is a comment iff it has quote:: and a parent block", () => {
    expect(isCommentBlock([["quote", "x"]], true)).toBe(true);
    expect(isCommentBlock([["quote", ""]], true)).toBe(true);
    expect(isCommentBlock([["quote", "x"]], false)).toBe(false);
    expect(isCommentBlock([["author", "claude"]], true)).toBe(false);
    expect(isCommentBlock([], true)).toBe(false);
    expect(authorOf([["Author", " claude "]])).toBe("claude");
    expect(authorOf([["author", ""]])).toBeNull();
    expect(quoteSelectorOf([["quote", "q"], ["quote-prefix", "p"], ["quote_suffix", "s"]]))
      .toEqual({ quote: "q", prefix: "p", suffix: "s" });
    expect(isCommentPresentationKey("author", false)).toBe(true);
    expect(isCommentPresentationKey("quote", false)).toBe(false);
    expect(isCommentPresentationKey("quote-prefix", true)).toBe(true);
    expect(isCommentPresentationKey("status", true)).toBe(false);
  });

  it("writes quote, prefix and suffix in that order under an empty first line", () => {
    expect(commentRaw({ quote: "a phrase", prefix: null, suffix: null }, "md")).toBe("\nquote:: a phrase");
    expect(commentRaw({ quote: "the cat", prefix: "sat; then", suffix: "ran" }, "md"))
      .toBe("\nquote:: the cat\nquote-prefix:: sat; then\nquote-suffix:: ran");
    expect(commentRaw({ quote: "", prefix: null, suffix: null }, "md")).toBe("\nquote:: ");
    const org = commentRaw({ quote: "a, b", prefix: null, suffix: null }, "org");
    expect(facetsOf(org, "org").properties).toEqual([["quote", "a, b"]]);
  });

  const AWKWARD = [
    "a, b, c",
    "key:: value",
    "x: y and :: z",
    "[[x]] and #tag",
    "\"quoted\"",
    "it's 'single'",
    "emoji 🎉 and ZWJ 👩‍👩‍👧",
    "`code` and **bold**",
    "{{embed [[x]]}}",
    "((6512a0c4-0000-4000-8000-000000000000))",
    ":PROPERTIES:",
    "TODO looks like a marker",
    "ends with \\",
    "42",
    "long ".repeat(400).trim(),
  ];
  it.each(["md", "org"] as const)("round-trips awkward quotes through the %s property reader", (format) => {
    for (const text of AWKWARD) {
      const selector = { quote: normalizeQuoteText(text), prefix: "pre, fix::", suffix: "[[suf]]" };
      const raw = commentRaw(selector, format);
      expect(quoteSelectorOf(facetsOf(raw, format).properties), `${format}: ${text}`).toEqual(selector);
      expect(quoteSelectorOf(blockRegions(raw, format).properties.filter((p) => p.primary).map((p) => [p.key, p.value])))
        .toEqual(selector);
      // The quoted text anchors in a parent that holds it.
      expect(anchorQuote(`before ${text} after`, selector).kind).toBe("range");
    }
  });
});
