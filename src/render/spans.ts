import { rawOffsetToVisibleOffset, type PropFormat } from "../editor/properties";
import { typographicSegments } from "./typography";
import type { Span, SpanMap } from "./ast";

const UTF8 = new TextEncoder();
const UTF8_DECODER = new TextDecoder();

export interface SpanDomAttrs {
  "data-so": string;
  "data-se"?: string;
  "data-sm"?: string;
  "data-sce"?: string;
}

type SpanDomData =
  | { kind: "plain"; span: Span; spanMap?: SpanMap }
  | { kind: "coarse"; start: number; end: number | null };

function clamp(n: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, n));
}

export function utf8ByteLength(text: string): number {
  return UTF8.encode(text).length;
}

export function utf16ToUtf8ByteOffset(text: string, utf16Offset: number): number {
  return UTF8.encode(text.slice(0, clamp(utf16Offset, 0, text.length))).length;
}

export function utf8ByteToUtf16Offset(text: string, byteOffset: number): number {
  const bytes = UTF8.encode(text);
  return UTF8_DECODER.decode(bytes.subarray(0, clamp(byteOffset, 0, bytes.length))).length;
}

export function rebulletedSourceByteToRawByte(raw: string, sourceByte: number): number {
  const rawLen = utf8ByteLength(raw);
  const lead = rawLen - utf8ByteLength(raw.trimStart());
  return clamp(sourceByte - 2 + lead, 0, rawLen);
}

export function sourceByteFromPlainTextByte(
  span: Span,
  spanMap: SpanMap | undefined,
  textByteOffset: number,
  textByteLength: number,
): number | null {
  if (textByteOffset < 0 || textByteOffset > textByteLength) return null;
  if (!spanMap || spanMap.length === 0) {
    const source = span[0] + textByteOffset;
    return source <= span[1] ? source : null;
  }

  let lastSourceEnd: number | null = null;
  for (const [textOff, sourceOff, len] of spanMap) {
    if (textByteOffset < textOff) return sourceOff; // uncovered rendered gap: snap forward.
    if (len > 0 && textByteOffset >= textOff && textByteOffset < textOff + len) {
      return sourceOff + (textByteOffset - textOff);
    }
    if (textByteOffset === textOff + len && textByteOffset === textByteLength) {
      return sourceOff + len;
    }
    lastSourceEnd = sourceOff + len;
  }
  return lastSourceEnd != null && textByteOffset <= textByteLength ? lastSourceEnd : null;
}

export function encodeSpanMap(spanMap: SpanMap): string {
  return spanMap.map(([textOff, sourceOff, len]) => `${textOff}:${sourceOff}:${len}`).join(";");
}

function decodeSpanMap(value: string | null): SpanMap | undefined | null {
  if (!value) return undefined;
  const out: SpanMap = [];
  for (const part of value.split(";")) {
    const nums = part.split(":").map((n) => Number(n));
    if (nums.length !== 3 || nums.some((n) => !Number.isInteger(n) || n < 0)) return null;
    out.push(nums as [number, number, number]);
  }
  return out;
}

export function plainSpanAttrs(span: Span | undefined, spanMap?: SpanMap): SpanDomAttrs | undefined {
  if (!span) return undefined;
  return {
    "data-so": String(span[0]),
    "data-se": String(span[1]),
    ...(spanMap && spanMap.length > 0 ? { "data-sm": encodeSpanMap(spanMap) } : {}),
  };
}

export function coarseSpanAttrs(span: Span | undefined): SpanDomAttrs | undefined {
  // `data-sce` is the coarse end byte (one-past). Unlike `data-se` it does NOT
  // promise byte-exact text mapping — a coarse element's rendered text usually
  // differs from its source bytes (`**bold**`→"bold", aliased links, entities).
  // It only lets a click resolve to whichever EDGE of the construct is nearer,
  // so clicking to the right of a trailing link lands the caret after it (not
  // before it — GH #34) rather than snapping to the span start.
  return span ? { "data-so": String(span[0]), "data-sce": String(span[1]) } : undefined;
}

/** Exact click mapping for inline code/verbatim nodes. lsdoc's span includes the
 *  symmetric Markdown backtick run or Org `=`/`~` delimiters, while `text` is
 *  the rendered literal body. When the remaining bytes split evenly between
 *  the two delimiters, expose only the body span as byte-exact plain text. Fall
 *  back to edge-only mapping for a parser shape that cannot prove symmetry. */
