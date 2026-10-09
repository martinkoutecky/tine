// Rendering one lsdoc plain text run: render-time typography, its source-span
// attributes (click-to-caret mapping), and the marks a margin comment's quote
// puts on it (vision §3.7).

import type { JSX } from "solid-js";
import type { Inline, Span } from "./ast";
import { EmojiText } from "./emoji";
import { typographic } from "./typography";
import { typographyMode } from "../ui";
import { plainSpanAttrs, typographicPlainSpanAttrs, utf8ByteLength, utf8ByteToUtf16Offset } from "./spans";

/** A plain run as text with its source span attributes. */
export function renderPlain(sourceText: string, span: Span | undefined, spanMap: Extract<Inline, { k: "plain" }>["span_map"], spanMode: boolean): JSX.Element {
  // Render-time typographic replacement (`->`→`→`, `--`→`–`, …) is a Tine
  // opinion applied ONLY to plain text — code/links/math/tags are other node
  // kinds, so they're excluded for free. Source keeps the ASCII.
  const text = typographyMode() === "render" ? typographic(sourceText) : sourceText;
  const attrs = spanMode
    ? text === sourceText
      ? plainSpanAttrs(span, spanMap)
      : typographicPlainSpanAttrs(sourceText, span, spanMap)
    : undefined;
  return attrs ? <span {...attrs}><EmojiText text={text} /></span> : <EmojiText text={text} />;
}

// A passage a margin comment quotes (vision §3.7) is marked only when it lies
// inside ONE plain text run whose text is byte-identical to its source (no span
// map): anything spanning markup or escapes keeps its ordinary rendering, and the
// comment card still shows the quote. Each piece keeps its own source span, so
// click-to-caret mapping is unchanged.
export function renderQuotedPlain(s: Extract<Inline, { k: "plain" }>, ranges: readonly (readonly [number, number])[]): JSX.Element | null {
  if (ranges.length === 0 || !s.span || (s.span_map && s.span_map.length > 0)) return null;
  const [from, to] = s.span;
  if (utf8ByteLength(s.text) !== to - from) return null;
  const hits = ranges.filter(([start, end]) => start >= from && end <= to && end > start)
    .slice().sort((x, y) => x[0] - y[0]);
  if (hits.length === 0) return null;
  const pieces: JSX.Element[] = [];
  let byte = from;
  let at = 0;
  for (const [start, end] of hits) {
    if (start < byte) continue; // overlaps the previous mark: keep the first
    const a = utf8ByteToUtf16Offset(s.text, start - from);
    const b = utf8ByteToUtf16Offset(s.text, end - from);
    if (a > at) pieces.push(renderPlain(s.text.slice(at, a), [byte, start], undefined, true));
    pieces.push(<span class="comment-quote-anchor">{renderPlain(s.text.slice(a, b), [start, end], undefined, true)}</span>);
    byte = end;
    at = b;
  }
  if (at < s.text.length) pieces.push(renderPlain(s.text.slice(at), [byte, to], undefined, true));
  return <>{pieces}</>;
}
