/** Code-fence state for the editor's raw-text decisions. Every answer is the parser's (I-12): lsdoc's
 *  accepted literal containers (`blockRegions(raw).literal_blocks`, fences, `#+BEGIN_SRC`, examples)
 *  presented in UTF-16, plus the one named editor-state policy of Rust `block_regions::open_fence`
 *  for a fence still being typed (no closer yet, so lsdoc forms no container). Nothing here
 *  recognizes a fence opener or closer: closer length/character, trailing blank lines, and which
 *  openers count (a `- ```js` line does not) are lsdoc's and OG's. Costs one cached parse of the
 *  text, skipped when the text holds none of the fence delimiters. */
import { blockRegions, parserReady } from "../render/parse";
import { utf8ToUtf16Cursor } from "../render/utf16Cursor";
import type { Format } from "../render/ast";

export interface LiteralContainer {
  kind: "src" | "example" | "other";
  /** Lower-cased language id of the info string ("" when none). */
  lang: string;
  /** UTF-16 offsets: whole container (trailing blank lines included, to the end of the text for an open fence). */
  start: number;
  end: number;
  /** After the opener line's newline; `raw.length + 1` when the opener ends the text. */
  openEnd: number;
  /** Start of the closer line; `raw.length + 1` for an open fence (the caret may sit at the very end). */
  closeStart: number;
  /** End of the opener's delimiter token; the info string follows it. */
  delimEnd: number;
  closed: boolean;
}

const clamp = (n: number, lo: number, hi: number) => Math.max(lo, Math.min(hi, n));
let last: { raw: string; format: Format; fences: LiteralContainer[] } | undefined;

/** The code containers (fences, source, example) of `raw` in source order, closed ones first-class,
 *  an open fence last. Empty before the parser is ready or for a quarantined block. */
export function codeFences(raw: string, format: Format = "md"): LiteralContainer[] {
  if (!parserReady()) return [];
  if (last?.raw === raw && last.format === format) return last.fences;
  let fences: LiteralContainer[] = [];
  // Cheap pre-filter: a fence delimiter run holds a doubled character, so text with none of these
  // substrings has no fence container and no open fence (lsdoc decides the rest).
  if (raw.includes("``") || raw.includes("~~") || raw.includes("#+")) {
    const regions = blockRegions(raw, format);
    if (!regions.quarantined) {
      const at = utf8ToUtf16Cursor(raw);
      for (const b of regions.literal_blocks) {
        if (b.kind === "other") continue;
        const start = at(b.range[0]);
        const delimEnd = at(b.delim_end);
        const openEnd = at(b.open_end);
        const closeStart = at(b.close_start);
        fences.push({ kind: b.kind, lang: b.lang.toLowerCase(), start, end: at(b.range[1]), openEnd, closeStart, delimEnd, closed: true });
      }
      const o = regions.open_fence;
      if (o) {
        const start = at(o.start);
        const delimEnd = at(o.delim_end);
        const openEnd = at(o.open_end);
        fences.push({ kind: "src", lang: o.lang.toLowerCase(), start, end: raw.length, openEnd: raw[openEnd - 1] === "\n" ? openEnd : raw.length + 1, closeStart: raw.length + 1, delimEnd, closed: false });
      }
    }
  }
  last = { raw, format, fences };
  return fences;
}

/** Whether a textarea caret offset is inside a code fence's content: strictly between the opener
 *  line and the closer line, or after the opener of a fence still being typed. */
export function caretInFence(raw: string, offset: number, format: Format = "md"): boolean {
  const at = clamp(offset, 0, raw.length);
  return codeFences(raw, format).some((f) => f.openEnd <= at && at < f.closeStart);
}

/** Whether a line that starts at `lineStart` begins inside a code fence (its closer line counts as
 *  inside), for completion triggers that must not fire in code. */
