// Helpers to derive a block's *rendered* view from its raw text. Raw stays
// authoritative (round-trip); these are computed projections.

import type { Format } from "./ast";

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

/** A page's pre-block properties as `[key, value]` pairs. Markdown reads
 *  `key:: value` lines; org reads `#+KEY: value` file directives plus a top
 *  `:PROPERTIES:` … `:END:` drawer's `:key: value` lines (org keys lowercased,
 *  as OG/mldoc stores them). Order preserved. */
export function pageProperties(
  preBlock: string | null | undefined,
  format: Format = "md"
): [string, string][] {
  if (!preBlock) return [];
  const out: [string, string][] = [];
  if (format === "org") {
    let inDrawer = false;
    for (const line of preBlock.split("\n")) {
      const t = line.trim();
      if (/^:PROPERTIES:$/i.test(t)) {
        inDrawer = true;
        continue;
      }
      if (/^:END:$/i.test(t)) {
        inDrawer = false;
        continue;
      }
      const dir = /^#\+([A-Za-z0-9_-]+):\s*(.*)$/.exec(t);
      if (dir) {
        out.push([dir[1].toLowerCase(), dir[2].trim()]);
        continue;
      }
      if (inDrawer) {
        const d = /^:([A-Za-z0-9_-]+):\s*(.*)$/.exec(t);
        if (d) out.push([d[1].toLowerCase(), d[2].trim()]);
      }
    }
  } else {
    for (const line of preBlock.split("\n")) {
      if (isPropertyLine(line)) {
        const idx = line.indexOf("::");
        out.push([line.slice(0, idx).trim(), line.slice(idx + 2).trim()]);
      }
    }
  }
  return out;
}

/** The alias names declared by a page's pre-block (`alias::` in markdown,
 *  `#+ALIAS:` / `:alias:` in org), comma-separated. Empty if none. */
export function aliasNames(
  preBlock: string | null | undefined,
  format: Format = "md"
): string[] {
  for (const [k, v] of pageProperties(preBlock, format)) {
    if (k.toLowerCase() === "alias") {
      return v
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

// SCHEDULED:/DEADLINE: planning timestamps yield the date badge (capture group 1).
// OG treats them as timestamp-ONLY lines that must stand alone, but we're
// intentionally lenient (Martin's call — this is rendering only): match the token
// ANYWHERE on a line, including inline after the marker (`TODO SCHEDULED: <…> do
// the thing`) or with text after the `<…>`. We pull the date out for the badge,
// strip the token (+ its surrounding whitespace), and keep whatever text remains
// as body. Deliberate, visible deviation from OG.
const SCHEDULED_RE = /SCHEDULED:\s*<([^>]+)>/;
const DEADLINE_RE = /DEADLINE:\s*<([^>]+)>/;
const SCHEDULED_STRIP = /\s*SCHEDULED:\s*<[^>]+>\s*/;
const DEADLINE_STRIP = /\s*DEADLINE:\s*<[^>]+>\s*/;

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
    const sm = SCHEDULED_RE.exec(line);
    const dm = DEADLINE_RE.exec(line);
    if (sm || dm) {
      let rest = line;
      if (sm) {
        scheduled = sm[1];
        rest = rest.replace(SCHEDULED_STRIP, " ");
      }
      if (dm) {
        deadline = dm[1];
        rest = rest.replace(DEADLINE_STRIP, " ");
      }
      rest = rest.trim();
      if (rest) lines.push(rest); // keep marker/text that shared the line; date is now a badge
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
  // Drop leading blank body lines so a marker/scheduled-only first line (whose
  // text became a badge) doesn't render as a spurious blank line above the body —
  // e.g. `TODO \nSCHEDULED: <…> do the thing` should render the marker, the text,
  // and the date badge all on one line, not under an empty line.
  while (lines.length > 1 && lines[0].trim() === "") lines.shift();
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
