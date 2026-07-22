import { For, type JSX } from "solid-js";
import { EmojiText } from "../render/emoji";
import type { MatchSpan } from "../types";

export type SearchMatchSpan = MatchSpan;

interface Segment {
  text: string;
  marked: boolean;
}

interface Window {
  start: number;
  end: number;
}

function graphemeBoundaries(text: string): number[] {
  const Segmenter = (Intl as unknown as {
    Segmenter?: new (locale?: string, options?: { granularity: "grapheme" }) => {
      segment(value: string): Iterable<{ index: number }>;
    };
  }).Segmenter;
  if (Segmenter) {
    const boundaries = [...new Segmenter(undefined, { granularity: "grapheme" }).segment(text)]
      .map((part) => part.index);
    boundaries.push(text.length);
    return boundaries;
  }
  const boundaries = [0];
  let offset = 0;
  for (const scalar of text) {
    offset += scalar.length;
    boundaries.push(offset);
  }
  return boundaries;
}

function snapWindow(text: string, window: Window): Window {
  const boundaries = graphemeBoundaries(text);
  let start = 0;
  let end = text.length;
  for (const boundary of boundaries) {
    if (boundary <= window.start) start = boundary;
    if (boundary >= window.end) {
      end = boundary;
      break;
    }
  }
  return { start, end };
}

const MAX_SPANS = 24;
const MAX_WINDOWS = 3;
const MAX_WINDOW_CHARS = 92;
const MAX_TOTAL_CHARS = 210;

function normalizedSpans(text: string, spans: SearchMatchSpan[]): SearchMatchSpan[] {
  const sorted = spans
    .map((span) => ({
      start: Math.max(0, Math.min(text.length, span.start | 0)),
      end: Math.max(0, Math.min(text.length, span.end | 0)),
    }))
    .filter((span) => span.end > span.start)
    .sort((a, b) => a.start - b.start || a.end - b.end)
    .slice(0, MAX_SPANS);
  const merged: SearchMatchSpan[] = [];
  for (const span of sorted) {
    const last = merged[merged.length - 1];
    if (last && span.start <= last.end) last.end = Math.max(last.end, span.end);
    else merged.push({ ...span });
  }
  return merged;
}

function excerptWindows(text: string, spans: SearchMatchSpan[]): Window[] {
  if (!text.length) return [];
  if (!spans.length) return [{ start: 0, end: Math.min(text.length, MAX_TOTAL_CHARS) }];
  const windows: Window[] = [];
  for (const span of spans) {
    let start = Math.max(0, span.start - 28);
    let end = Math.min(text.length, Math.max(span.end + 44, start + 48));
    if (end - start > MAX_WINDOW_CHARS) {
      end = Math.min(text.length, start + MAX_WINDOW_CHARS);
      if (end < span.end) {
        end = span.end;
        start = Math.max(0, end - MAX_WINDOW_CHARS);
      }
    }
    const last = windows[windows.length - 1];
    if (last && start <= last.end + 12) last.end = Math.min(text.length, Math.max(last.end, end));
    else windows.push({ start, end });
    if (windows.length >= MAX_WINDOWS) break;
  }

  let remaining = MAX_TOTAL_CHARS;
  return windows.map((window) => {
    const end = Math.min(window.end, window.start + remaining);
    remaining = Math.max(0, remaining - (end - window.start));
    return snapWindow(text, { start: window.start, end });
  }).filter((window) => window.end > window.start);
}

/** Pure bounded excerpt model, exported for regression and performance tests. */
export function buildSearchExcerpt(text: string, inputSpans: SearchMatchSpan[]): Segment[] {
  const spans = normalizedSpans(text, inputSpans);
  const windows = excerptWindows(text, spans);
  const segments: Segment[] = [];

  windows.forEach((window, index) => {
    if (index > 0 || window.start > 0) segments.push({ text: "…", marked: false });
    let cursor = window.start;
    for (const span of spans) {
      if (span.end <= window.start || span.start >= window.end) continue;
      const start = Math.max(window.start, span.start);
      const end = Math.min(window.end, span.end);
      if (start > cursor) segments.push({ text: text.slice(cursor, start), marked: false });
      if (end > start) segments.push({ text: text.slice(start, end), marked: true });
      cursor = Math.max(cursor, end);
    }
    if (cursor < window.end) segments.push({ text: text.slice(cursor, window.end), marked: false });
    if (window.end < text.length && index === windows.length - 1) {
      segments.push({ text: "…", marked: false });
    }
  });
  return segments;
}

export function SearchResultRow(props: {
  page: string;
  breadcrumb: string[];
  text: string;
  spans: SearchMatchSpan[];
}): JSX.Element {
  const context = () => [props.page, ...props.breadcrumb].filter(Boolean).join(" › ");
  const segments = () => buildSearchExcerpt(props.text, props.spans);
  return (
    <>
      <span class="switcher-kind">block</span>
      <span class="search-result-body" aria-label={`${context()}: ${props.text}`}>
        <span class="search-result-context"><EmojiText text={context()} /></span>
        <span class="search-result-excerpt">
          <For each={segments()}>{(segment) => segment.marked
            ? <mark><EmojiText text={segment.text} /></mark>
            : <EmojiText text={segment.text} />}</For>
        </span>
      </span>
    </>
  );
}
