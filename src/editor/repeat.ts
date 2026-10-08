// Repeating tasks. A SCHEDULED/DEADLINE timestamp may carry a repeater, e.g.
// `<2026-06-16 Tue +1w>` (cumulative), `.+1w` (from completion), `++1w`. When a
// repeating task is cycled to DONE, OG instead advances the date(s) to the next
// occurrence and resets the marker to the workflow's open state. Pure + tested.

import { leadingMarker, nextMarker, cycleMarker, setMarker, type Workflow } from "./marker";
import { matchLeadingMarker, taskCheckboxState } from "../markers";
import { applyMarkerTransition } from "../logbook";
import { blockRegions, editBlock } from "../render/parse";
import type { TimestampPoint } from "../render/ast";
import type { Format } from "../types";

import { appNow, localCalendarDate } from "../journal";
const WD = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
export type RepMode = "+" | "++" | ".+";
type Repeater = {mode: RepMode; num: number; unit: string};

/** Decode lsdoc's accepted repetition tuple. UI/completion share this policy:
 * day/week/month/year are supported; zero cookies retain their existing no-advance policy; hourly cookies are unsupported. */
const repetitionCache = new WeakMap<TimestampPoint, Repeater | null>();
function repetition(point: TimestampPoint): Repeater | null {
  if (repetitionCache.has(point)) return repetitionCache.get(point)!;
  const r = point.repetition as [[string], [string], number] | undefined;
  if (!Array.isArray(r) || !Array.isArray(r[0]) || !Array.isArray(r[1])) return null;
  const mode = ({Plus:"+", DoublePlus:"++", Dotted:".+"} as Record<string,RepMode>)[r[0][0]];
  const unit = ({Day:"d", Week:"w", Month:"m", Year:"y"} as Record<string,string>)[r[1][0]];
  const result = mode && unit && r[2] >= 0 ? {mode, unit, num:r[2]} : null;
  repetitionCache.set(point, result);
  return result;
}
const cookie = (r: Repeater | null): string | null => r ? `${r.mode}${r.num}${r.unit}` : null;

/** Project a parser-accepted planning point into calendar UI state. O(1), no parse. */
export function scheduleParts(value: unknown): {y:number; m:number; d:number; time:string|null; repeater:string|null} | null {
  const point = value as TimestampPoint;
  if (!point?.date) return null;
  const {year:y, month, day:d} = point.date;
  if (!localCalendarDate(y, month - 1, d)) return null;
  const time = point.time ? `${String(point.time.hour).padStart(2,"0")}:${String(point.time.min).padStart(2,"0")}` : null;
  return {y, m:month - 1, d, time, repeater:cookie(repetition(point))};
}

/** Read a date-picker cookie using the same parser grammar as task planning.
 * Bounded synthetic timestamp, no regex/fallback grammar, no graph or I/O. */
export function parseRepeater(raw: string | null): Repeater {
  const p = raw ? blockRegions(`SCHEDULED: <2000-01-01 Sat ${raw}>`, "md").planning[0] : null;
  return p && repetition(p.date as TimestampPoint) || {mode:"+", num:1, unit:""};
}

/** One planning writer, using explicit parts. Month is zero-based. O(1). */
export function planningTimestamp(parts: {y:number; m:number; d:number; time?:string|null; repeater?:string|null}): string {
  const date = localCalendarDate(parts.y, parts.m, parts.d);
  if (!date) throw new Error("Invalid planning date");
  return `<${String(parts.y).padStart(4,"0")}-${String(parts.m+1).padStart(2,"0")}-${String(parts.d).padStart(2,"0")} ${WD[date.getDay()]}${parts.time ? ` ${parts.time}` : ""}${parts.repeater ? ` ${parts.repeater}` : ""}>`;
}

interface MarkerTimeOptions {
  format: Format;
  enabled: boolean;
  withSeconds: boolean;
}

/** True when parser-owned planning contains a supported repeater.
 * O(planning entries) on the warm block parse; literals never supply planning. */
export function hasRepeater(raw: string, format: Format): boolean {
  return blockRegions(raw, format).planning.some(p => p.kind !== "Closed" && repetition(p.date as TimestampPoint) !== null);
}

/** Advance accepted planning and reset the marker. Completion-time `. +` and
 * catch-up `++` policies retain the existing calendar rollover semantics.
 * One cached parse; O(planning entries + catch-up steps). */
export function rollRepeat(raw: string, workflow: Workflow, format: Format): string | null {
  let next = raw, rolled = false;
  for (const p of blockRegions(raw, format).planning) {
    if (p.kind === "Closed") continue;
    const point = p.date as TimestampPoint;
    const r = repetition(point), parts = scheduleParts(point);
    if (!r || !parts) continue;
    if (r.num === 0) { rolled = true; continue; }
    const date = r.mode === ".+" ? appNow() : localCalendarDate(parts.y, parts.m, parts.d)!;
    const step = () => {
      if (r.unit === "d") date.setDate(date.getDate() + r.num);
      else if (r.unit === "w") date.setDate(date.getDate() + r.num*7);
      else if (r.unit === "m") date.setMonth(date.getMonth() + r.num);
      else date.setFullYear(date.getFullYear() + r.num);
    };
    const today = appNow(); today.setHours(0,0,0,0);
    let guard = 0;
    do { step(); } while (r.mode === "++" && date <= today && ++guard < 100000);
    next = editBlock(next, format, {kind:"planning", which:p.kind, value:planningTimestamp({
      y:date.getFullYear(), m:date.getMonth(), d:date.getDate(), time:parts.time, repeater:parts.repeater,
    })});
    rolled = true;
  }
  return rolled ? setMarker(next, workflow === "now" ? "LATER" : "TODO") : null;
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
