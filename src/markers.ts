// Single source of truth for the task-marker set, its subsets, AND the one
// leading-marker recognizer. Before this, the list was hand-copied into ~6
// frontend spots (render chip, marker cycler, priority anchor, query builder,
// "carry unfinished tasks", the mock) and they had drifted — e.g. the carry
// open-task set + the query-builder set were both missing IN-PROGRESS and WAIT,
// so carry silently skipped those tasks and the builder couldn't filter them.
// The recognizers later drifted the same way (DUP-7, 2026-08-25 duplication
// audit): MARKER_RE accepted a TAB after the marker and store.ts pre-trimmed
// with Unicode `trimStart`, while editor/marker.ts and render/block.ts used a
// narrower literal-space/no-trim rule — so carry-forward and the priority
// writers disagreed with the lsdoc-rendered checkbox. Everything now imports
// from here.
//
// Backend mirror: `crates/tine-core/src/doc.rs` `MARKERS` (cross-language, so it
// can't share this literal — `markers.test.ts` guards the two sets against drift).
//
// Order is prefix-safe for substring pre-filtering only (WAITING before WAIT).
import { parserReady } from "./render/parse";
import { header_tokens_json, __tineReinstantiate } from "./render/wasm/lsdoc_wasm.js";
import { utf8ToUtf16Cursor } from "./render/utf16Cursor";
import { reportUiFailure } from "./uiFailure";

export const MARKERS = [
  "TODO",
  "DOING",
  "NOW",
  "LATER",
  "WAITING",
  "WAIT",
  "STARTED",
  "IN-PROGRESS",
  "DONE",
  "CANCELED",
  "CANCELLED",
] as const;

/** "Closed" markers — a task in one of these is finished/dropped (not an open task,
 *  not carried forward). `done` chip styling keys off this too. */
export const DONE_MARKERS: ReadonlySet<string> = new Set(["DONE", "CANCELED", "CANCELLED"]);

/** "Open" (unfinished) task markers = all markers minus the closed ones. Used by
 *  "carry unfinished tasks" and any open-task scan. */
export const OPEN_MARKERS: ReadonlySet<string> = new Set(
  MARKERS.filter((m) => !DONE_MARKERS.has(m))
);

/** Where a recognized leading marker sits in the block raw. `start`/`end` are
 *  UTF-16 offsets of the marker text itself, so editors can splice exactly the
 *  marker without touching anything else. */
export interface LeadingMarkerMatch {
  marker: string;
  start: number;
  end: number;
}

/** A header token the parser accepted: its text and UTF-16 span in the block raw. */
export interface HeaderToken {
  text: string;
  start: number;
  end: number;
}

let memo: { raw: string; format: "md" | "org"; tokens: { marker: HeaderToken | null; priority: HeaderToken | null } } | undefined;

/**
 * The ONE accepted marker / priority answer for readers and writers (I-12): lsdoc's header facts
 * (`header_tokens_json`: the regions door's header without the regions walk, marker and priority
 * spans located inside the accepted header only), converted to UTF-16. Whatever whitespace the parser skips before the marker (U+0085 yes, U+FEFF
 * no), the literal-space rule after it, and `[#X]` placement come from the parser, not from here.
 * `MARKERS` below stays the vocabulary; nothing in the editor re-derives where a marker sits.
 * The substring test is only a cheap pre-filter that cannot reject any parser-accepted marker, so
 * ordinary blocks never reach the parser. Before the parser is ready there is no answer (null).
 * One-entry memo: a typing step reads the previous raw (hit) and the new one (one light parse).
 */
export function headerTokens(raw: string, format: "md" | "org" = "md"): { marker: HeaderToken | null; priority: HeaderToken | null } {
  const none = { marker: null, priority: null };
  if (!parserReady() || (!MARKERS.some((m) => raw.includes(m)) && !raw.includes("[#"))) return none;
  if (memo?.raw !== raw || memo.format !== format) {
    let json: string | undefined;
    for (let attempt = 0; attempt < 2 && json === undefined; attempt++) {
      try { json = header_tokens_json(raw, format === "org"); } catch (error) {
        __tineReinstantiate();
        // The retry recovers a poisoned instance; only a second failure is shown.
        if (attempt === 1) reportUiFailure("marker-read", error);
      }
    }
    if (json === undefined) return none;
    const header = JSON.parse(json) as { marker: string | null; priority: string | null; marker_range: [number, number] | null; priority_range: [number, number] | null };
    const toUtf16 = utf8ToUtf16Cursor(raw);
    const token = (text: string | null, range: [number, number] | null): HeaderToken | null =>
      text !== null && range ? { text, start: toUtf16(range[0]), end: toUtf16(range[1]) } : null;
    memo = { raw, format, tokens: { marker: token(header.marker, header.marker_range), priority: token(header.priority, header.priority_range) } };
  }
  return memo.tokens;
}

/** Defines `marker`/`priority` on `block` as accessors that read {@link headerTokens} on first access
 *  once the parser is ready (a read before that is undefined and not remembered). For the demo mock,
 *  which builds its blocks at import, before the wasm parser exists. */
export function lazyHeaderFacets<T extends { raw: string }>(block: T): T {
  let facets: { marker?: string; priority?: string } | undefined;
  const read = () => {
    if (!facets && parserReady()) {
      const { marker, priority } = headerTokens(block.raw);
      facets = { marker: marker?.text, priority: priority?.text };
    }
    return facets ?? {};
  };
  return Object.defineProperties(block, {
    marker: { get: () => read().marker, enumerable: true, configurable: true },
    priority: { get: () => read().priority, enumerable: true, configurable: true },
  });
}

/** The leading task marker the parser accepted, with its UTF-16 span, or null. */
export function matchLeadingMarker(raw: string, format: "md" | "org" = "md"): LeadingMarkerMatch | null {
  const marker = headerTokens(raw, format).marker;
  return marker ? { marker: marker.text, start: marker.start, end: marker.end } : null;
}

/** The recognized leading marker's name, or null. */
export function leadingMarker(raw: string, format: "md" | "org" = "md"): string | null {
  return matchLeadingMarker(raw, format)?.marker ?? null;
}

/** Whether a block with this leading marker renders a task checkbox, and if so
 *  its state — matching OG's `block-checkbox`: `DONE` → checked, any OPEN task
 *  marker → unchecked, everything else (CANCELED/CANCELLED/none) → no checkbox.
 *  Returns `true` (checked) / `false` (unchecked) / `null` (no checkbox). */
export function taskCheckboxState(marker: string | null | undefined): boolean | null {
  if (!marker) return null;
  if (marker === "DONE") return true;
  if (OPEN_MARKERS.has(marker)) return false;
  return null;
}
