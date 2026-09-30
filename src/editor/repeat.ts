// Repeating tasks. A SCHEDULED/DEADLINE timestamp may carry a repeater, e.g.
// `<2026-06-16 Tue +1w>` (cumulative), `.+1w` (from completion), `++1w`. When a
// repeating task is cycled to DONE, OG instead advances the date(s) to the next
// occurrence and resets the marker to the workflow's open state. Pure + tested.

import { leadingMarker, nextMarker, cycleMarker, setMarker, type Workflow } from "./marker";
import { matchLeadingMarker, taskCheckboxState } from "../markers";
import { applyMarkerTransition } from "../logbook";
import { blockRegions } from "../render/parse";
import { utf8ByteToUtf16Offset } from "../render/spans";
import type { Format } from "../types";

import { appNow } from "../journal";
const REPEATER = /([.+]{1,2})(\d+)([dwmy])/;
const TS_RE = /<(\d{4})-(\d{2})-(\d{2})(?:\s+[A-Za-z]{3})?(?:\s+([.+]{1,2})(\d+)([dwmy]))?>/;
const WD = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

interface MarkerTimeOptions {
  format: Format;
  enabled: boolean;
  withSeconds: boolean;
}

/** True if the block has a repeater on a SCHEDULED/DEADLINE line. A line of a
 *  code/src block is content, never the task's planning (C3 L14; the one answer
 *  is editor/literalLines.ts — its Markdown parse also recognizes `#+BEGIN_SRC`). */
export function hasRepeater(raw: string, format: Format): boolean {
  return blockRegions(raw, format).planning.some(p => p.kind !== "Closed" && REPEATER.test(
    raw.slice(utf8ByteToUtf16Offset(raw,p.timestamp[0]),utf8ByteToUtf16Offset(raw,p.timestamp[1]))));
}

/** Advance one `<…>` timestamp by its repeater; null if it has none. */
function advanceTimestamp(ts: string): string | null {
  const m = TS_RE.exec(ts);
  if (!m || !m[4]) return null;
  const [, y, mo, d, kind, n, unit] = m;
  const num = Number(n);
  if (!num) return null; // a +0 repeater is degenerate — don't loop/advance
  const step = (dt: Date) => {
    if (unit === "d") dt.setDate(dt.getDate() + num);
    else if (unit === "w") dt.setDate(dt.getDate() + num * 7);
    else if (unit === "m") dt.setMonth(dt.getMonth() + num);
    else if (unit === "y") dt.setFullYear(dt.getFullYear() + num);
  };
  // `.+` repeats from the completion date (today); `+`/`++` from the stored date.
  // `++` is catch-up: advance repeatedly until strictly past today (skipping any
  // missed occurrences); `+`/`.+` advance once. The kind is preserved verbatim.
  let dt: Date;
  if (kind === ".+") {
    dt = appNow();
    step(dt);
  } else {
    dt = new Date(Number(y), Number(mo) - 1, Number(d));
    if (kind === "++") {
      const today = appNow();
      today.setHours(0, 0, 0, 0);
      let guard = 0;
      do {
        step(dt);
      } while (dt <= today && ++guard < 100000);
    } else {
      step(dt);
    }
  }
  const yyyy = dt.getFullYear();
  const MM = String(dt.getMonth() + 1).padStart(2, "0");
  const dd = String(dt.getDate()).padStart(2, "0");
  return `<${yyyy}-${MM}-${dd} ${WD[dt.getDay()]} ${kind}${num}${unit}>`;
}

/** Roll a repeating task forward: advance its dates and reset the marker to the
 *  workflow's open state. Returns the new raw, or null if not repeating. */
export function rollRepeat(raw: string, workflow: Workflow, format: Format): string | null {
  if (!hasRepeater(raw, format)) return null;
  const open = workflow === "now" ? "LATER" : "TODO";
  let next = raw;
  const entries = blockRegions(raw, format).planning.filter(p => p.kind !== "Closed").sort((a,b) => b.timestamp[0]-a.timestamp[0]);
  for (const p of entries) {
    const start = utf8ByteToUtf16Offset(raw,p.timestamp[0]);
    const end = utf8ByteToUtf16Offset(raw,p.timestamp[1]);
    const accepted = raw.slice(start,end);
    const lt = accepted.indexOf("<");
    const adv = advanceTimestamp(accepted.slice(lt));
    if (adv) next = next.slice(0,start+lt) + adv + next.slice(end);
  }
  // The marker is spliced at its recognized offsets (it may follow leading
  // whitespace or a blank line); planning lines come after it, so their
  // advance above leaves those offsets valid (C3 L14).
  return setMarker(next, open);
}

