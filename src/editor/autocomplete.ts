// Pure autocomplete logic for the block editor: detect a `[[`, `#`, or `/`
// trigger at the caret, and apply a chosen completion. No DOM — unit-testable.

import { TEMPLATE_VARS } from "./templateVars";

export type TriggerKind = "page" | "tag" | "command" | "block";

export interface Trigger {
  kind: TriggerKind;
  /** The partial text typed after the trigger marker. */
  query: string;
  /** Index in `raw` where the trigger marker starts. */
  start: number;
  /** Index in `raw` where the query ends (the caret). */
  end: number;
}

/** Detect an active completion trigger immediately before `caret`. */
export function detectTrigger(raw: string, caret: number): Trigger | null {
  // No trigger spans a newline: the `[[` inner forbids it, and `#tag`/`/command`
  // are anchored at line start or after whitespace. So only the CURRENT line's
  // prefix can matter — slicing just that (not the whole `raw[0..caret]`) avoids
  // an O(block length) allocation per keystroke on long blocks. Returned indices
  // are offset back into `raw` by `lineStart`, so callers see absolute positions.
  const lineStart = raw.lastIndexOf("\n", caret - 1) + 1;
  const before = raw.slice(lineStart, caret);

  // [[page and ((block — an opener with no closer since (the line has no
  // newline). Whichever opener sits closer to the caret wins (you can type a
  // `((` inside text that follows a `[[`, or vice-versa, e.g. `{{embed ((`).
  const openPage = before.lastIndexOf("[[");
  const openBlock = before.lastIndexOf("((");
  if (openPage !== -1 && openPage > openBlock) {
    const inner = before.slice(openPage + 2);
    if (!inner.includes("]") && !inner.includes("[")) {
      return { kind: "page", query: inner, start: lineStart + openPage, end: caret };
    }
  }
  if (openBlock !== -1 && openBlock > openPage) {
    const inner = before.slice(openBlock + 2);
    if (!inner.includes(")") && !inner.includes("(")) {
      return { kind: "block", query: inner, start: lineStart + openBlock, end: caret };
    }
  }

  // #tag — `#` at start or after whitespace, followed by tag chars.
  const tag = /(^|\s)#([\w/_.-]*)$/.exec(before);
  if (tag) {
    const start = lineStart + before.length - tag[2].length - 1; // position of '#'
    return { kind: "tag", query: tag[2], start, end: caret };
  }

  // /command — `/` at start or after whitespace.
  const cmd = /(^|\s)\/([\w-]*)$/.exec(before);
  if (cmd) {
    const start = lineStart + before.length - cmd[2].length - 1;
    return { kind: "command", query: cmd[2], start, end: caret };
  }

  return null;
}

/** Replace [start,end) in `raw` with `insert`; caret lands at
 *  `start + caretInInsert` (defaults to end of the inserted text). */
export function applyCompletion(
  raw: string,
  start: number,
  end: number,
  insert: string,
  caretInInsert = insert.length
): { raw: string; caret: number } {
  return {
    raw: raw.slice(0, start) + insert + raw.slice(end),
    caret: start + caretInInsert,
  };
}

/** Build the inserted text for a page reference (`[[Name]]`). */
export function pageInsert(name: string): string {
  return `[[${name}]]`;
}

/** Build the inserted text for a tag (`#name` or `#[[multi word]]`). */
export function tagInsert(name: string): string {
  return /\s/.test(name) ? `#[[${name}]]` : `#${name}`;
}

/** Action commands need runtime behaviour (date stamps, file picker) rather
 *  than a fixed insertion; the editor resolves these when chosen. */
export type CommandAction =
  | "scheduled"
  | "deadline"
  | "upload-asset"
  | "now-time"
  | "today"
  | "query-builder"
  | "page-props"
  | "priority-a"
  | "priority-b"
  | "priority-c";

export interface Command {
  label: string;
  /** Text to insert in place of `/query` (omitted for action commands). */
  insert?: string;
  /** Caret offset within `insert` (default: end). */
  caret?: number;
  /** A runtime action resolved by the editor instead of a literal insert. */
  action?: CommandAction;
  /** Optional short match alias scored in ADDITION to the label, so a one-letter
   *  query surfaces this command first (mirrors OG, whose priority commands are
   *  literally named "A"/"B"/"C" — a full-length exact match that outranks longer
   *  partial matches). Display still uses `label`. */
  key?: string;
}

