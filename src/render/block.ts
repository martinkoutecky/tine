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
  /** GFM checkbox state of a `[ ]`/`[x]` list-item block — independent of the
   *  TODO/marker system (a tickable checklist item that is NOT an agenda task). */
  checkbox: "unchecked" | "checked" | null;
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

  // GFM checkbox `[ ]`/`[x]` at the head of a (non-task) block → a tickable
  // checklist item. Distinct from TODO markers: no agenda/marker semantics. The
  // `[ ]`/`[x]` stays verbatim in `raw`; only the badge is rendered.
  let checkbox: "unchecked" | "checked" | null = null;
  if (marker === null) {
    const cm = /^\[([ xX])\] /.exec(first);
    if (cm) {
      checkbox = cm[1] === " " ? "unchecked" : "checked";
      first = first.slice(cm[0].length);
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
    checkbox,
    priority,
    headingLevel,
    lines,
    scheduled,
    deadline,
    properties,
  };
}
