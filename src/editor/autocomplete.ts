// Pure autocomplete logic for the block editor: detect a `[[`, `#`, or `/`
// trigger at the caret, and apply a chosen completion. No DOM — unit-testable.

export type TriggerKind = "page" | "tag" | "command";

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
  const before = raw.slice(0, caret);

  // [[page — open brackets with no closing/newline since.
  const open = before.lastIndexOf("[[");
  if (open !== -1) {
    const inner = before.slice(open + 2);
    if (!inner.includes("]") && !inner.includes("[") && !inner.includes("\n")) {
      return { kind: "page", query: inner, start: open, end: caret };
    }
  }

  // #tag — `#` at start or after whitespace, followed by tag chars.
  const tag = /(^|\s)#([\w/_.-]*)$/.exec(before);
  if (tag) {
    const start = before.length - tag[2].length - 1; // position of '#'
    return { kind: "tag", query: tag[2], start, end: caret };
  }

  // /command — `/` at start or after whitespace.
  const cmd = /(^|\s)\/([\w-]*)$/.exec(before);
  if (cmd) {
    const start = before.length - cmd[2].length - 1;
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
  { label: "Priority A", action: "priority-a" },
  { label: "Priority B", action: "priority-b" },
  { label: "Priority C", action: "priority-c" },
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
  { label: "Quote", insert: "> " },
  { label: "Divider", insert: "---" },
  { label: "Query", insert: "{{query }}", caret: 8 },
  { label: "Embed", insert: "{{embed }}", caret: 8 },
  { label: "Math block", insert: "$$$$", caret: 2 },
  { label: "Current time", action: "now-time" },
  { label: "Today", action: "today" },
];

export function filterCommands(query: string): Command[] {
  const q = query.toLowerCase();
  return COMMANDS.filter((c) => c.label.toLowerCase().includes(q));
}
