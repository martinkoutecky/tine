import { blockRegions, pastedOutline, outlineMaxDepth as kernelOutlineMaxDepth } from "../render/parse";
import { utf8ToUtf16Cursor } from "../render/utf16Cursor";
import type { Format } from "../render/ast";

// Parse pasted text into an outline tree (paste-as-blocks). The blocks, their
// nesting and their bodies are exactly what the page reader reads from the same
// text as a file (`pastedOutline`): every line from a block's header up to the
// next header is that block's, blank lines, tables and list items included, and
// lsdoc decides which lines are headers, so a bullet-looking line inside literal
// source stays text. Text before the first header, the page text a file would
// keep above its blocks, becomes one block of its own when it is not blank.

export interface OutlineNode {
  raw: string;
  children: OutlineNode[];
}

/** One-based outline nesting ceiling: the outline kernel's
 *  `outlineMaxDepth()`, which tine-store page admission also uses (twin 4).
 *  Inserters, indent and reparenting refuse a deeper result. Callouts, quote
 *  markers and inline delimiters have independent parser safety ceilings. Read
 *  from the kernel once; structural edits need the parser, so before it is
 *  ready this refuses as they do. */
let maxDepth: number | null = null;
export function outlineMaxDepth(): number {
  if (maxDepth === null) {
    const depth = kernelOutlineMaxDepth();
    if (depth === null) throw new Error("Structural edit refused: parser is not ready");
    maxDepth = depth;
  }
  return maxDepth;
}
/** Longest outline text parsed (UTF-16 units). A UTF-8 file is never shorter in
 *  bytes, so this cannot refuse anything the 64 MiB file admission accepts. */
export const OUTLINE_MAX_SOURCE_CHARS = 64 * 1024 * 1024;

/** Depth of an outline forest (1 for a flat list, 0 for none). Iterative. */
export function outlineDepth(nodes: readonly OutlineNode[]): number {
  let max = 0;
  const pending = nodes.map((node) => ({ node, depth: 1 }));
  while (pending.length) {
    const { node, depth } = pending.pop()!;
    max = Math.max(max, depth);
    for (const child of node.children) pending.push({ node: child, depth: depth + 1 });
  }
  return max;
}

/** Returns no nodes for text over `OUTLINE_MAX_SOURCE_CHARS` (callers treat an
 *  empty outline as nothing to insert). */
export function parseOutline(text: string, format: Format = "md"): OutlineNode[] {
  if (text.length > OUTLINE_MAX_SOURCE_CHARS) return [];
  const [pre, blocks] = pastedOutline(text, format);
  const roots: OutlineNode[] = pre?.trim() ? [{ raw: pre, children: [] }] : [];
  const path: OutlineNode[] = [];
  for (const [depth, raw] of blocks) {
    const node: OutlineNode = { raw, children: [] };
    (depth ? path[depth - 1].children : roots).push(node);
    path[depth] = node;
  }
  return roots;
}

// --- Plain-text paste classification. ONE module answers "what outline does pasted text hold", so the
// decision, the outline parse and the paragraph split agree on which text is literal (a bullet-looking
// or blank line inside a fenced payload is code, not structure; I-12). The classifier is OG's:
// 6e7afa8eb src/main/frontend/handler/paste.cljs:101-107 (Markdown: -, +, *, ATX headings; Org: stars),
// paragraphs split on blank lines and trimmed, paste.cljs:34-47,173-174. NAMED OG DIVERGENCE: OG applies
// both to the raw text; here a line or break inside literal source (lsdoc's answer) is not counted.

const BLOCKS_MD = /^\s*(?:[-+*]|#+)\s+/gm;
const BLOCKS_ORG = /^\s*\*+\s+/gm;
const PARAGRAPH_BREAK = /(?:\r?\n){2,}/g;

/** UTF-16 ranges of literal source in `text`, each ending at its last non-blank character. */
function literalSpans(text: string, format: Format): [number, number][] {
  const at = utf8ToUtf16Cursor(text);
  return blockRegions(text, format).literals.map(([a, b]) => {
    const start = at(a);
    return [start, start + text.slice(start, at(b)).trimEnd().length];
  });
}

/** The blocks pasted plain text makes, or null when it should be inserted as text: an outline when a
 *  non-literal line starts a bullet/heading, else the blank-line-separated paragraphs when a break
 *  lies outside literal source, else null. At most `OUTLINE_MAX_SOURCE_CHARS` is considered. */
export function pastedPlainBlocks(text: string, format: Format): OutlineNode[] | null {
  const spans = literalSpans(text, format);
  const literalAt = (at: number) => spans.some(([a, b]) => a <= at && at < b);
  const marker = format === "org" ? BLOCKS_ORG : BLOCKS_MD;
  for (const hit of text.matchAll(marker)) {
    if (!literalAt(hit.index + hit[0].length - hit[0].trimStart().length)) return parseOutline(text, format);
  }
  const paragraphs: OutlineNode[] = [];
  let from = 0;
  for (const hit of text.matchAll(PARAGRAPH_BREAK)) {
    if (spans.some(([a, b]) => a < hit.index && hit.index < b)) continue;
    paragraphs.push({ raw: text.slice(from, hit.index).trim(), children: [] });
    from = hit.index + hit[0].length;
  }
  if (!paragraphs.length) return null;
  paragraphs.push({ raw: text.slice(from).trim(), children: [] });
  return paragraphs;
}
