// The ONE answer to "is raw line i literal content?" for every editor/document
// edit that rewrites or moves lines by shape: property writes (`key::`, org
// drawers, `id`), planning moves (SCHEDULED/DEADLINE hoist and normalize),
// repeater advance, priority prefixing, and the hidden-property split (I-12).
//
// A literal line belongs to a code/src/example/query/math/raw-HTML/LaTeX block,
// delimiters included. Such content is never rewritten or moved by these edits.
// This is a NAMED OG DIVERGENCE (C3 L13/L14): OG's `insert-property` hoists every
// line starting with SCHEDULED/DEADLINE and its property writers scan code text
// (util/property.cljs:244-248); og treats code as bytes a metadata edit must not
// touch. Pinned by src/editor/literalLines.test.ts.
//
// The answer is lsdoc's, the parser that renders the block and that the backend
// projects properties from, so the editor agrees with what the user sees (an
// UNCLOSED ``` is not code to lsdoc). Where the CommonMark fence rule sees a
// CLOSED fence that lsdoc ends early (mldoc lets a shorter ``` or a `~~~` close
// a longer fence), the line counts as literal too: every caller either refuses
// to rewrite/move a literal line or declines to read it as metadata, so the
// union errs toward leaving bytes alone. Without a loaded (or with a
// quarantined) parse, the closed CommonMark fences and closed `#+BEGIN_…#+END_`
// blocks alone answer.
import { isQuarantined, parseBlock, parserReady } from "../render/parse";
import { rebulletedSourceByteToRawByte, utf8ByteToUtf16Offset } from "../render/spans";
import type { Block, Format } from "../render/ast";
import { transitionFence, type FenceState } from "./fences";

const LITERAL_KINDS = new Set<Block["kind"]>(["src", "example", "displayed_math", "latex_env", "raw_html"]);
const LITERAL_CUSTOM = /^(?:query|export|comment)$/i;
// Cheap exit: none of the literal openers occurs, so no line is literal.
const CANDIDATE = /```|~~~|#\+begin_|\$\$|\\begin\{|^\s*</im;
const BEGIN = /^\s*#\+BEGIN_(SRC|EXAMPLE|EXPORT|COMMENT|QUERY)\b/i;

function isLiteralBlock(b: Block): boolean {
  return LITERAL_KINDS.has(b.kind) || (b.kind === "custom" && LITERAL_CUSTOM.test(b.name));
}

/** Per line of `raw`: the index of the contiguous literal run it belongs to, or
 *  -1. Two adjacent lines share a run iff their indices are equal and not -1,
 *  which is what "may a line be inserted between them" asks (two literal blocks
 *  that touch count as one run: conservatively, nothing is inserted between). */
export function literalBlockOfLine(raw: string, format: Format = "md"): number[] {
  const lines = raw.split("\n");
  const literal: boolean[] = new Array(lines.length).fill(false);
  if (!CANDIDATE.test(raw)) return literal.map(() => -1);
  closedFences(lines, literal);
  const blocks = parserReady() ? parseBlock(raw, format === "org") : null;
  if (!blocks || isQuarantined(blocks)) {
    closedBeginBlocks(lines, literal);
  } else {
    const lineOf = (sourceByte: number) => {
      const at = utf8ByteToUtf16Offset(raw, rebulletedSourceByteToRawByte(raw, sourceByte));
      let line = 0;
      for (let p = raw.indexOf("\n"); p !== -1 && p < at; p = raw.indexOf("\n", p + 1)) line++;
      return line;
    };
    for (const b of blocks) {
      if (!b.span || !isLiteralBlock(b)) continue;
      const last = lineOf(Math.max(b.span[0], b.span[1] - 1));
      for (let i = lineOf(b.span[0]); i <= last && i < lines.length; i++) literal[i] = true;
    }
  }
  // Number the contiguous literal runs.
  let run = 0;
  return literal.map((on, i) => (on ? (i > 0 && literal[i - 1] ? run : ++run) : -1));
}

/** CommonMark fences that CLOSE (an unclosed fence is not code to lsdoc). */
function closedFences(lines: string[], literal: boolean[]): void {
  let fence: FenceState | null = null;
  let open = -1;
  for (let i = 0; i < lines.length; i++) {
    const t = transitionFence(fence, lines[i]);
    if (t.opens) open = i;
    if (t.closes) for (let j = open; j <= i; j++) literal[j] = true;
    fence = t.next;
  }
}

function closedBeginBlocks(lines: string[], literal: boolean[]): void {
  for (let i = 0; i < lines.length; i++) {
    const begin = BEGIN.exec(lines[i]);
    if (!begin || literal[i]) continue;
    const end = new RegExp(`^\\s*#\\+END_${begin[1]}\\b`, "i");
    const close = lines.findIndex((l, j) => j > i && end.test(l));
    if (close === -1) continue;
    for (let j = i; j <= close; j++) literal[j] = true;
    i = close;
  }
}

/** Whether a new line may be inserted before line `k` (k = lines.length is the
 *  end) without landing inside a literal block. */
export function insertableBefore(literal: number[], k: number): boolean {
  return k <= 0 || k >= literal.length || literal[k] === -1 || literal[k - 1] !== literal[k];
}
