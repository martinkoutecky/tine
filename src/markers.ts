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
// Order is prefix-safe for the recognizer's alternation: the longer of any
// prefix pair comes first (WAITING before WAIT).
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

/** One leading-whitespace character before the marker. Mirrors the trim the
 *  one-block lsdoc boundary applies (`crates/lsdoc-block-parse.rs::prepare` =
 *  Rust `trim_start`: Unicode whitespace — newlines and NBSP included), plus
 *  lsdoc's own parser spaces SUB (0x1A) and FF that it additionally skips when
 *  looking for the marker. Checking `end === raw.length` on the skipped prefix
 *  is what makes a bare `TODO` followed by more lines NOT a marker. */
function isMarkerLeadWhitespace(ch: string): boolean {
  return /\s/.test(ch) || ch === "\x1a";
}

/**
 * The ONE leading task-marker recognizer (DUP-7). Byte-equivalent to what the
 * lsdoc boundary projects for a block raw (verified against the vendored lsdoc
 * wasm v0.5.5): skip leading whitespace, then the marker must be followed by a
 * literal ASCII space — or be the entire rest of the input (lsdoc `marker_eof`).
 *
 * Consequences, all intentional and probe-verified:
 *  - `TODO\tx` is NOT a task (lsdoc requires a literal space after the marker);
 *  - `  TODO x`, `\tTODO x`, `\nTODO x` ARE tasks (the boundary trim skips them);
 *  - a bare `TODO` is a task ONLY with nothing after it — `TODO\nbody` (and
 *    even `TODO\n`) is not, but `TODO ` / `TODO \nbody` (space, empty title) is;
 *  - the match is case-sensitive and prefix-safe (`TODOLIST`, `TODO:`, `WAIT`
 *    vs `WAITING` are never misread).
 */
export function matchLeadingMarker(raw: string): LeadingMarkerMatch | null {
  let start = 0;
  while (start < raw.length && isMarkerLeadWhitespace(raw[start])) start++;
  for (const marker of MARKERS) {
    if (!raw.startsWith(marker, start)) continue;
    const end = start + marker.length;
    if (end === raw.length || raw[end] === " ") return { marker, start, end };
  }
  return null;
}

/** The recognized leading marker's name, or null. */
export function leadingMarker(raw: string): string | null {
  return matchLeadingMarker(raw)?.marker ?? null;
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