export function literalSpanAttrs(text: string, span: Span | undefined): SpanDomAttrs | undefined {
  if (!span) return undefined;
  const extra = span[1] - span[0] - utf8ByteLength(text);
  if (extra < 2 || extra % 2 !== 0) return coarseSpanAttrs(span);
  const delimiter = extra / 2;
  return plainSpanAttrs([span[0] + delimiter, span[1] - delimiter]);
}

/** Span attrs for a plain whose RENDERED text differs from the AST text by the
 *  typographic substitution (`->`→`→`, `--`→`–`, …). The emitted `data-sm` maps
 *  RENDERED text bytes → block-source bytes: the unchanged runs between glyph
 *  replacements map byte-equal, composed through the plain's own `span_map` when
 *  present; the glyph bytes stay uncovered, so a click on a glyph snaps to the
 *  next mapped run (same rule as CRLF joiners). Falls back to coarse attrs when
 *  nothing maps (e.g. the whole plain is a single trigger). */
export function typographicPlainSpanAttrs(
  sourceText: string,
  span: Span | undefined,
  spanMap: SpanMap | undefined,
): SpanDomAttrs | undefined {
  if (!span) return undefined;
  const typo = typographicSegments(sourceText);
  if (typo.length === 0) return plainSpanAttrs(span, spanMap);

  // Outer map: rendered-byte → AST-text-byte, over the unchanged runs.
  const outer: SpanMap = [];
  let renderedByte = 0;
  let textUtf16 = 0;
  let textByte = 0;
  const advance = (to: number) => {
    const runBytes = utf8ByteLength(sourceText.slice(textUtf16, to));
    if (runBytes > 0) outer.push([renderedByte, textByte, runBytes]);
    renderedByte += runBytes;
    textByte += runBytes;
    textUtf16 = to;
  };
  for (const seg of typo) {
    advance(seg.start);
    // The trigger's bytes are consumed from the text side, the glyph's from the
    // rendered side; neither is covered by a mapping segment.
    renderedByte += utf8ByteLength(seg.glyph);
    textByte += utf8ByteLength(sourceText.slice(seg.start, seg.end));
    textUtf16 = seg.end;
  }
  advance(sourceText.length);

  // Compose with the plain's own text-byte → block-source-byte mapping.
  const composed: SpanMap = [];
  const inner: SpanMap = spanMap && spanMap.length > 0
    ? spanMap
    : [[0, span[0], utf8ByteLength(sourceText)]];
  for (const [rOff, tOff, len] of outer) {
    for (const [tiOff, siOff, ilen] of inner) {
      const a = Math.max(tOff, tiOff);
      const b = Math.min(tOff + len, tiOff + ilen);
      if (b > a) composed.push([rOff + (a - tOff), siOff + (a - tiOff), b - a]);
    }
  }
  if (composed.length === 0) return coarseSpanAttrs(span);
  return {
    "data-so": String(span[0]),
    "data-se": String(span[1]),
    "data-sm": encodeSpanMap(composed),
  };
}

function byteAttr(el: Element, name: string): number | null {
  const value = el.getAttribute(name);
  if (value == null || value === "") return null;
  const n = Number(value);
  return Number.isInteger(n) && n >= 0 ? n : null;
}

function spanDataFromElement(el: Element): SpanDomData | null {
  const start = byteAttr(el, "data-so");
  if (start == null) return null;
  const end = byteAttr(el, "data-se");
  if (end == null) return { kind: "coarse", start, end: byteAttr(el, "data-sce") };
  if (end < start) return null;
  const spanMap = decodeSpanMap(el.getAttribute("data-sm"));
  if (spanMap === null) return null;
  return { kind: "plain", span: [start, end], spanMap };
}

function elementFromNode(node: Node): Element | null {
  return node.nodeType === Node.ELEMENT_NODE ? node as Element : node.parentElement;
}

function closestSpanElement(root: Element, node: Node): Element | null {
  let el = elementFromNode(node);
  while (el && root.contains(el)) {
    if (el.hasAttribute("data-so")) return el;
    if (el === root) break;
    el = el.parentElement;
  }
  return null;
}

/** DOM-text walk used for click mapping. It treats <br> as "\n" and Twemoji
 *  <img alt="…"> as its alt text, matching the logical rendered plain string. */
