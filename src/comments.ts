// Margin dialogue, slice 1 (vision 2026-10 §3.7): a comment is an ordinary child
// block that carries `tine.quote::` (the text it is about). Its own children are its
// thread. Agents attribute what they write with `tine.author::`; a block without it is
// the graph owner's. The keys are `tine.*` so a user's own `quote::` / `author::`
// (book notes) is never mistaken for one (D-20261102-PNS, Discussion #825; the
// Beta-only bare keys have no reader on purpose). Everything here is derived from the block's properties at
// render time: nothing is persisted beyond the ordinary outline, and no offset is
// ever stored (the quote is re-anchored against the parent's text on each render).
//
// Authoring keys live here; the kernel owns the comment discriminator used by
// both quoteSelectorOf and the editor's hidden-property projection.

import { editBlock, editorText, parserReady, rawByteOfEditor } from "./render/parse";
import { propertyKeyNorm } from "./propertyKey";
import type { Format } from "./render/ast";
import { comment_quote } from "./render/wasm/lsdoc_wasm.js";
import { utf8ToUtf16Cursor } from "./render/utf16Cursor";
import { appNow } from "./journal";

export const COMMENT_QUOTE_KEY = "tine.quote";
export const COMMENT_QUOTE_PREFIX_KEY = "tine.quote-prefix";
export const COMMENT_QUOTE_SUFFIX_KEY = "tine.quote-suffix";
export const AUTHOR_KEY = "tine.author";
export const COMMENT_RESOLVED_KEY = "tine.resolved";
export const COMMENT_DATE_KEY = "tine.date";
/** Initial context budget; repeated passages can require more (GH #891). */
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
  | { kind: "stale" }
  | { kind: "ambiguous" };

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

