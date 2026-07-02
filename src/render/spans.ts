import { rawOffsetToVisibleOffset } from "../editor/properties";
import type { Span, SpanMap } from "./ast";

const UTF8 = new TextEncoder();
const UTF8_DECODER = new TextDecoder();

export interface SpanDomAttrs {
  "data-so": string;
  "data-se"?: string;
  "data-sm"?: string;
}

type SpanDomData =
  | { kind: "plain"; span: Span; spanMap?: SpanMap }
  | { kind: "coarse"; start: number };

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
  return span ? { "data-so": String(span[0]) } : undefined;
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
  if (end == null) return { kind: "coarse", start };
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

export function editorOffsetFromRenderedRange(
  root: Element,
  range: Pick<Range, "startContainer" | "startOffset">,
  raw: string,
  isHidden: (key: string) => boolean,
): number | null {
  const el = closestSpanElement(root, range.startContainer);
  if (!el) return null;
  const data = spanDataFromElement(el);
  if (!data) return null;

  let sourceByte: number | null;
  if (data.kind === "coarse") {
    sourceByte = data.start;
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
  return rawOffsetToVisibleOffset(raw, rawUtf16, isHidden);
}
