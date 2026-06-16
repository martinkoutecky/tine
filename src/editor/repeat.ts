// Repeating tasks. A SCHEDULED/DEADLINE timestamp may carry a repeater, e.g.
// `<2026-06-16 Tue +1w>` (cumulative), `.+1w` (from completion), `++1w`. When a
// repeating task is cycled to DONE, OG instead advances the date(s) to the next
// occurrence and resets the marker to the workflow's open state. Pure + tested.

import { leadingMarker, nextMarker, cycleMarker, type Workflow } from "./marker";

const REPEATER = /([.+]{1,2})(\d+)([dwmy])/;
const TS_RE = /<(\d{4})-(\d{2})-(\d{2})(?:\s+[A-Za-z]{3})?(?:\s+([.+]{1,2})(\d+)([dwmy]))?>/;
const WD = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

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
  // `.+` repeats from the completion date (today); others from the stored date.
  const dt = kind === ".+" ? new Date() : new Date(Number(y), Number(mo) - 1, Number(d));
  if (unit === "d") dt.setDate(dt.getDate() + num);
  else if (unit === "w") dt.setDate(dt.getDate() + num * 7);
  else if (unit === "m") dt.setMonth(dt.getMonth() + num);
  else if (unit === "y") dt.setFullYear(dt.getFullYear() + num);
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

/** Cycle the marker, but if the step would mark a *repeating* task DONE, roll it
 *  forward instead. Returns the new raw + caret delta on the first line. */
export function cycleMarkerSmart(raw: string, workflow: Workflow): { raw: string; delta: number } {
  const cur = leadingMarker(raw);
  if (nextMarker(cur, workflow) === "DONE") {
    const rolled = rollRepeat(raw, workflow);
    if (rolled) {
      const open = workflow === "now" ? "LATER" : "TODO";
      const oldLen = cur ? cur.length + 1 : 0;
      return { raw: rolled, delta: open.length + 1 - oldLen };
    }
  }
  return cycleMarker(raw, workflow);
}
