import { blockRegions } from "../render/parse";
import type { Format } from "../render/ast";
import { OUTLINE_MAX_SOURCE_CHARS, type OutlineNode } from "./outline";

// Read the Markdown that HTML paste converts the clipboard's HTML into as an
// outline tree. This is the HTML route's own reader, kept as it was before
// plain-text paste began reading text as a file read does (`parseOutline`).
// Two readers of outline text is a known second answerer; it goes when HTML
// paste emits an outline that `parseOutline` reads.
//
// It handles both a Logseq outline (every line a `- ` bullet, indentation =
// nesting, continuation lines indented to the bullet's content column) AND
// arbitrary markdown where headings, paragraphs and `- ` list items are
// intermixed. The guiding rule is LOSSLESS: every non-blank line ends up in
// some block, never silently dropped or merged into an unrelated one. A
// non-bullet line is treated as a *continuation* of the preceding bullet only
// when it is indented to at least that bullet's content column with no blank
// line in between; otherwise it becomes its own block at its own indentation
// depth.

function leadingWs(line: string): number {
  let i = 0;
  while (i < line.length && (line[i] === " " || line[i] === "\t")) i++;
  return i;
}

function bullet(line: string, format: Format): { col: number; contentStart: number; content: string } | null {
  const col = leadingWs(line);
  const rest = line.slice(col);
  // An Org headline's level is its stars, not its indentation (OG parses the
  // paste with mldoc). Level 1 falls through to the shared marker below.
  const stars = format === "org" ? /^\*{2,}(?:\s+|$)/.exec(rest) : null;
  if (stars) {
    const level = stars[0].trimEnd().length;
    return { col: col + level - 1, contentStart: col + stars[0].length, content: rest.slice(stars[0].length) };
  }
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
export function parseHtmlOutlineMarkdown(text: string, format: Format = "md"): OutlineNode[] {
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
    const b = bullet(line, format);
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
