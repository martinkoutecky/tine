// Margin dialogue, slice 1 (vision 2026-10 §3.7): a comment is an ordinary child
// block that carries `quote::` (the text it is about). Its own children are its
// thread. Agents attribute what they write with `author::`; a block without it is
// the graph owner's. Everything here is derived from the block's properties at
// render time: nothing is persisted beyond the ordinary outline, and no offset is
// ever stored (the quote is re-anchored against the parent's text on each render).
//
// Format constants live only here, so renaming a key is a one-line change.

import { editBlock } from "./render/parse";
import { propertyKeyNorm } from "./propertyKey";
import type { Format } from "./render/ast";

export const COMMENT_QUOTE_KEY = "quote";
export const COMMENT_QUOTE_PREFIX_KEY = "quote-prefix";
export const COMMENT_QUOTE_SUFFIX_KEY = "quote-suffix";
export const AUTHOR_KEY = "author";
/** W3C TextQuoteSelector context length (Hypothes.is uses 32 as well). */
export const QUOTE_CONTEXT_MAX = 32;

/** What a comment quotes. `quote === ""` means the whole parent block. */
export interface QuoteSelector {
  quote: string;
  prefix: string | null;
  suffix: string | null;
}

/** Where a quote lands in the parent's raw text, in UTF-16 offsets. */
export type QuoteAnchor =
  | { kind: "whole" }
  | { kind: "range"; start: number; end: number }
  | { kind: "stale" };

// --- whitespace normalisation ----------------------------------------------
// A property value holds one line, so a quote collapses every whitespace run to
// one space and drops the ends (the property reader trims values anyway). The
// parent is searched under the same normalisation, through an index map back to
// its raw text. A character test, not a pattern over block content.

function isSpace(code: number): boolean {
  return code === 0x20 || (code >= 0x09 && code <= 0x0d) || code === 0xa0 || code === 0x1680
    || (code >= 0x2000 && code <= 0x200a) || code === 0x2028 || code === 0x2029
    || code === 0x202f || code === 0x205f || code === 0x3000 || code === 0xfeff;
}

interface Normalized {
  text: string;
  /** Raw index where normalized char i starts. */
  starts: number[];
  /** Raw index just past normalized char i (a collapsed run ends at the run's end). */
  ends: number[];
}

function normalizeWithMap(raw: string): Normalized {
  let text = "";
  const starts: number[] = [];
  const ends: number[] = [];
  let i = 0;
  while (i < raw.length && isSpace(raw.charCodeAt(i))) i++;
  while (i < raw.length) {
    if (isSpace(raw.charCodeAt(i))) {
      const runStart = i;
      while (i < raw.length && isSpace(raw.charCodeAt(i))) i++;
      if (i >= raw.length) break; // trailing run: trimmed
      text += " ";
      starts.push(runStart);
      ends.push(i);
      continue;
    }
    text += raw[i];
    starts.push(i);
    ends.push(i + 1);
    i++;
  }
  return { text, starts, ends };
}

/** The one-line form a quote (or its context) is written in. */
export function normalizeQuoteText(text: string): string {
  return normalizeWithMap(text).text;
}

/** Drop a surrogate half left at either edge by a fixed-length cut. */
function wholeCodePoints(text: string): string {
  let start = 0;
  let end = text.length;
  if (end > 0 && text.charCodeAt(0) >= 0xdc00 && text.charCodeAt(0) <= 0xdfff) start = 1;
  if (end > start && text.charCodeAt(end - 1) >= 0xd800 && text.charCodeAt(end - 1) <= 0xdbff) end--;
  return text.slice(start, end);
}

function occurrences(haystack: string, needle: string): number[] {
  const out: number[] = [];
  if (!needle) return out;
  for (let at = haystack.indexOf(needle); at !== -1; at = haystack.indexOf(needle, at + 1)) out.push(at);
  return out;
}

/** The selector for a text selection [start, end) of `editorText`, the editor's
 * view of the parent whose current raw text is `parentRaw`. The quote is the
 * normalised selection. Prefix and suffix (≤32 chars, normalised, trimmed) are
 * written only when the quote occurs more than once in the parent, to say which
 * occurrence was meant. An empty selection quotes the whole block (`quote === ""`).
 * Cost O(parent length × occurrences). */