export function lineStartsInFence(raw: string, lineStart: number, format: Format = "md"): boolean {
  return codeFences(raw, format).some((f) => f.openEnd <= lineStart && lineStart <= f.closeStart);
}

/** Whether the caret is on an opening delimiter line after the delimiter token. The delimiter line
 * is outside `caretInFence` by design, but Enter/paste there must continue the source block instead
 * of splitting the outline (OG's `thing-at-point = source-block` behavior). */
export function caretOnOpeningFence(raw: string, offset: number, format: Format = "md"): boolean {
  const at = clamp(offset, 0, raw.length);
  return codeFences(raw, format).some((f) => f.delimEnd <= at && at < f.openEnd);
}

/** The double-Enter exit of a CLOSED fence: with the caret on a blank line directly above the
 *  closer line and nothing but blank lines after the closer, the text without that sentinel line. */
export function fenceExitTrim(text: string, lineStart: number, lineEnd: number, format: Format = "md"): string | null {
  const fence = codeFences(text, format).find((f) => f.closed && f.openEnd <= lineStart && f.closeStart === lineEnd + 1);
  if (!fence) return null;
  const closerEnd = text.indexOf("\n", fence.closeStart);
  if (closerEnd !== -1 && text.slice(closerEnd + 1).trim() !== "") return null;
  return text.slice(0, lineStart - 1) + text.slice(lineEnd);
}

const MATH_DELIM = "$$";

/** Advance display-math state across text known not to be inside a code fence.
 * Every `$$` toggles, so `$$x$$` on one line opens and closes again (net
 * outside) while a lone `$$` opens a multi-line environment. */
function toggleDisplayMath(open: boolean, text: string): boolean {
  let at = text.indexOf(MATH_DELIM);
  while (at !== -1) {
    open = !open;
    at = text.indexOf(MATH_DELIM, at + MATH_DELIM.length);
  }
  return open;
}

/** Display-math state after `raw[0, limit)`, ignoring everything inside code fences. */
function displayMathOpenBefore(raw: string, limit: number): boolean {
  let open = false;
  let from = 0;
  for (const f of codeFences(raw)) {
    if (f.start >= limit) break;
    open = toggleDisplayMath(open, raw.slice(from, f.start));
    from = Math.max(from, Math.min(f.end, limit));
  }
  return toggleDisplayMath(open, raw.slice(from, limit));
}

/** Whether a caret offset sits inside an open `$$ … $$` display-math environment.
 *
 * This is a DELIBERATE DIVERGENCE FROM OG, not a parity fix (GH #278, Martin's
 * call). OG's Enter dwim recognises only ``` and `#+BEGIN_`
 * (`frontend/util/thingatpt.cljs` `admonition&src-at-point`), so pressing Enter
 * inside `$$ … $$` splits the bullet and breaks the environment there too. A
 * multi-line display-math block is one piece of content, so Tine keeps Enter
 * inside it the way it does for a code fence.
 *
 * Confined to the editor's Enter decision on purpose: the parser, the bytes on
 * disk, and property classification (`classifyLines`) are untouched, so nothing
 * about how a block is stored or read changes. `$$` inside a code fence is
 * literal text and opens nothing. */
export function caretInDisplayMath(raw: string, offset: number): boolean {
  const at = clamp(offset, 0, raw.length);
  const inFenceLines = codeFences(raw).some((f) => {
    const closerEnd = f.closed ? raw.indexOf("\n", f.closeStart) : -1;
    return f.start <= at && at <= (closerEnd === -1 ? raw.length : closerEnd);
  });
  return !inFenceLines && displayMathOpenBefore(raw, at);
}

/** Display-math state after consuming `text` whole — for the double-Enter exit,
 * which asks "was the environment open before this blank line?". */
export function displayMathOpenAfter(text: string): boolean {
  return displayMathOpenBefore(text, text.length);
}

/** Whether a line closes an open display-math environment. */
export function closesDisplayMath(line: string): boolean {
  return line.includes(MATH_DELIM);
}
