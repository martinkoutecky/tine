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

function fenceMarker(line: string): string | null {
  const match = /^\s*(`{3,}|~{3,})/.exec(line);
  return match?.[1] ?? null;
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

export function parseOutline(text: string): OutlineNode[] {
  const lines = text.replace(/\r\n/g, "\n").replace(/\r/g, "\n").split("\n");
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
    const fence = fenceMarker(line);
    if (fence) {
      const indent = leadingWs(line);
      const fenced = [line.trim()];
      let next = lineIndex + 1;
      const close = new RegExp(`^\\s*${fence[0]}{${fence.length},}\\s*$`);
      while (next < lines.length) {
        fenced.push(stripWs(lines[next], indent));
        if (close.test(lines[next])) {
          next += 1;
          break;
        }
        next += 1;
      }
      const node: OutlineNode = { raw: fenced.join("\n"), children: [] };
      place(indent, node);
      stack.push({ col: indent, contentStart: indent, kind: "block", node });
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
      while (next < lines.length && tableRow(lines[next])) {
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
