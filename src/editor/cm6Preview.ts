import { blockRegions, parseBlock } from "../render/parse";
import { utf8ToUtf16Cursor } from "../render/utf16Cursor";
import type { Block, Inline } from "../render/ast";

export interface PreviewSpan {
  from: number;
  to: number;
  kind: string;
  inline?: Inline;
  block?: Block;
  headerTags?: string[];
  label?: string;
}

const cache = new WeakMap<Block[], PreviewSpan[]>();

export function constructRevealed(span: { from: number; to: number }, selections: readonly { from: number; to: number }[]): boolean {
  return selections.some(r => r.from <= span.to && r.to >= span.from);
}

/** lsdoc is the sole syntax authority. CM6 receives UTF-16 ranges. Containers
 * and descendants share the same reveal policy, including header tokens.
 * Source stays in the document: decorations never rewrite boundary bytes. */
export function previewSpans(raw: string, format: "md" | "org"): PreviewSpan[] {
  const blocks = parseBlock(raw, format === "org");
  const cached = cache.get(blocks);
  if (cached) return cached;
  const result: PreviewSpan[] = [];
  const cursor = utf8ToUtf16Cursor(raw);
  // parseBlock re-bullets trimmed raw with "- " / "* "; region ranges
  // already use raw coordinates. Same mapping as render/spans.ts.
  const lead = new TextEncoder().encode(raw.slice(0, raw.length - raw.trimStart().length)).length;
  const regions = blockRegions(raw, format);
  if (regions.quarantined) return result;
  const add = (span: PreviewSpan) => {
    if (span.from >= 0 && span.to <= raw.length && span.from < span.to) result.push(span);
  };
  for (const [kind, range, label] of [
    ["task", regions.header.marker_range, regions.header.marker],
    ["priority", regions.header.priority_range, regions.header.priority],
  ] as const) {
    if (range && label) add({ from: cursor(range[0]), to: cursor(range[1]), kind, label });
  }
  const visit = (value: unknown): void => {
    if (Array.isArray(value)) { value.forEach(visit); return; }
    if (!value || typeof value !== "object") return;
    const n = value as Inline | Block;
    if (n.span) {
      const from = cursor(n.span[0] - 2 + lead);
      let to = cursor(n.span[1] - 2 + lead);
      if ("k" in n) {
        // Plain runs can own escapes. Only byte-identical plain text and
        // ordinary line breaks have nothing to conceal.
        if (n.k !== "break" && !(n.k === "plain" && raw.slice(from, to) === n.text)) {
          const kind = n.k === "emphasis" ? (n.emph === "Bold" ? "bold" : n.emph === "Italic" ? "italic" : n.emph)
            : n.k === "link" && n.url.type === "page_ref" ? "page" : n.k === "verbatim" ? "code" : n.k;
          add({ from, to, kind, inline: n });
        }
      } else if (n.kind !== "paragraph" && (n.kind !== "bullet" || n.size)) {
        // Leave the block's separator in the editor, including CRLF.
        while (to > from && (raw[to - 1] === "\n" || raw[to - 1] === "\r")) to--;
        add({ from, to, kind: n.kind, block: n });
      }
      if (!("k" in n) && (n.kind === "bullet" || n.kind === "heading") && n.htags?.length) {
        // lsdoc owns headline tags but stores them as header facets. Their
        // suffix starts where the final header inline ends, in the same AST.
        const end = n.inline.at(-1)?.span?.[1];
        if (end !== undefined) {
          let start = cursor(end - 2 + lead);
          while (start < to && (raw[start] === " " || raw[start] === "\t")) start++;
          while (to > start && (raw[to - 1] === "\n" || raw[to - 1] === "\r")) to--;
          add({ from: start, to, kind: "header-tags", headerTags: n.htags });
        }
      }
    }
    Object.values(value).forEach(visit);
  };
  visit(blocks);
  result.sort((a, b) => a.from - b.from || b.to - a.to);
  cache.set(blocks, result);
  return result;
}

/** One rule for every construct, in previewSpans' source order. Boundaries
 * reveal too, so native typing/deletion can reach every source byte. */
export function concealedSpans(spans: readonly PreviewSpan[], selections: readonly { from: number; to: number }[]): PreviewSpan[] {
  const result: PreviewSpan[] = [];
  let coveredTo = -1;
  for (const span of spans) {
    if (constructRevealed(span, selections)) continue;
    // A rendered parent already renders its descendants. When revealed,
    // descendants remain free to render using this same rule.
    if (span.to <= coveredTo) continue;
    result.push(span);
    coveredTo = span.to;
  }
  return result;
}
