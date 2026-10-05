/** Is a UTF-16 range of an editor buffer literal source text (inline code, verbatim, code containers)?
 *  Every answer is lsdoc's, through the block-region door (I-12): `blockRegions(text).literals`.
 *  Nothing here recognizes a backtick, tilde or fence; the one editor-state policy is a span still
 *  being typed. lsdoc forms no literal from an unclosed opener, so "would the next delimiter, typed
 *  here, close a literal that already contains the range?" is asked of lsdoc too, by parsing the
 *  text with that delimiter spliced in. The candidates are the formats' inline-literal delimiters
 *  (Markdown backtick runs; Org `~` and `=`), tried only when the range is not literal as it stands.
 *  Costs one cached parse, plus up to two more when no literal covers the range; callers invoke it
 *  only once a typography trigger has completed. Before the parser is ready, or for a quarantined
 *  block, the range counts as literal (the edit refuses rather than rewrites). */
import { blockRegions, parserReady } from "../render/parse";
import { utf8ToUtf16Cursor } from "../render/utf16Cursor";
import type { Format } from "../render/ast";

const CLOSERS: Record<Format, readonly string[]> = { md: ["`", "``"], org: ["~", "="] };

function literalRanges(text: string, format: Format): [number, number][] {
  const at = utf8ToUtf16Cursor(text);
  return blockRegions(text, format).literals.map(([a, b]) => [at(a), at(b)]);
}

export function rangeInLiteral(text: string, format: Format, from: number, to: number): boolean {
  if (!parserReady()) return true;
  if (literalRanges(text, format).some(([a, b]) => a < to && b > from)) return true;
  // A literal that opens strictly before the range and closes after it once its closer is typed.
  for (const closer of CLOSERS[format]) {
    const closed = text.slice(0, to) + closer + text.slice(to);
    if (literalRanges(closed, format).some(([a, b]) => a < from && b > from)) return true;
  }
  return false;
}

/** Whether UTF-16 offset `at` of `text` lies inside literal source (a parser literal as it stands). */
export function offsetInLiteral(text: string, format: Format, at: number): boolean {
  if (!parserReady()) return true;
  return literalRanges(text, format).some(([a, b]) => a <= at && at < b);
}