/** Toggle a task's checkbox the way OG's `check`/`uncheck` do: an OPEN task →
 *  `DONE` (but a *repeating* task rolls its date(s) forward and stays open
 *  instead); `DONE` → the workflow's open marker (`TODO`, or `LATER` under the
 *  `now` workflow). Returns the new raw, or null if the block has no checkbox
 *  (no leading marker, or a CANCELED/CANCELLED one). Only line 0's marker word
 *  is rewritten; the rest of the block (properties, SCHEDULED/DEADLINE) is kept. */
/** Whether a marker label click does anything (see toggleMarkerLabel). */
export function markerLabelClickable(marker: string | null | undefined): boolean {
  return marker === "TODO" || marker === "DOING" || marker === "LATER" || marker === "NOW";
}

/** Logseq's marker-label click is deliberately separate from its keyboard
 * cycle: TODO <-> DOING and LATER <-> NOW. DONE and all other markers are not
 * clickable, so a stray label click can never remove completion state. */
export function toggleMarkerLabel(raw: string, time?: MarkerTimeOptions): string | null {
  const current = leadingMarker(raw);
  const target =
    current === "TODO" ? "DOING" :
    current === "DOING" ? "TODO" :
    current === "LATER" ? "NOW" :
    current === "NOW" ? "LATER" :
    null;
  if (!target) return null;
  const next = setMarker(raw, target);
  return time ? applyMarkerTransition(raw, next, time.format, time.enabled, time.withSeconds) : next;
}

export function toggleTaskDone(raw: string, workflow: Workflow, format: Format, time?: MarkerTimeOptions): string | null {
  const cur = leadingMarker(raw);
  const state = taskCheckboxState(cur);
  if (state === null) return null;

  if (state === true) {
    // DONE → open marker (uncheck).
    const next = setMarker(raw, workflow === "now" ? "LATER" : "TODO");
    return time ? applyMarkerTransition(raw, next, time.format, time.enabled, time.withSeconds) : next;
  }
  // OPEN → DONE (check). A repeater rolls forward instead of closing.
  const rolled = rollRepeat(raw, workflow, format);
  if (rolled) return time ? applyMarkerTransition(raw, rolled, time.format, time.enabled, time.withSeconds) : rolled;
  const next = setMarker(raw, "DONE");
  return time ? applyMarkerTransition(raw, next, time.format, time.enabled, time.withSeconds) : next;
}

/** Offset just past the leading marker and its one separating space (0 with no
 *  marker): the same prefix `cycleMarker` measures its caret delta over. */
function markerPrefixEnd(raw: string): number {
  const m = matchLeadingMarker(raw);
  return m ? m.end + (raw[m.end] === " " ? 1 : 0) : 0;
}

/** Cycle the marker, but if the step would mark a *repeating* task DONE, roll it
 *  forward instead. Returns the new raw + caret delta on the first line. */
export function cycleMarkerSmart(raw: string, workflow: Workflow, format: Format, time?: MarkerTimeOptions): { raw: string; delta: number } {
  const cur = leadingMarker(raw);
  if (nextMarker(cur, workflow) === "DONE") {
    const rolled = rollRepeat(raw, workflow, format);
    if (rolled) {
      return {
        raw: time ? applyMarkerTransition(raw, rolled, time.format, time.enabled, time.withSeconds) : rolled,
        delta: markerPrefixEnd(rolled) - markerPrefixEnd(raw),
      };
    }
  }
  const cycled = cycleMarker(raw, workflow);
  return {
    raw: time ? applyMarkerTransition(raw, cycled.raw, time.format, time.enabled, time.withSeconds) : cycled.raw,
    delta: cycled.delta,
  };
}
