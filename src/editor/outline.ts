import { blockRegions } from "../render/parse";
import { utf8ToUtf16Cursor } from "../render/utf16Cursor";
import type { Format } from "../render/ast";

// Parse pasted text into an outline tree (paste-as-blocks). Handles both a
// Logseq outline (every line a `- ` bullet, indentation = nesting, continuation
// lines indented to the bullet's content column) AND arbitrary markdown / plain
// text where headings, paragraphs and `- ` list items are intermixed.
//
// The guiding rule is LOSSLESS: every non-blank line ends up in some block,
// never silently dropped or merged into an unrelated one. A non-bullet line is
// treated as a *continuation* of the preceding bullet only when it is indented
// to at least that bullet's content column with no blank line in between
// (matching how Logseq writes multi-line block bodies); otherwise it becomes
// its own block at its own indentation depth.

export interface OutlineNode {
  raw: string;
  children: OutlineNode[];
}

/** One-based outline nesting ceiling shared with Rust page admission and save.
 *  Inserters, indent and reparenting refuse a deeper result. Callouts, quote
 *  markers and inline delimiters have independent parser safety ceilings. */
export const OUTLINE_MAX_DEPTH = 128;
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

function leadingWs(line: string): number {
  let i = 0;
  while (i < line.length && (line[i] === " " || line[i] === "\t")) i++;
  return i;
}

function bullet(line: string): { col: number; contentStart: number; content: string } | null {
  const col = leadingWs(line);
  const rest = line.slice(col);
  const marker = /^(?:[-+*]|\d+[.)])(?:\s+|$)/.exec(rest);
  if (marker) return { col, contentStart: col + marker[0].length, content: rest.slice(marker[0].length) };
  return null;
}

function tableDelimiter(line: string): boolean {
  const cells = line.trim().replace(/^\|/, "").replace(/\|$/, "").split("|");
  return cells.length >= 2 && cells.every((cell) => /^\s*:?-{3,}:?\s*$/.test(cell));
}

function tableRow(line: string): boolean {
  return line.includes("|") && line.trim().length > 1;
}

function stripWs(line: string, n: number): string {
  let i = 0;
  while (i < n && i < line.length && (line[i] === " " || line[i] === "\t")) i++;
  return line.slice(i);
}

interface Frame {
  col: number; // indentation of this node's marker / first char
  contentStart: number; // column a continuation line must reach to join this node
  kind: "bullet" | "block";
  node: OutlineNode;
}

/** Returns no nodes for text over `OUTLINE_MAX_SOURCE_CHARS` (callers treat an
 *  empty outline as nothing to insert). */
export function parseOutline(text: string, format: Format = "md"): OutlineNode[] {
  if (text.length > OUTLINE_MAX_SOURCE_CHARS) return [];
  const normalized = text.replace(/\r\n/g, "\n").replace(/\r/g, "\n");
  const lines = normalized.split("\n");
  const literals = blockRegions(normalized, format).literals;
  const starts = [0];
  const utf8 = new TextEncoder();
  for (const line of lines) starts.push(starts.at(-1)! + utf8.encode(line).length + 1);
  let literalIndex = 0;
  const literalAt = (at: number) => {
    while (literalIndex < literals.length && literals[literalIndex][1] <= at) literalIndex++;
    const range = literals[literalIndex];
    return range && range[0] <= at && at < range[1] ? range : null;
  };
  const roots: OutlineNode[] = [];
  const stack: Frame[] = [];
  let sawBlank = false;

  // Attach `node` at indentation `col`: pop deeper/equal frames, then nest under
  // the remaining top (or make it a root).
  const place = (col: number, node: OutlineNode) => {
    while (stack.length && stack[stack.length - 1].col >= col) stack.pop();
    if (stack.length) stack[stack.length - 1].node.children.push(node);
    else roots.push(node);
  };

  for (let lineIndex = 0; lineIndex < lines.length; lineIndex++) {
    const line = lines[lineIndex];
    // Test the first content byte, so an inline literal after a list marker
    // does not hide that marker. Extents, including blank lines, are parser-owned.
    const literal = literalAt(starts[lineIndex] + leadingWs(line));
    if (literal) {
      let next = lineIndex + 1;
      while (next < lines.length && starts[next] < literal[1]) next++;
      const raw = lines.slice(lineIndex, next).join("\n");
      const indent = leadingWs(line);
      const top = stack.at(-1);
      if (top?.kind === "bullet" && !sawBlank && indent >= top.contentStart) {
        top.node.raw += "\n" + raw;
      } else {
        const node: OutlineNode = { raw, children: [] };
        place(indent, node);
        stack.push({ col: indent, contentStart: indent, kind: "block", node });
      }
      lineIndex = next - 1;
      sawBlank = false;
      continue;
    }
    // A GFM table is one Markdown content unit. Keeping its contiguous rows in a
    // single block prevents the generic line parser from turning every row into
    // unrelated sibling blocks (GH #58).
    if (tableRow(line) && lineIndex + 1 < lines.length && tableDelimiter(lines[lineIndex + 1])) {
      const indent = leadingWs(line);
      const table = [line.trim()];
      let next = lineIndex + 1;
      while (next < lines.length && !literalAt(starts[next] + leadingWs(lines[next])) && tableRow(lines[next])) {
        table.push(lines[next].trim());
        next += 1;
      }
      const node: OutlineNode = { raw: table.join("\n"), children: [] };
      place(indent, node);
      stack.push({ col: indent, contentStart: indent, kind: "block", node });
      lineIndex = next - 1;
      sawBlank = false;
      continue;
    }
    const b = bullet(line);
    if (b) {
      const node: OutlineNode = { raw: b.content, children: [] };
      place(b.col, node);
      stack.push({ col: b.col, contentStart: b.contentStart, kind: "bullet", node });
      sawBlank = false;
      continue;
    }
    if (line.trim().length === 0) {
      sawBlank = true;
      continue;
    }
    const indent = leadingWs(line);
    const top = stack.length ? stack[stack.length - 1] : null;
    // Continuation of the current bullet: indented into its body, no blank gap.
    if (top && top.kind === "bullet" && !sawBlank && indent >= top.contentStart) {
      top.node.raw += "\n" + stripWs(line, top.contentStart);
      continue;
    }
    // Otherwise its own block (heading / paragraph line / loose text). Like a
    // flat plain-text paste, a block never absorbs the lines that follow it.
    const node: OutlineNode = { raw: line.trim(), children: [] };
    place(indent, node);
    stack.push({ col: indent, contentStart: indent, kind: "block", node });
    sawBlank = false;
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
