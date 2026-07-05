// Repeating tasks. A SCHEDULED/DEADLINE timestamp may carry a repeater, e.g.
// `<2026-06-16 Tue +1w>` (cumulative), `.+1w` (from completion), `++1w`. When a
// repeating task is cycled to DONE, OG instead advances the date(s) to the next
// occurrence and resets the marker to the workflow's open state. Pure + tested.

import { leadingMarker, nextMarker, cycleMarker, type Workflow } from "./marker";
import { taskCheckboxState } from "../markers";
import { applyMarkerTransition } from "../logbook";
import type { Format } from "../types";

const REPEATER = /([.+]{1,2})(\d+)([dwmy])/;
const TS_RE = /<(\d{4})-(\d{2})-(\d{2})(?:\s+[A-Za-z]{3})?(?:\s+([.+]{1,2})(\d+)([dwmy]))?>/;
const WD = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

interface MarkerTimeOptions {
  format: Format;
  enabled: boolean;
  withSeconds: boolean;
}

/** True if the block has a repeater on a SCHEDULED/DEADLINE line. */
export function hasRepeater(raw: string): boolean {
  return raw.split("\n").some((l) => {
    const t = l.trim();
    return (t.startsWith("SCHEDULED:") || t.startsWith("DEADLINE:")) && REPEATER.test(t);
  });
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
    dt = new Date();
    step(dt);
  } else {
    dt = new Date(Number(y), Number(mo) - 1, Number(d));
    if (kind === "++") {
      const today = new Date();
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
export function rollRepeat(raw: string, workflow: Workflow): string | null {
  if (!hasRepeater(raw)) return null;
  const open = workflow === "now" ? "LATER" : "TODO";
  const lines = raw.split("\n");
  for (let i = 0; i < lines.length; i++) {
    const m = /^(\s*)(SCHEDULED|DEADLINE):\s*(<[^>]+>)(.*)$/.exec(lines[i]);
    if (m) {
      const adv = advanceTimestamp(m[3]);
      if (adv) lines[i] = `${m[1]}${m[2]}: ${adv}${m[4]}`;
    }
  }
  const cur = leadingMarker(raw);
  let first = lines[0];
  if (cur) first = first.slice(cur.length).replace(/^ /, "");
  lines[0] = first ? `${open} ${first}` : open;
  return lines.join("\n");
}

/** Toggle a task's checkbox the way OG's `check`/`uncheck` do: an OPEN task →
 *  `DONE` (but a *repeating* task rolls its date(s) forward and stays open
 *  instead); `DONE` → the workflow's open marker (`TODO`, or `LATER` under the
 *  `now` workflow). Returns the new raw, or null if the block has no checkbox
 *  (no leading marker, or a CANCELED/CANCELLED one). Only line 0's marker word
 *  is rewritten; the rest of the block (properties, SCHEDULED/DEADLINE) is kept. */
export function toggleTaskDone(raw: string, workflow: Workflow, time?: MarkerTimeOptions): string | null {
  const cur = leadingMarker(raw);
  const state = taskCheckboxState(cur);
  if (state === null) return null;

  const lines = raw.split("\n");
  const rest = cur ? lines[0].slice(cur.length).replace(/^ /, "") : lines[0];
  if (state === true) {
    // DONE → open marker (uncheck).
    const open = workflow === "now" ? "LATER" : "TODO";
    lines[0] = rest ? `${open} ${rest}` : open;
    const next = lines.join("\n");
    return time ? applyMarkerTransition(raw, next, time.format, time.enabled, time.withSeconds) : next;
  }
  // OPEN → DONE (check). A repeater rolls forward instead of closing.
  const rolled = rollRepeat(raw, workflow);
  if (rolled) return time ? applyMarkerTransition(raw, rolled, time.format, time.enabled, time.withSeconds) : rolled;
  lines[0] = rest ? `DONE ${rest}` : "DONE";
  const next = lines.join("\n");
  return time ? applyMarkerTransition(raw, next, time.format, time.enabled, time.withSeconds) : next;
}

/** Cycle the marker, but if the step would mark a *repeating* task DONE, roll it
 *  forward instead. Returns the new raw + caret delta on the first line. */
export function cycleMarkerSmart(raw: string, workflow: Workflow, time?: MarkerTimeOptions): { raw: string; delta: number } {
  const cur = leadingMarker(raw);
  if (nextMarker(cur, workflow) === "DONE") {
    const rolled = rollRepeat(raw, workflow);
    if (rolled) {
      const open = workflow === "now" ? "LATER" : "TODO";
      const oldLen = cur ? cur.length + 1 : 0;
      return {
        raw: time ? applyMarkerTransition(raw, rolled, time.format, time.enabled, time.withSeconds) : rolled,
        delta: open.length + 1 - oldLen,
      };
    }
  }
  const cycled = cycleMarker(raw, workflow);
  return {
    raw: time ? applyMarkerTransition(raw, cycled.raw, time.format, time.enabled, time.withSeconds) : cycled.raw,
    delta: cycled.delta,
  };
}
