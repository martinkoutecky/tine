import { blockRegions, parseBlock } from "../render/parse";
import { utf8ToUtf16Cursor } from "../render/utf16Cursor";
import type { Block, Inline } from "../render/ast";
import { planningChipOf } from "../render/facets";

export interface PreviewSpan {
  from: number;
  to: number;
  kind: string;
  inline?: Inline;
  block?: Block;
  headerTags?: string[];
  label?: string;
  /** A standalone SCHEDULED/DEADLINE line, drawn as the view's date chip. */
  planning?: { ts: "Scheduled" | "Deadline"; value: string };
  /** Where the reveal test starts when it differs from `from`: the planning span also replaces the line break
   * before its line (so the chip sits at the end of the header line, as in the view), which must not reveal the
   * source when the caret is merely at the end of that header line. */
  revealFrom?: number;
}

const cache = new WeakMap<Block[], PreviewSpan[]>();

export function constructRevealed(span: { from: number; to: number; revealFrom?: number }, selections: readonly { from: number; to: number }[]): boolean {
  const from = span.revealFrom ?? span.from;
  return selections.some(r => r.from <= span.to && r.to >= from);
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
      const chip = "k" in n && n.k === "timestamp" ? planningChipOf(n, raw) : null;
      if (chip?.value) {
        // The view draws a standalone planning line as a date chip after the first line, not as body text.
        // Take the line break (and indentation) before the line too, so edit mode keeps the chip there; a
        // line with trailing body text keeps its own line.
        let start = from, lineStart = from, end = to;
        if (!chip.hasTrailingBody) {
          let k = from;
          while (k > 0 && (raw[k - 1] === " " || raw[k - 1] === "\t")) k--;
          if (k > 0 && raw[k - 1] === "\n") { start = raw[k - 2] === "\r" ? k - 2 : k - 1; lineStart = k; }
          while (end < raw.length && (raw[end] === " " || raw[end] === "\t")) end++;
        }
        to = end;
        add({ from: start, to, kind: "planning", planning: { ts: chip.ts, value: chip.value }, ...(start < from ? { revealFrom: lineStart } : {}) });
      } else if ("k" in n) {
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
