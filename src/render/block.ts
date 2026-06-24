// Helpers to derive a block's *rendered* view from its raw text. Raw stays
// authoritative (round-trip); these are computed projections.

export const MARKERS = [
  "TODO",
  "DOING",
  "DONE",
  "NOW",
  "LATER",
  "WAITING",
  "WAIT",
  "CANCELED",
  "CANCELLED",
  "IN-PROGRESS",
];

const PROP_RE = /^[A-Za-z0-9_./-]+::\s?.*$/;

export function isPropertyLine(line: string): boolean {
  const idx = line.indexOf("::");
  if (idx <= 0) return false;
  const key = line.slice(0, idx).trim();
  return key.length > 0 && /^[A-Za-z0-9_./-]+$/.test(key) && PROP_RE.test(line);
}

/** The alias names declared by a page's `alias::` pre-block property
 *  (comma-separated, as Logseq stores them). Empty array if there are none. */
export function aliasNames(preBlock: string | null | undefined): string[] {
  if (!preBlock) return [];
  for (const line of preBlock.split("\n")) {
    const idx = line.indexOf("::");
    if (idx <= 0) continue;
    if (line.slice(0, idx).trim().toLowerCase() === "alias") {
      return line
        .slice(idx + 2)
        .split(",")
        .map((s) => s.trim())
        .filter(Boolean);
    }
  }
  return [];
}

export interface BlockView {
  marker: string | null;
  done: boolean;
  priority: "A" | "B" | "C" | null;
  headingLevel: number | null;
  /** Body lines with marker/priority/heading prefix and SCHEDULED/DEADLINE
   *  lines stripped. */
  lines: string[];
  scheduled: string | null;
  deadline: string | null;
  properties: [string, string][];
}

const SCHEDULED_RE = /^SCHEDULED:\s*<([^>]+)>/;
const DEADLINE_RE = /^DEADLINE:\s*<([^>]+)>/;

export function blockView(raw: string): BlockView {
  const allLines = raw.split("\n");
  const properties: [string, string][] = [];
  const lines: string[] = [];
  let scheduled: string | null = null;
  let deadline: string | null = null;
  // Skip org drawers (`:LOGBOOK:`/`:PROPERTIES:` … `:END:`) and bare CLOCK lines
  // so they don't render as literal text. Their content stays in `raw`.
  let inDrawer = false;
  for (const line of allLines) {
    const t = line.trim();
    if (inDrawer) {
      if (/^:END:$/i.test(t)) inDrawer = false;
      continue;
    }
    if (/^:(LOGBOOK|PROPERTIES):$/i.test(t)) {
      inDrawer = true;
      continue;
    }
    if (/^CLOCK:\s/i.test(t)) continue;
    const sm = SCHEDULED_RE.exec(t);
    const dm = DEADLINE_RE.exec(t);
    if (sm) {
      scheduled = sm[1];
    } else if (dm) {
      deadline = dm[1];
    } else if (isPropertyLine(line)) {
      const idx = line.indexOf("::");
      properties.push([line.slice(0, idx).trim(), line.slice(idx + 2).trim()]);
    } else {
      lines.push(line);
    }
  }
  if (lines.length === 0) lines.push("");

  let first = lines[0];
  let marker: string | null = null;
  for (const m of MARKERS) {
    if (first === m || first.startsWith(m + " ")) {
      marker = m;
      first = first.slice(m.length).replace(/^ /, "");
      break;
    }
  }

  let priority: "A" | "B" | "C" | null = null;
  const pm = /^\[#([ABC])\]\s?/.exec(first);
  if (pm) {
    priority = pm[1] as "A" | "B" | "C";
    first = first.slice(pm[0].length);
  }

  let headingLevel: number | null = null;
  const hm = /^(#{1,6}) /.exec(first);
  if (hm) {
    headingLevel = hm[1].length;
    first = first.slice(hm[1].length + 1);
  }

  lines[0] = first;
  return {
    marker,
    done: marker === "DONE" || marker === "CANCELED" || marker === "CANCELLED",
    priority,
    headingLevel,
    lines,
    scheduled,
    deadline,
    properties,
  };
}
