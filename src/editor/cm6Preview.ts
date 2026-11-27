import { blockRegions, parseBlock, renderPlanningOf } from "../render/parse";
import { utf8ToUtf16Cursor } from "../render/utf16Cursor";
import type { Block, Inline } from "../render/ast";
import { typographicSegments } from "../render/typography";
import { calloutLead } from "../render/body";

export interface PreviewSpan {
  from: number;
  to: number;
  kind: string;
  inline?: Inline;
  block?: Block;
  headerTags?: string[];
  label?: string;
  /** A `typography` span: the glyph the view draws for this ASCII sequence (`render/typography.ts`). */
  glyph?: string;
  /** A standalone SCHEDULED/DEADLINE line, drawn as the view's date chip. */
  planning?: { ts: "Scheduled" | "Deadline"; value: string };
  /** Where the reveal test starts when it differs from `from`: the planning span also replaces the line break
   * before its line (so the chip sits at the end of the header line, as in the view), which must not reveal the
   * source when the caret is merely at the end of that header line. */
  revealFrom?: number;
}

const cache = new WeakMap<Block[], { spans: PreviewSpan[]; glyphs: PreviewSpan[] }>();

export function constructRevealed(span: { from: number; to: number; revealFrom?: number }, selections: readonly { from: number; to: number }[]): boolean {
  const from = span.revealFrom ?? span.from;
  return selections.some(r => r.from <= span.to && r.to >= from);
}

/** lsdoc is the sole syntax authority. CM6 receives UTF-16 ranges. Containers
 * and descendants share the same reveal policy, including header tokens.
 * Source stays in the document: decorations never rewrite boundary bytes. */
export function previewSpans(raw: string, format: "md" | "org"): PreviewSpan[] {
  return preview(raw, format).spans;
}

/** The `typography` spans of the same walk, apart: sorted and disjoint, so the editor draws only those in view
 *  ({@link glyphDecorations}) and never walks them on a caret move. */
export function previewGlyphs(raw: string, format: "md" | "org"): PreviewSpan[] {
  return preview(raw, format).glyphs;
}