export function normalizeWithMap(raw: string): Normalized {
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

/** Prose boundaries come from the kernel's property projection, so hidden
 * identity/author rows and Org drawer delimiters cannot move a boundary.
 * This never changes the source used to score old textual context. */
function contentBounds(raw: string, format: Format): { start: number; end: number } {
  if (!parserReady()) return { start: 0, end: raw.length };
  const policy = '{"kind":"all"}';
  const body = normalizeWithMap(editorText(raw, format, policy).visible);
  if (!body.text) return { start: 0, end: raw.length };
  const cursor = utf8ToUtf16Cursor(raw);
  return {
    start: cursor(rawByteOfEditor(raw, format, policy, body.starts[0])),
    end: cursor(rawByteOfEditor(raw, format, policy, body.ends[body.text.length - 1])),
  };
}

/** Drop a surrogate half left at either edge by a fixed-length cut. */
function wholeCodePoints(text: string): string {
  let start = 0;
  let end = text.length;
  if (end > 0 && text.charCodeAt(0) >= 0xdc00 && text.charCodeAt(0) <= 0xdfff) start = 1;
  if (end > start && text.charCodeAt(end - 1) >= 0xd800 && text.charCodeAt(end - 1) <= 0xdbff) end--;
  return text.slice(start, end);
}

export function occurrences(haystack: string, needle: string): number[] {
  const out: number[] = [];
  if (!needle) return out;
  for (let at = haystack.indexOf(needle); at !== -1; at = haystack.indexOf(needle, at + 1)) out.push(at);
  return out;
}

/** The selector for a text selection [start, end) of `editorText`, the editor's
 * view of the parent whose current raw text is `parentRaw`. The quote is the
 * normalised selection. Context starts at 32 chars, trimmed at word boundaries,
 * and grows until the occurrence is unique. Empty context marks a boundary;
 * null means unknown (including legacy comments with omitted context). Both
 * sides are always written when the quote occurs in the parent, so a
 * later second occurrence of the quoted words cannot re-anchor the comment
 * (ADR 0075, D-20261018-MC2). An empty selection quotes the whole block (`quote === ""`).
 * Cost O(parent length × occurrences × log(parent length)) in the repeated case. */
export function quoteSelectorFor(editorText: string, start: number, end: number, parentRaw: string, format: Format = "md"): QuoteSelector {
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
  if (found.length === 0) return { quote, prefix: null, suffix: null };
  const at = found[Math.min(ordinal, found.length - 1)];
  const after = at + quote.length;
  const bounds = contentBounds(parentRaw, format);
  const bodyStart = Math.max(0, parent.starts.findIndex(raw => raw >= bounds.start));
  const beyondBody = parent.ends.findIndex(raw => raw > bounds.end);
  const bodyEnd = beyondBody === -1 ? parent.text.length : beyondBody;
  // If properties themselves were selected, retain their ordinary raw context.
  const first = at >= bodyStart && after <= bodyEnd ? bodyStart : 0;
  const last = at >= bodyStart && after <= bodyEnd ? bodyEnd : parent.text.length;
  for (let size = QUOTE_CONTEXT_MAX; ; size *= 2) {
    const from = Math.max(first, at - size);
    const to = Math.min(last, after + size);
    const prefix = wordBoundaryStart(parent.text, from, wholeCodePoints(parent.text.slice(from, at))).trim();
    const suffix = wordBoundaryEnd(parent.text, to, wholeCodePoints(parent.text.slice(after, to))).trim();
    const selector = { quote, prefix, suffix };
    const anchor = anchorQuote(parentRaw, selector, format);
    if ((anchor.kind === "range" && anchor.start === parent.starts[at]) || (from === first && to === last)) return selector;
  }
}

/** `cut` starts at `from` in `text`; drop a word the cut split, unless that word is all of it. */
function wordBoundaryStart(text: string, from: number, cut: string): string {
  if (from === 0 || /\s/.test(text[from - 1]) || /^\s/.test(cut)) return cut;
  const space = cut.search(/\s/);
  const kept = space === -1 ? "" : cut.slice(space);
  return kept.trim() ? kept : cut;
}

/** `cut` ends at `to` in `text`; drop a word the cut split, unless that word is all of it. */
function wordBoundaryEnd(text: string, to: number, cut: string): string {
  if (to >= text.length || /\s/.test(text[to]) || /\s$/.test(cut)) return cut;
  const m = /\s\S*$/.exec(cut);
  const kept = m ? cut.slice(0, m.index) : "";
  return kept.trim() ? kept : cut;
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
 * surroundings agree best with the stored prefix and suffix. Explicit empty
 * properties break textual ties at the corresponding block boundary; old
 * writers omitted empty context, so absence never implies a boundary. A tie
 * is ambiguous, an empty quote whole, a missing quote stale.
 * Never persisted. Cost O(parent length × occurrences). */
export function anchorQuote(parentRaw: string, selector: QuoteSelector, format: Format = "md"): QuoteAnchor {
  const quote = normalizeQuoteText(selector.quote);
  if (!quote) return { kind: "whole" };
  const parent = normalizeWithMap(parentRaw);
  const found = occurrences(parent.text, quote);
  if (found.length === 0) return { kind: "stale" };
  let best = found[0];
  let tied = false;
  if (found.length > 1) {
    const prefix = normalizeQuoteText(selector.prefix ?? "");
    const suffix = normalizeQuoteText(selector.suffix ?? "");
    const bounds = selector.prefix === "" || selector.suffix === "" ? contentBounds(parentRaw, format) : null;
    let bestScore = -1;
    for (const at of found) {
      const textScore = commonSuffixLength(parent.text.slice(Math.max(0, at - prefix.length - 1), at).trimEnd(), prefix)
        + commonPrefixLength(parent.text.slice(at + quote.length, at + quote.length + suffix.length + 1).trimStart(), suffix);
      const boundaryScore = Number(selector.prefix === "" && parent.starts[at] === bounds?.start)
        + Number(selector.suffix === "" && parent.ends[at + quote.length - 1] === bounds?.end);
      // Surviving text is stronger evidence than a boundary after later edits.
      const score = textScore * 3 + boundaryScore;
      if (score > bestScore) {
        bestScore = score;
        best = at;
        tied = false;
      } else if (score === bestScore) tied = true;
    }
  }
  return tied ? { kind: "ambiguous" } : { kind: "range", start: parent.starts[best], end: parent.ends[best + quote.length - 1] };
}

// --- reading the format -----------------------------------------------------

function propertyValue(properties: readonly (readonly [string, string])[], key: string): string | null {
  for (const [k, v] of properties) if (propertyKeyNorm(k) === key) return v.trim();
  return null;
}

/** The selector a block's properties carry, or null when it has no `tine.quote::`. */
export function quoteSelectorOf(properties: readonly (readonly [string, string])[]): QuoteSelector | null {
  const quote = comment_quote(JSON.stringify(properties));
  if (quote == null) return null;
  return {
    quote: quote.trim(),
    prefix: propertyValue(properties, COMMENT_QUOTE_PREFIX_KEY),
    suffix: propertyValue(properties, COMMENT_QUOTE_SUFFIX_KEY),
  };
}

/** The block's `tine.author::`, or null for the graph owner (absent or empty). */
export function authorOf(properties: readonly (readonly [string, string])[]): string | null {
  return propertyValue(properties, AUTHOR_KEY) || null;
}

export function resolvedOf(properties: readonly (readonly [string, string])[]): boolean {
  return propertyValue(properties, COMMENT_RESOLVED_KEY)?.toLowerCase() === "true";
}

/** Creation only: editing a message retains its existing hidden properties. */
export function stampCommentDate(raw: string, format: Format): string {
  return editBlock(raw, format, { kind: "property", key: COMMENT_DATE_KEY, value: appNow("instant").toISOString().slice(0, 16) + "Z" });
}

export function commentDateOf(properties: readonly (readonly [string, string])[]): string | null {
  const value = propertyValue(properties, COMMENT_DATE_KEY);
  if (!value || !/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}Z$/.test(value)) return null;
  const date = new Date(value);
  return Number.isFinite(date.valueOf()) && date.toISOString().slice(0, 16) + "Z" === value ? value : null;
}

/** A block is a comment iff it has a `tine.quote::` property and a parent block. */
export function isCommentBlock(properties: readonly (readonly [string, string])[], hasParentBlock: boolean): boolean {
  return hasParentBlock && quoteSelectorOf(properties) !== null;
}

/** Keys shown as the comment header or author chip instead of property rows. */
export function isCommentPresentationKey(key: string, comment: boolean): boolean {
  const k = propertyKeyNorm(key);
  if (k === AUTHOR_KEY) return true;
  return comment && (k === COMMENT_QUOTE_KEY || k === COMMENT_QUOTE_PREFIX_KEY || k === COMMENT_QUOTE_SUFFIX_KEY
    || k === COMMENT_RESOLVED_KEY || k === COMMENT_DATE_KEY);
}

/** Raw text of a new, empty-bodied comment: an empty first line (the caret's
 * line) followed by the selector's properties and, when the user has set a
 * name (commentIdentity.ts), `tine.author::`, through the parser's one property
 * writer for either format. */
export function commentRaw(selector: QuoteSelector, format: Format, author: string = ""): string {
  let raw = editBlock("\n", format, { kind: "property", key: COMMENT_QUOTE_KEY, value: selector.quote });
  if (selector.prefix !== null) raw = editBlock(raw, format, { kind: "property", key: COMMENT_QUOTE_PREFIX_KEY, value: selector.prefix });
  if (selector.suffix !== null) raw = editBlock(raw, format, { kind: "property", key: COMMENT_QUOTE_SUFFIX_KEY, value: selector.suffix });
  if (author) raw = editBlock(raw, format, { kind: "property", key: AUTHOR_KEY, value: author });
  return raw;
}

/** Raw text of a new, empty reply written by the user: an empty first line (the
 * caret's line) and the user's `tine.author::`. Only called with a name set. */
export function replyRaw(format: Format, author: string): string {
  return editBlock("\n", format, { kind: "property", key: AUTHOR_KEY, value: author });
}