export function renderedTextCaret(
  root: Node,
  container: Node,
  offset: number,
): { text: string; caret: number | null } {
  let text = "";
  let caret: number | null = null;
  const walk = (n: Node) => {
    if (n.nodeType === Node.TEXT_NODE) {
      const t = n.textContent ?? "";
      if (n === container) caret = text.length + clamp(offset, 0, t.length);
      text += t;
      return;
    }
    if (n.nodeType !== Node.ELEMENT_NODE) return;
    const el = n as Element;
    if (el.tagName === "BR") {
      if (n === container) caret = text.length;
      text += "\n";
      return;
    }
    if (el.tagName === "IMG") {
      const alt = el.getAttribute("alt") ?? "";
      if (n === container) caret = text.length + (offset <= 0 ? 0 : alt.length);
      text += alt;
      return;
    }
    const children = Array.from(n.childNodes);
    if (n === container && offset <= 0) caret = text.length;
    for (let i = 0; i < children.length; i++) {
      walk(children[i]);
      if (n === container && offset === i + 1) caret = text.length;
    }
    if (n === container && caret == null) caret = text.length;
  };
  walk(root);
  return { text, caret };
}

/** Rectangles of one rendered block's content, one per visual line box.
 *  Narrower than `DOMRectList` so tests can hand in plain objects. */
export interface LineBox {
  readonly top: number;
  readonly bottom: number;
  readonly right: number;
}

/** True when (x, y) lies horizontally past the last glyph of the FINAL visual
 *  line of `boxes` — i.e. in the empty run-out at the end of the block.
 *
 *  This is the whole of GH #465, and it is deliberately geometric rather than
 *  syntactic. A block ending in `*text*` renders an `<em>` whose span map stops
 *  before the closing `*`, so a click past the end maps to a perfectly valid
 *  interior source offset — one byte before the invisible delimiter — and no
 *  "mapping failed" fallback ever fires. Asking where the click landed instead
 *  of what it landed on covers every trailing construct with a hidden closing
 *  delimiter at once, in Markdown and Org alike, rather than accumulating a
 *  special case per syntax.
 *
 *  Only the final line qualifies: a click past the right edge of an earlier
 *  wrapped line belongs at that line's end, which the ordinary span mapping
 *  already gets right. */
export function beyondFinalGlyph(boxes: ArrayLike<LineBox>, x: number, y: number): boolean {
  if (boxes.length === 0) return false;
  // Line boxes arrive in DOM order, which is visual order for text but need not
  // be once floats or inline-blocks are involved, so find the bottom band by
  // coordinate rather than by taking the last entry.
  let bottom = -Infinity;
  for (let i = 0; i < boxes.length; i++) bottom = Math.max(bottom, boxes[i].bottom);
  let top = Infinity;
  let right = -Infinity;
  for (let i = 0; i < boxes.length; i++) {
    const box = boxes[i];
    // Half a pixel of slack: sub-pixel line heights otherwise split one visual
    // line into two bands that each look like "not the last one".
    if (box.bottom < bottom - 0.5) continue;
    top = Math.min(top, box.top);
    right = Math.max(right, box.right);
  }
  return y >= top && y <= bottom && x > right;
}

/** {@link beyondFinalGlyph} against a live element's rendered line boxes.
 *  Measures the CONTENTS, not the element: a block-level container is full
 *  width, so its own border box says nothing about where the text ends. */
export function clickBeyondRenderedEnd(root: Element, x: number, y: number): boolean {
  const doc = root.ownerDocument;
  const range = doc.createRange();
  // An environment with no layout engine has no line boxes to ask about: jsdom
  // does not implement Range.getClientRects at all, and calling it there threw
  // out of the click handler and took the whole edit gesture with it. Declining
  // is the right answer, not an approximation — the caller falls back to the
  // ordinary span mapping, which is what ran before GH #465.
  if (typeof range.getClientRects !== "function") return false;
  const view = doc.defaultView;
  const boxes: LineBox[] = [];
  for (const child of Array.from(root.childNodes)) collectInFlowRects(child, view, range, boxes);
  range.detach?.();
  return beyondFinalGlyph(boxes, x, y);
}

