// A block's text direction (GH #823): automatic, derived only from the block's
// text, never stored. Nothing here writes to a note; a graph stays byte-identical.
//
// The browser's own `dir="auto"` looks at the first strong character of the
// source, and a Persian task's source starts with the Latin `TODO`. So direction
// comes from what the reader sees: the parser says which runs are prose. A view
// and its editor call the same function on the same raw, so a block cannot jump
// between them.
//
// Cost: one regex scan of the raw per evaluation, O(raw bytes), no allocation. A
// block with no right-to-left character stops there and gets no attribute at all,
// so a left-to-right graph takes no parse and no DOM change. Only a block that has
// one (or any block under the mirrored note layout) reads the parse, which the
// renderer has already cached for that raw.
import type { Block, Format, Inline } from "./ast";
import { parseBody } from "./facets";
import { parserReady } from "./parse";
import { noteLayoutRtl } from "../noteLayoutSetting";

export type TextDirection = "rtl" | "ltr";

// Bidi classes R and AL by range, plus the two right-to-left format marks.
// U+FEFF (a byte order mark) sits just past the Arabic presentation forms.
const RTL_RANGES = "\\u0590-\\u08FF\\u200F\\uFB1D-\\uFDFF\\uFE70-\\uFEFC\\u{10800}-\\u{10FFF}\\u{1E800}-\\u{1EFFF}";
const ANY_RTL = new RegExp(`[${RTL_RANGES}]`, "u");
const RTL_START = new RegExp(`^[${RTL_RANGES}]`, "u");
// Letters plus the explicit marks: digits, punctuation and emoji are not a language.
const FIRST_STRONG = /[\p{L}‎‏؜]/u;

/** Direction of the first strong character, or null when there is none. A leading
 *  U+200F or U+200E counts, which is the way to overrule the guess in one block. */
export function firstStrongDirection(text: string): TextDirection | null {
  const hit = FIRST_STRONG.exec(text);
  if (!hit) return null;
  return RTL_START.test(hit[0]) || hit[0] === "؜" ? "rtl" : "ltr";
}

interface Found { prose: TextDirection | null; label: TextDirection | null }

function scanInlines(inlines: readonly Inline[], found: Found, inLabel: boolean): void {
  for (const inline of inlines) {
    if (found.prose) return;
    switch (inline.k) {
      case "plain":
        if (inLabel) found.label ??= firstStrongDirection(inline.text);
        else found.prose = firstStrongDirection(inline.text);
        break;
      case "emphasis": case "subscript": case "superscript":
        scanInlines(inline.children, found, inLabel);
        break;
      // A link or tag names something: `[[Tine]] یک برنامه است` is a Persian
      // sentence. They decide only for a block with no prose of its own.
      case "link":
        if (inline.label?.length) scanInlines(inline.label, found, true);
        else if (inline.url.type === "page_ref") found.label ??= firstStrongDirection(inline.url.v);
        break;
      case "tag":
        scanInlines(inline.children, found, true);
        break;
      // Code, math, timestamps, macros and the rest are not the block's language.
      default:
    }
  }
}

function scanBlocks(blocks: readonly Block[], found: Found): void {
  for (const block of blocks) {
    if (found.prose) return;
    switch (block.kind) {
      case "paragraph": case "heading": case "bullet": case "footnote_def":
        scanInlines(block.inline, found, false);
        break;
      case "quote": case "custom":
        scanBlocks(block.children, found);
        break;
      case "list":
        for (const item of block.items) scanListItem(item, found);
        break;
      case "table":
        for (const cell of [...(block.header ?? []), ...block.rows.flat()]) scanInlines(cell, found, false);
        break;
      default:
    }
  }
}

function scanListItem(item: Extract<Block, { kind: "list" }>["items"][number], found: Found): void {
  scanBlocks(item.content, found);
  for (const child of item.items) scanListItem(child, found);
}

/** What the reader sees decides: prose first, then link and tag labels, else null
 *  (no language: inherit the note layout). Falls back to the raw text only before
 *  the parser is up. */
export function blockTextDirection(raw: string, format: Format): TextDirection | null {
  if (!parserReady()) return firstStrongDirection(raw);
  const found: Found = { prose: null, label: null };
  scanBlocks(parseBody(raw, format), found);
  return found.prose ?? found.label;
}

/** The `dir` attribute for a block's rendered body and for its editor. */
export function blockDirAttr(raw: string, format: Format, mirrored: boolean = noteLayoutRtl()): TextDirection | undefined {
  if (!mirrored && !ANY_RTL.test(raw)) return undefined;
  return blockTextDirection(raw, format) ?? undefined;
}

/** Does this element sit in a right-to-left context? Reads the `dir` attributes
 *  this feature sets, so it costs no style recalculation. */
export function isRtlContext(el: Element): boolean {
  return el.closest("[dir]")?.getAttribute("dir") === "rtl";
}

/** Is the interface itself mirrored? True only under a right-to-left language. */
export function rootIsRtl(): boolean {
  return typeof document !== "undefined" && document.documentElement.getAttribute("dir") === "rtl";
}

/** Textarea boundary fallback. There is no browser API for bidi caret affinity.
 * An opposite-direction boundary run can still have text on either visual side
 * of offset 0/length. Leave that movement to the browser; Up/Down still cross
 * blocks at the first/last visual row. Read direction only at candidate offsets. */
export function arrowLeave(key: string, el: Element, start: number, end: number, length: number): "backward" | "forward" | null {
  if (start !== end || (key !== "ArrowLeft" && key !== "ArrowRight")) return null;
  if (start !== 0 && start !== length) return null;
  const rtl = isRtlContext(el);
  const raw = (el as HTMLTextAreaElement).value ?? "";
  let boundary: TextDirection | null = null;
  if (start === 0) boundary = firstStrongDirection(raw);
  else {
    for (let i = raw.length - 1; i >= 0 && !boundary; i--) {
      const lowSurrogate = raw.charCodeAt(i) >= 0xDC00 && raw.charCodeAt(i) <= 0xDFFF;
      const character = lowSurrogate && i > 0 ? raw.slice(i - 1, i + 1) : raw[i];
      if (lowSurrogate && i > 0) i--;
      boundary = firstStrongDirection(character);
    }
  }
  if (boundary && (boundary === "rtl") !== rtl) return null;
  if (start === 0 && key === (rtl ? "ArrowRight" : "ArrowLeft")) return "backward";
  if (start === length && key === (rtl ? "ArrowLeft" : "ArrowRight")) return "forward";
  return null;
}