function preview(raw: string, format: "md" | "org"): { spans: PreviewSpan[]; glyphs: PreviewSpan[] } {
  const blocks = parseBlock(raw, format === "org");
  const cached = cache.get(blocks);
  if (cached) return cached;
  const result: PreviewSpan[] = [], glyphs: PreviewSpan[] = [];
  const out = { spans: result, glyphs };
  const cursor = utf8ToUtf16Cursor(raw);
  // parseBlock re-bullets trimmed raw with "- " / "* "; region ranges
  // already use raw coordinates. Same mapping as render/spans.ts.
  const lead = new TextEncoder().encode(raw.slice(0, raw.length - raw.trimStart().length)).length;
  const regions = blockRegions(raw, format);
  if (regions.quarantined) return out;
  const add = (span: PreviewSpan) => {
    if (span.from >= 0 && span.to <= raw.length && span.from < span.to) result.push(span);
  };
  for (const [kind, range, label] of [
    ["task", regions.header.marker_range, regions.header.marker],
    ["priority", regions.header.priority_range, regions.header.priority],
  ] as const) {
    if (range && label) add({ from: cursor(range[0]), to: cursor(range[1]), kind, label });
  }
  const chips = new Map(renderPlanningOf(blocks).map((p) => [p.span_start, p]));
  // `typo`: a plain run here is drawn by the view's plain renderer (`renderPlain`), the only place the render pass
  // substitutes glyphs. Emphasis, sub- and superscript draw their children through it; a link, tag or other inline
  // node draws its own text, so its plain children stay raw (when such a node is concealed, its widget is the view's).
  // A callout's title is drawn as raw text by the view (render/body.tsx), so its lead run stays raw here too.
  const titles = new Set<unknown>();
  const visit = (value: unknown, typo = true): void => {
    if (Array.isArray(value)) { value.forEach(v => visit(v, typo)); return; }
    if (!value || typeof value !== "object") return;
    const n = value as Inline | Block;
    if ("kind" in n && n.kind === "quote") { const title = calloutLead(n)?.lead; if (title) titles.add(title); }
    if ("k" in n && n.k !== "plain" && n.k !== "emphasis" && n.k !== "subscript" && n.k !== "superscript") typo = false;
    if (n.span) {
      const from = cursor(n.span[0] - 2 + lead);
      let to = cursor(n.span[1] - 2 + lead);
      const chip = "k" in n && n.k === "timestamp" ? chips.get(n.span[0]) : undefined;
      if (chip?.value) {
        // The view draws a standalone planning line as a date chip after the first line, not as body text.
        // Take the line break (and indentation) before the line too, so edit mode keeps the chip there; a
        // line with trailing body text keeps its own line.
        let start = from, lineStart = from, end = to;
        if (!chip.has_trailing_body) {
          let k = from;
          while (k > 0 && (raw[k - 1] === " " || raw[k - 1] === "\t")) k--;
          if (k > 0 && raw[k - 1] === "\n") { start = raw[k - 2] === "\r" ? k - 2 : k - 1; lineStart = k; }
          while (end < raw.length && (raw[end] === " " || raw[end] === "\t")) end++;
        }
        to = end;
        add({ from: start, to, kind: "planning", planning: { ts: chip.kind, value: chip.value }, ...(start < from ? { revealFrom: lineStart } : {}) });
      } else if ("k" in n) {
        // Plain runs can own escapes. Only byte-identical plain text and
        // ordinary line breaks have nothing to conceal, except the glyphs the view substitutes in such a run
        // (an escaped run is concealed whole, and its widget is the view's plain renderer).
        const verbatimPlain = n.k === "plain" && raw.slice(from, to) === n.text;
        if (verbatimPlain && typo && !titles.has(n)) {
          for (const seg of typographicSegments(n.text)) glyphs.push({ from: from + seg.start, to: from + seg.end, kind: "typography", glyph: seg.glyph });
        }
        if (n.k !== "break" && !verbatimPlain) {
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
    Object.values(value).forEach(v => visit(v, typo));
  };
  visit(blocks);
  result.sort((a, b) => a.from - b.from || b.to - a.to);
  glyphs.sort((a, b) => a.from - b.from);
  cache.set(blocks, out);
  return out;
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

/** What the editor's glyph pass did, for the counting tests: glyph spans compared against a range, and glyph
 *  decorations made. */
export const glyphWork = { examined: 0, built: 0 };

/** The first glyph ending at or after `pos` (glyphs are sorted and disjoint, so their ends are sorted too). */
function firstGlyphFrom(glyphs: readonly PreviewSpan[], pos: number): number {
  let lo = 0, hi = glyphs.length;
  while (lo < hi) { const mid = (lo + hi) >> 1; if (glyphs[mid].to < pos) lo = mid + 1; else hi = mid; }
  return lo;
}

type Ranges = readonly { from: number; to: number }[];

/** Whether moving the selection from `before` to `after` reveals or hides a glyph in `visible`. A glyph can flip only
 *  where exactly one of the two selections lies, so only the glyphs in that symmetric difference, inside the visible
 *  ranges, are looked at: shrinking a long selection by a character looks at the glyphs at that character. */
export function glyphRevealChanges(glyphs: readonly PreviewSpan[], before: Ranges, after: Ranges, visible: Ranges): boolean {
  const covers = (ranges: Ranges, x: number) => ranges.some(r => r.from <= x && x <= r.to);
  const flips = (x: number) => covers(before, x) !== covers(after, x);
  const points = [...new Set([...before, ...after].flatMap(r => [r.from, r.to]))].sort((x, y) => x - y);
  const diff: { from: number; to: number }[] = [];
  points.forEach((x, i) => {
    if (flips(x)) diff.push({ from: x, to: x });
    const y = points[i + 1];
    if (y !== undefined && flips((x + y) / 2)) diff.push({ from: x, to: y });
  });
  for (const d of diff) {
    for (const v of visible) {
      const from = Math.max(d.from, v.from), to = Math.min(d.to, v.to);
      for (let i = firstGlyphFrom(glyphs, from); from <= to && i < glyphs.length && glyphs[i].from <= to; i++) {
        glyphWork.examined++;
        if (constructRevealed(glyphs[i], before) !== constructRevealed(glyphs[i], after)) return true;
      }
    }
  }
  return false;
}

/** The glyphs to draw in `visible`: those no selection touches and no concealed construct covers (a concealed
 *  construct's widget is the view's renderer, which draws its own glyphs). `concealed` is concealedSpans' output:
 *  sorted and disjoint. Work is bounded by the glyphs in view. */
export function glyphDecorations<T>(glyphs: readonly PreviewSpan[], visible: readonly { from: number; to: number }[], concealed: readonly PreviewSpan[],
  selections: readonly { from: number; to: number }[], make: (glyph: PreviewSpan) => T): T[] {
  const out: T[] = [];
  let next = 0; // a glyph is drawn once even where two visible ranges meet
  for (const range of visible) {
    let lo = 0, hi = concealed.length;
    while (lo < hi) { const mid = (lo + hi) >> 1; if (concealed[mid].to < range.from) lo = mid + 1; else hi = mid; }
    let c = lo;
    for (let i = Math.max(next, firstGlyphFrom(glyphs, range.from)); i < glyphs.length && glyphs[i].from <= range.to; i++) {
      const glyph = glyphs[i];
      next = i + 1;
      glyphWork.examined++;
      while (c < concealed.length && concealed[c].to < glyph.from) c++;
      if (c < concealed.length && concealed[c].from <= glyph.from && glyph.to <= concealed[c].to) continue;
      if (constructRevealed(glyph, selections)) continue;
      glyphWork.built++;
      out.push(make(glyph));
    }
  }
  return out;
}