/** Line boxes of the IN-FLOW content of `node`, appended to `out`.
 *
 *  A single range over the whole block also picks up its floats, and a float is
 *  drawn hard against the block's right edge. A float TALLER than the line it
 *  rides then owns the bottom band, and the block reports its text as ending at
 *  the full content width — so no click in it can ever be past the end.
 *  `{{img … right}}` is the live case: measured in the running app, its wrapper
 *  box was right 1080 / bottom 545.2 against text right 665.5 / bottom 542.2,
 *  and the past-the-end caret was dead in that block. The wrapper sits inside
 *  the inline tree rather than directly under `.block-content`, hence the
 *  recursion. (The reference-count badge floats too but is shorter than the
 *  line, so it never defines the bottom band and never had this effect —
 *  measured, not assumed, after a synthetic fixture wrongly said it did.) */
function collectInFlowRects(node: Node, view: Window | null, range: Range, out: LineBox[]): void {
  if (node.nodeType === 1) {
    const el = node as Element;
    // Positive tests, not `float !== "none"`: a stylesheet-less environment
    // reports "" for everything, and treating that as out-of-flow would discard
    // the whole block.
    const style = view?.getComputedStyle(el);
    const f = style?.float;
    const p = style?.position;
    if (f === "left" || f === "right" || p === "absolute" || p === "fixed") return;
    if (el.childElementCount > 0) {
      for (const child of Array.from(el.childNodes)) collectInFlowRects(child, view, range, out);
      return;
    }
  }
  // A leaf: a text node, or an element with no element children — including a
  // replaced one such as an image, which has no contents to measure but does
  // occupy a box on the line. Hence selectNode, not selectNodeContents.
  range.selectNode(node);
  const rects = range.getClientRects();
  for (let i = 0; i < rects.length; i++) out.push(rects[i]);
}

/** The offset inside a whole-block CODE CARD's own text for a click that landed
 *  in it, or null when the click was not inside one.
 *
 *  A code card is highlight.js `innerHTML`, so it carries no lsdoc span data:
 *  {@link editorOffsetFromRenderedRange} declines and every click inside one
 *  used to fall through to "the end of the block". In a three-line fence nobody
 *  noticed. In a thousand-line one the caret lands hundreds of lines from the
 *  click, and when a long line sits at the end the no-wrap code editor scrolls
 *  to that line's far right — so the block shows blank space and is hard to
 *  scroll back (GH #489).
 *
 *  The card's text is exactly what the editor will hold for such a block (the
 *  editor shows the fenced body, not the wrapper), so the offset is simply
 *  counted. The caller owns the "is this block one code card" question and
 *  clamps the result to the body it is about to edit. */
export function codeCardOffsetFromRange(
  root: Element,
  range: Pick<Range, "startContainer" | "startOffset">,
): number | null {
  const code = root.querySelector("pre.code-block > code");
  if (!code) return null;
  const container = range.startContainer;
  if (code !== container && !code.contains(container)) return null;
  const { caret } = renderedTextCaret(code, container, range.startOffset);
  return caret;
}

export function editorOffsetFromRenderedRange(
  root: Element,
  range: Pick<Range, "startContainer" | "startOffset">,
  raw: string,
  isHidden: (key: string) => boolean,
  format: PropFormat = "md",
): number | null {
  const el = closestSpanElement(root, range.startContainer);
  if (!el) return null;
  const data = spanDataFromElement(el);
  if (!data) return null;

  let sourceByte: number | null;
  if (data.kind === "coarse") {
    // Coarse elements map to an EDGE, not an interior byte. Pick the nearer one:
    // a click landing in the second half of the rendered text (e.g. to the right
    // of a trailing `[[link]]`) resolves to the span end, so the caret sits after
    // the construct instead of before it (GH #34).
    sourceByte = data.start;
    if (data.end != null) {
      const { text, caret } = renderedTextCaret(el, range.startContainer, range.startOffset);
      if (caret != null && text.length > 0 && caret * 2 >= text.length) sourceByte = data.end;
    }
  } else {
    const { text, caret } = renderedTextCaret(el, range.startContainer, range.startOffset);
    if (caret == null) return null;
    sourceByte = sourceByteFromPlainTextByte(
      data.span,
      data.spanMap,
      utf16ToUtf8ByteOffset(text, caret),
      utf8ByteLength(text),
    );
  }
  if (sourceByte == null) return null;

  const rawByte = rebulletedSourceByteToRawByte(raw, sourceByte);
  const rawUtf16 = utf8ByteToUtf16Offset(raw, rawByte);
  return rawOffsetToVisibleOffset(raw, rawUtf16, isHidden, format);
}