export function quoteSelectorFor(editorText: string, start: number, end: number, parentRaw: string): QuoteSelector {
  const lo = Math.max(0, Math.min(start, end, editorText.length));
  const hi = Math.min(editorText.length, Math.max(start, end));
  const quote = normalizeQuoteText(editorText.slice(lo, hi));
  if (!quote) return { quote: "", prefix: null, suffix: null };
  // Which occurrence of the quote (counted in the editor's text) was selected.
  const editor = normalizeWithMap(editorText);
  let selectedAt = editor.starts.findIndex((rawIndex) => rawIndex >= lo);
  if (selectedAt === -1) selectedAt = editor.text.length;
  const ordinal = occurrences(editor.text, quote).filter((at) => at < selectedAt).length;
  const parent = normalizeWithMap(parentRaw);
  const found = occurrences(parent.text, quote);
  if (found.length <= 1) return { quote, prefix: null, suffix: null };
  const at = found[Math.min(ordinal, found.length - 1)];
  const prefix = wholeCodePoints(parent.text.slice(Math.max(0, at - QUOTE_CONTEXT_MAX), at)).trim();
  const after = at + quote.length;
  const suffix = wholeCodePoints(parent.text.slice(after, after + QUOTE_CONTEXT_MAX)).trim();
  return { quote, prefix: prefix || null, suffix: suffix || null };
}

function commonSuffixLength(a: string, b: string): number {
  let n = 0;
  while (n < a.length && n < b.length && a[a.length - 1 - n] === b[b.length - 1 - n]) n++;
  return n;
}

function commonPrefixLength(a: string, b: string): number {
  let n = 0;
  while (n < a.length && n < b.length && a[n] === b[n]) n++;
  return n;
}

/** Re-anchor a selector in the parent's raw text: the occurrence whose
 * surroundings agree best with the stored prefix and suffix (the earliest on a
 * tie), `whole` for an empty quote, `stale` when the quote no longer occurs.
 * Never persisted. Cost O(parent length × occurrences). */
export function anchorQuote(parentRaw: string, selector: QuoteSelector): QuoteAnchor {
  const quote = normalizeQuoteText(selector.quote);
  if (!quote) return { kind: "whole" };
  const parent = normalizeWithMap(parentRaw);
  const found = occurrences(parent.text, quote);
  if (found.length === 0) return { kind: "stale" };
  let best = found[0];
  if (found.length > 1) {
    const prefix = normalizeQuoteText(selector.prefix ?? "");
    const suffix = normalizeQuoteText(selector.suffix ?? "");
    let bestScore = -1;
    for (const at of found) {
      const score = commonSuffixLength(parent.text.slice(Math.max(0, at - prefix.length - 1), at).trimEnd(), prefix)
        + commonPrefixLength(parent.text.slice(at + quote.length, at + quote.length + suffix.length + 1).trimStart(), suffix);
      if (score > bestScore) {
        bestScore = score;
        best = at;
      }
    }
  }
  return { kind: "range", start: parent.starts[best], end: parent.ends[best + quote.length - 1] };
}

// --- reading the format -----------------------------------------------------

function propertyValue(properties: readonly (readonly [string, string])[], key: string): string | null {
  for (const [k, v] of properties) if (propertyKeyNorm(k) === key) return v.trim();
  return null;
}

/** The selector a block's properties carry, or null when it has no `quote::`. */
export function quoteSelectorOf(properties: readonly (readonly [string, string])[]): QuoteSelector | null {
  const quote = propertyValue(properties, COMMENT_QUOTE_KEY);
  if (quote === null) return null;
  return {
    quote,
    prefix: propertyValue(properties, COMMENT_QUOTE_PREFIX_KEY) || null,
    suffix: propertyValue(properties, COMMENT_QUOTE_SUFFIX_KEY) || null,
  };
}

/** The block's `author::`, or null for the graph owner (absent or empty). */
export function authorOf(properties: readonly (readonly [string, string])[]): string | null {
  return propertyValue(properties, AUTHOR_KEY) || null;
}

/** A block is a comment iff it has a `quote::` property and a parent block. */
export function isCommentBlock(properties: readonly (readonly [string, string])[], hasParentBlock: boolean): boolean {
  return hasParentBlock && quoteSelectorOf(properties) !== null;
}

/** Keys shown as the comment header or author chip instead of property rows. */
export function isCommentPresentationKey(key: string, comment: boolean): boolean {
  const k = propertyKeyNorm(key);
  if (k === AUTHOR_KEY) return true;
  return comment && (k === COMMENT_QUOTE_KEY || k === COMMENT_QUOTE_PREFIX_KEY || k === COMMENT_QUOTE_SUFFIX_KEY);
}

/** Raw text of a new, empty-bodied comment: an empty first line (the caret's
 * line) followed by the selector's properties, through the parser's one
 * property writer for either format. */
export function commentRaw(selector: QuoteSelector, format: Format): string {
  let raw = editBlock("\n", format, { kind: "property", key: COMMENT_QUOTE_KEY, value: selector.quote });
  if (selector.prefix) raw = editBlock(raw, format, { kind: "property", key: COMMENT_QUOTE_PREFIX_KEY, value: selector.prefix });
  if (selector.suffix) raw = editBlock(raw, format, { kind: "property", key: COMMENT_QUOTE_SUFFIX_KEY, value: selector.suffix });
  return raw;
}
