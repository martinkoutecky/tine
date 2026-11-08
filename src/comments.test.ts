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
  it("anchors the final laborum with an explicit end boundary (GH #891)", () => {
    const part = "Duis aute irure dolor in reprehenderit in voluptate laborum.";
    const parent = part + " " + part;
    const at = parent.lastIndexOf("laborum.");
    const selector = quoteSelectorFor(parent, at, parent.length, parent);
    expect(selector.suffix).toBe("");
    expect(anchorQuote(parent, selector)).toEqual({ kind: "range", start: at, end: parent.length });
  });

  it("grows context past 32 characters until an interior occurrence is unique", () => {
    const common = " many identical words before the selected word ";
    const tail = " after the target with many identical trailing words ";
    const parent = "alpha" + common + "target" + tail + "beta" + common + "target" + tail + "omega";
    const at = parent.lastIndexOf("target" + tail);
    const selector = quoteSelectorFor(parent, at, at + 6, parent);
    expect(Math.max(selector.prefix!.length, selector.suffix!.length)).toBeGreaterThan(32);
    expect(anchorQuote(parent, selector)).toEqual({ kind: "range", start: at, end: at + 6 });
  });

  it.each(["md", "org"] as const)("treats the prose end before %s metadata as a boundary", (format) => {
    const prose = "Same words before laborum. Same words before laborum.";
    const parent = prose + (format === "md" ? "\nid:: 89100000-0000-4000-8000-000000000001\ntine.author:: Ada" : "\n:PROPERTIES:\n:id: 89100000-0000-4000-8000-000000000001\n:tine.author: Ada\n:END:");
    const at = prose.lastIndexOf("laborum.");
    const selector = quoteSelectorFor(prose, at, prose.length, parent, format);
    expect(selector.suffix).toBe("");
    expect(anchorQuote(parent, { quote: "laborum.", prefix: "Same words before", suffix: "" }, format))
      .toEqual({ kind: "range", start: at, end: prose.length });
    expect(anchorQuote(parent, selector, format)).toEqual({ kind: "range", start: at, end: prose.length });
  });

  it("mirrors the boundary rule at the start and does not reinterpret missing legacy context", () => {
    const parent = "word repeated word";
    expect(anchorQuote(parent, { quote: "word", prefix: "", suffix: null }))
      .toEqual({ kind: "range", start: 0, end: 4 });
    expect(anchorQuote(parent, { quote: "word", prefix: null, suffix: "" }))
      .toEqual({ kind: "range", start: 14, end: 18 });
    expect(anchorQuote(parent, { quote: "word", prefix: null, suffix: null })).toEqual({ kind: "ambiguous" });
    // A boundary hint cannot outweigh better surviving text context after an edit.
    expect(anchorQuote("word different. nearby word", { quote: "word", prefix: "nearby", suffix: "" }))
      .toEqual({ kind: "range", start: 23, end: 27 });
  });

  it.each(["md", "org"] as const)("round-trips empty boundary context distinctly from absent legacy context in %s", (format) => {
    const selector = { quote: "word", prefix: "", suffix: "" };
    expect(quoteSelectorOf(facetsOf(commentRaw(selector, format), format).properties)).toEqual(selector);
    expect(quoteSelectorOf(facetsOf(commentRaw({ quote: "word", prefix: null, suffix: null }, format), format).properties))
      .toEqual({ quote: "word", prefix: null, suffix: null });
  });

  it("keeps legacy textual winners, while identical nonempty context remains ambiguous", () => {
    const parent = "same word tail. same word tail.";
    expect(anchorQuote(parent, { quote: "word", prefix: "same", suffix: "tail." })).toEqual({ kind: "ambiguous" });
    const edited = "clue word tail. different word tail.";
    expect(anchorQuote(edited, { quote: "word", prefix: "different", suffix: null }))
      .toEqual({ kind: "range", start: edited.lastIndexOf("word"), end: edited.lastIndexOf("word") + 4 });
  });

  it("anchors a unique quote and always keeps context", () => {
    const parent = "Agent's paragraph text that has a phrase worth disputing.\ntine.author:: claude";
    const at = parent.indexOf("a phrase");
    const selector = quoteSelectorFor(parent, at, at + "a phrase worth disputing".length, parent);
    expect(selector).toEqual({ quote: "a phrase worth disputing", prefix: "Agent's paragraph text that has", suffix: "." });
    expect(sliceOf(parent, anchorQuote(parent, selector))).toBe("a phrase worth disputing");
  });

  it("always writes context, so a later second occurrence does not re-anchor the comment (ADR 0075, MC2)", () => {
    const parent = "Start here, then a phrase worth disputing, then the end.";
    const at = parent.indexOf("a phrase");
    const selector = quoteSelectorFor(parent, at, at + "a phrase".length, parent);
    expect(selector.prefix).toBe("Start here, then");
    expect(selector.suffix).toBe("worth disputing, then the end.");
    const edited = "a phrase added at the top. " + parent;
    const anchor = anchorQuote(edited, selector);
    expect(anchor).toEqual({ kind: "range", start: edited.lastIndexOf("a phrase"), end: edited.lastIndexOf("a phrase") + 8 });
  });

  it("trims context at a word boundary", () => {
    const parent = "unbelievably longwinded introductory words before the quote and plenty of trailing words after it ends";
    const at = parent.indexOf("the quote");
    const selector = quoteSelectorFor(parent, at, at + "the quote".length, parent);
    expect(selector.prefix!.length).toBeLessThanOrEqual(QUOTE_CONTEXT_MAX);
    expect(parent).toContain(" " + selector.prefix + " the quote");
    expect(selector.suffix!.length).toBeLessThanOrEqual(QUOTE_CONTEXT_MAX);
    expect(parent).toContain("the quote " + selector.suffix + " ");
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

  it("keeps the initial 32-character context on whole code points when sufficient", () => {
    const filler = "🎉".repeat(40);
    const parent = `${filler} one x ${filler} two x ${filler}`;
    const second = parent.lastIndexOf(" x ") + 1;
    const selector = quoteSelectorFor(parent, second, second + 1, parent);
    expect(selector.prefix!.length).toBeLessThanOrEqual(QUOTE_CONTEXT_MAX);
    expect(selector.suffix!.length).toBeLessThanOrEqual(QUOTE_CONTEXT_MAX);
    // The cut splits the emoji run before "two", so the word boundary drops it (MC2); the suffix is all one word.
    expect(selector.prefix).toBe("two");
    expect(selector.suffix!.startsWith("🎉")).toBe(true);
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
  it("is a comment iff it has tine.quote:: and a parent block", () => {
    expect(isCommentBlock([["tine.quote", "x"]], true)).toBe(true);
    expect(isCommentBlock([["tine.quote", ""]], true)).toBe(true);
    expect(isCommentBlock([["tine.quote", "x"]], false)).toBe(false);
    expect(isCommentBlock([["tine.author", "claude"]], true)).toBe(false);
    expect(isCommentBlock([], true)).toBe(false);
    expect(authorOf([["Tine.Author", " claude "]])).toBe("claude");
    expect(authorOf([["tine.author", ""]])).toBeNull();
    expect(quoteSelectorOf([["tine.quote", "q"], ["tine.quote-prefix", "p"], ["tine.quote_suffix", "s"]]))
      .toEqual({ quote: "q", prefix: "p", suffix: "s" });
    expect(isCommentPresentationKey("tine.author", false)).toBe(true);
    expect(isCommentPresentationKey("tine.quote", false)).toBe(false);
    expect(isCommentPresentationKey("tine.quote-prefix", true)).toBe(true);
    expect(isCommentPresentationKey("status", true)).toBe(false);
  });

  it("a user's own quote:: and author:: are ordinary properties (Discussion #825, D-20261102-PNS)", () => {
    // Book notes: `quote::` on a child bullet is not a margin comment, and
    // `author::` is not attribution. The Beta-only bare keys have no reader.
    expect(isCommentBlock([["quote", "The obstacle is the way."]], true)).toBe(false);
    expect(quoteSelectorOf([["quote", "x"], ["quote-prefix", "p"]])).toBeNull();
    expect(authorOf([["author", "Marcus Aurelius"]])).toBeNull();
    for (const key of ["author", "quote", "quote-prefix", "quote-suffix"]) {
      expect(isCommentPresentationKey(key, false)).toBe(false);
      expect(isCommentPresentationKey(key, true)).toBe(false);
    }
  });

  it("writes quote, prefix and suffix in that order under an empty first line", () => {
    expect(commentRaw({ quote: "a phrase", prefix: null, suffix: null }, "md")).toBe("\ntine.quote:: a phrase");
    expect(commentRaw({ quote: "the cat", prefix: "sat; then", suffix: "ran" }, "md"))
      .toBe("\ntine.quote:: the cat\ntine.quote-prefix:: sat; then\ntine.quote-suffix:: ran");
    expect(commentRaw({ quote: "", prefix: null, suffix: null }, "md")).toBe("\ntine.quote:: ");
    const org = commentRaw({ quote: "a, b", prefix: null, suffix: null }, "org");
    expect(facetsOf(org, "org").properties).toEqual([["tine.quote", "a, b"]]);
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