export const COMMANDS: Command[] = [
  { label: "TODO", insert: "TODO " },
  { label: "DOING", insert: "DOING " },
  { label: "LATER", insert: "LATER " },
  { label: "NOW", insert: "NOW " },
  { label: "DONE", insert: "DONE " },
  { label: "WAITING", insert: "WAITING " },
  { label: "WAIT", insert: "WAIT " },
  { label: "IN-PROGRESS", insert: "IN-PROGRESS " },
  { label: "CANCELED", insert: "CANCELED " },
  { label: "Priority A", action: "priority-a", key: "A" },
  { label: "Priority B", action: "priority-b", key: "B" },
  { label: "Priority C", action: "priority-c", key: "C" },
  { label: "Scheduled", action: "scheduled" },
  { label: "Deadline", action: "deadline" },
  { label: "Heading 1", insert: "# " },
  { label: "Heading 2", insert: "## " },
  { label: "Heading 3", insert: "### " },
  { label: "Heading 4", insert: "#### " },
  { label: "Page reference", insert: "[[]]", caret: 2 },
  { label: "Link", insert: "[]()", caret: 1 },
  { label: "Upload an asset", action: "upload-asset" },
  { label: "Code block", insert: "```\n\n```", caret: 4 },
  { label: "Calculator", insert: "```calc\n\n```", caret: 8 },
  { label: "Quote", insert: "> " },
  // Org-mode admonitions (Logseq's colored callouts). Caret lands on the empty
  // content line between BEGIN/END.
  ...["NOTE", "TIP", "IMPORTANT", "WARNING", "CAUTION"].map((t) => ({
    label: `Admonition: ${t.toLowerCase()}`,
    insert: `#+BEGIN_${t}\n\n#+END_${t}`,
    caret: `#+BEGIN_${t}\n`.length,
  })),
  { label: "Divider", insert: "---" },
  { label: "Query", insert: "{{query }}", caret: 8 },
  { label: "Query (visual builder)", action: "query-builder" },
  { label: "Embed", insert: "{{embed }}", caret: 8 },
  { label: "Math block", insert: "$$$$", caret: 2 },
  { label: "Current time", action: "now-time" },
  { label: "Today", action: "today" },
  { label: "Page properties", action: "page-props" },
  // Template variables: insert the `<% … %>` placeholder (expanded when the
  // template is inserted). The list is shared with the "Make a template" hint.
  ...TEMPLATE_VARS.map((v) => ({ label: `Template var: ${v.label}`, insert: v.insert })),
  { label: "Template var: date…", insert: "<% date:  %>", caret: 9 },
];

// --- Fuzzy ranking (port of OG Logseq's frontend.search/score) ---------------
// Higher = better; a score of 0 means "not a match" (the query isn't even a
// subsequence). The dominant term is a +1000 boost when the query appears as a
// contiguous substring; ties then break on a length-distance term that is 1.0
// for an exact full-length match (so a one-char query "a" ranks the command "A"
// above longer partial matches like "LATER"), and a small per-char run bonus.

const MAX = 1000;

function cleanStr(s: string): string {
  // Lowercase and drop the same punctuation OG ignores when matching.
  return s.toLowerCase().replace(/[[\]\\/_ ()]+/g, "");
}

function strLenDistance(a: string, b: string): number {
  const maxed = Math.max(a.length, b.length);
  if (maxed === 0) return 1;
  const mined = Math.min(a.length, b.length);
  return 1 - (maxed - mined) / maxed;
}

/** OG's fuzzy score for `query` against `str` (case-insensitive). */
export function fuzzyScore(query: string, str: string): number {
  const oq = query.toLowerCase();
  const os = str.toLowerCase();
  const q = cleanStr(oq);
  const s = cleanStr(os);
  let qi = 0;
  let si = 0;
  let mult = 1;
  let score = 0;
  for (;;) {
    if (qi >= q.length) {
      // All query chars matched (as a subsequence). Add the length-distance tie-
      // breaker and the exact-substring boost (on the un-cleaned lowercased text,
      // like OG).
      return score + strLenDistance(q, s) + (os.includes(oq) ? MAX : 0);
    }
    if (si >= s.length) return 0; // query left over, string exhausted → no match
    if (q[qi] === s[si]) {
      score += mult; // reward longer matched runs: mult grows on consecutive hits
      mult += 1;
      qi += 1;
      si += 1;
    } else {
      mult = 1; // gap → reset the run multiplier
      si += 1;
    }
  }
}

/** Fuzzy score for a command against `query`: the best of its label and its
 *  optional short `key` (so a one-letter query can surface it first). */
export function commandScore(query: string, c: Command): number {
  return Math.max(fuzzyScore(query, c.label), c.key ? fuzzyScore(query, c.key) : 0);
}

/** Slash-command matches for `query`, ranked best-first (OG-style). An empty
 *  query (bare `/`) lists every command in its defined order. */
export function filterCommands(query: string): Command[] {
  if (!query) return COMMANDS.slice();
  return COMMANDS.map((c) => ({ c, s: commandScore(query, c) }))
    .filter((x) => x.s > 0)
    .sort((a, b) => b.s - a.s) // stable: equal scores keep their defined order
    .map((x) => x.c);
}
