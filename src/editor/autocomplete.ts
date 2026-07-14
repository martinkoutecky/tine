// Pure autocomplete logic for the block editor: detect a `[[`, `#`, or `/`
// trigger at the caret, and apply a chosen completion. No DOM — unit-testable.

import { TEMPLATE_VARS } from "./templateVars";
import { tagRef } from "../tags";

export type TriggerKind = "page" | "tag" | "command" | "block" | "code-language";

export interface CodeLanguageItem {
  /** Canonical highlight.js/common identifier written to the fence. */
  id: string;
  label: string;
  aliases: readonly string[];
}

// Runtime rendering lazy-loads highlight.js/lib/common. Keep this small static
// mirror so opening the editor does not eagerly pull the highlighter into the
// main bundle; a drift test compares it with the pinned dependency's registry.
export const COMMON_CODE_LANGUAGES: readonly CodeLanguageItem[] = [
  { id: "javascript", label: "JavaScript", aliases: ["js", "jsx", "mjs", "cjs"] },
  { id: "typescript", label: "TypeScript", aliases: ["ts", "tsx", "mts", "cts"] },
  { id: "python", label: "Python", aliases: ["py", "gyp", "ipython"] },
  { id: "bash", label: "Bash", aliases: ["sh", "zsh"] },
  { id: "json", label: "JSON", aliases: ["jsonc"] },
  { id: "markdown", label: "Markdown", aliases: ["md", "mkdown", "mkd"] },
  { id: "xml", label: "HTML, XML", aliases: ["html", "xhtml", "rss", "atom", "xjb", "xsd", "xsl", "plist", "wsf", "svg"] },
  { id: "css", label: "CSS", aliases: [] },
  { id: "sql", label: "SQL", aliases: [] },
  { id: "java", label: "Java", aliases: ["jsp"] },
  { id: "c", label: "C", aliases: ["h"] },
  { id: "cpp", label: "C++", aliases: ["cc", "c++", "h++", "hpp", "hh", "hxx", "cxx"] },
  { id: "csharp", label: "C#", aliases: ["cs", "c#"] },
  { id: "go", label: "Go", aliases: ["golang"] },
  { id: "rust", label: "Rust", aliases: ["rs"] },
  { id: "kotlin", label: "Kotlin", aliases: ["kt", "kts"] },
  { id: "swift", label: "Swift", aliases: [] },
  { id: "php", label: "php", aliases: [] },
  { id: "ruby", label: "Ruby", aliases: ["rb", "gemspec", "podspec", "thor", "irb"] },
  { id: "yaml", label: "YAML", aliases: ["yml"] },
  { id: "plaintext", label: "Plain text", aliases: ["text", "txt"] },
  { id: "diff", label: "Diff", aliases: ["patch"] },
  { id: "graphql", label: "GraphQL", aliases: ["gql"] },
  { id: "ini", label: "TOML, also INI", aliases: ["toml"] },
  { id: "less", label: "Less", aliases: [] },
  { id: "lua", label: "Lua", aliases: ["pluto"] },
  { id: "makefile", label: "Makefile", aliases: ["mk", "mak", "make"] },
  { id: "perl", label: "Perl", aliases: ["pl", "pm"] },
  { id: "objectivec", label: "Objective-C", aliases: ["mm", "objc", "obj-c", "obj-c++", "objective-c++"] },
  { id: "php-template", label: "PHP template", aliases: [] },
  { id: "python-repl", label: "python-repl", aliases: ["pycon"] },
  { id: "r", label: "R", aliases: [] },
  { id: "scss", label: "SCSS", aliases: [] },
  { id: "shell", label: "Shell Session", aliases: ["console", "shellsession"] },
  { id: "vbnet", label: "Visual Basic .NET", aliases: ["vb"] },
  { id: "wasm", label: "WebAssembly", aliases: [] },
];

export interface Trigger {
  kind: TriggerKind;
  /** The partial text typed after the trigger marker. */
  query: string;
  /** Index in `raw` where the trigger marker starts. */
  start: number;
  /** Index in `raw` where the query ends (the caret). */
  end: number;
}

/** True when the current line starts inside a preceding Markdown fence. A
 * fence-looking line inside code is content/closing syntax, never an opening
 * language declaration. */
function insideFenceBefore(raw: string, lineStart: number): boolean {
  let open: { char: "`" | "~"; len: number } | null = null;
  for (const line of raw.slice(0, lineStart).split("\n")) {
    const match = /^ {0,3}(`{3,}|~{3,})(.*)$/.exec(line);
    if (!match) continue;
    const fence = match[1];
    const char = fence[0] as "`" | "~";
    if (!open) {
      open = { char, len: fence.length };
    } else if (char === open.char && fence.length >= open.len && match[2].trim() === "") {
      open = null;
    }
  }
  return open !== null;
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

  // Opening Markdown fence language. Do not pop a menu for a bare fence typed
  // by hand (Enter keeps its established behavior); one language character is
  // enough. The /Code block command explicitly opens the empty picker instead.
  const fence = /^( {0,3})(`{3,}|~{3,})([\w+#.-]+)$/.exec(before);
  if (fence && !insideFenceBefore(raw, lineStart)) {
    const start = lineStart + fence[1].length + fence[2].length;
    return { kind: "code-language", query: fence[3], start, end: caret };
  }

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

/** OG-style bracket auto-pairing for page refs, run AFTER the browser has
 *  applied a single typed character. `value`/`caret` are the post-input textarea
 *  state; `typed` is the inserted char. Returns the adjusted `{value, caret}`, or
 *  null when nothing should change.
 *
 *  Two behaviours, both scoped to `]`/`[` so we don't auto-pair every bracket:
 *  - Typing the second `[` of a `[[` auto-inserts the matching `]]` (caret stays
 *    between → `[[|]]`), so a page ref is pre-closed like OG. Skipped if a `]`
 *    already follows (editing inside an existing ref).
 *  - Typing a `]` immediately before an existing `]` types THROUGH it instead of
 *    stacking a new one — so manually closing an auto-paired `[[…]]` can never
 *    pile up to `]]]]`. (The autocomplete-picker path is handled separately in
 *    Block.tsx's replaceTrigger, which swallows a trailing `]]`.) */
export function autoPairEdit(
  value: string,
  caret: number,
  typed: string
): { value: string; caret: number } | null {
  if (typed === "[" && caret >= 2 && value.slice(caret - 2, caret) === "[[" && value[caret] !== "]") {
    return { value: value.slice(0, caret) + "]]" + value.slice(caret), caret };
  }
  if (typed === "]" && value[caret] === "]") {
    // The char at caret-1 is the `]` we just typed; drop it and step over the
    // pre-existing `]` so the closer isn't duplicated.
    return { value: value.slice(0, caret - 1) + value.slice(caret), caret };
  }
  return null;
}

/** Normalize Chinese IME full-width page-ref openers to the existing ASCII
 *  auto-paired ref form. `value`/`caret` are the post-input textarea state. */
export function fullWidthRefReplace(value: string, caret: number): { value: string; caret: number } | null {
  if (caret >= 2 && value.slice(caret - 2, caret) === "【【") {
    return { value: value.slice(0, caret - 2) + "[[]]" + value.slice(caret), caret };
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

/** After accepting a page/block-ref completion, optionally insert a single space
 *  after the closing `]]`/`))` so typing continues cleanly (GH #35, Tine default).
 *  Applies only when `insertedText` ends in a ref pair and `caret` sits right after
 *  it; a no-op when disabled, when the completion isn't a ref, or when a space (or
 *  end-of-buffer immediately followed by a space) already sits there — never doubles
 *  a space. Pure: returns the adjusted `raw`+`caret`. */
export function withRefCompletionSpace(
  raw: string,
  caret: number,
  insertedText: string,
  enabled: boolean
): { raw: string; caret: number } {
  if (!enabled) return { raw, caret };
  if (!/(\]\]|\)\))$/.test(insertedText)) return { raw, caret };
  if (raw[caret] === " ") return { raw, caret };
  return { raw: raw.slice(0, caret) + " " + raw.slice(caret), caret: caret + 1 };
}

/** Build the inserted text for a tag (`#name` or `#[[multi word]]`). */
export function tagInsert(name: string): string {
  return tagRef(name);
}

/** Order the `[[`/`#` completion list, deciding which item is the DEFAULT (first,
 *  Enter) action. A blank query or an exact existing match shows the matches with
 *  no "Create" option. Otherwise `linkFirst` chooses the default: false (OG) →
 *  "Create" leads (Enter makes a new page/tag); true → the first match leads
 *  (Enter links it) and "Create" trails. The other items stay reachable by arrow. */
export function orderAcItems<T>(
  matches: T[],
  createItem: T,
  opts: { hasQuery: boolean; exact: boolean; linkFirst: boolean }
): T[] {
  if (!opts.hasQuery || opts.exact) return matches;
  return opts.linkFirst ? [...matches, createItem] : [createItem, ...matches];
}

/** Action commands need runtime behaviour (date stamps, file picker) rather
 *  than a fixed insertion; the editor resolves these when chosen. */
export type CommandAction =
  | "scheduled"
  | "deadline"
  | "upload-asset"
  | "record"
  | "drawio"
  | "now-time"
  | "today"
  | "query-builder"
  | "page-props"
  | "code-block"
  | "sheet-grid"
  | "sheet-table"
  | "sheet-board"
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
  { label: "Grid", action: "sheet-grid" },
  { label: "Table", action: "sheet-table" },
  // "Kanban" alias: the fuzzy matcher scores label + key, so /kanban (and /kan)
  // surfaces the Board command even though its display name stays "Board".
  { label: "Board", action: "sheet-board", key: "Kanban" },
  { label: "Heading 1", insert: "# " },
  { label: "Heading 2", insert: "## " },
  { label: "Heading 3", insert: "### " },
  { label: "Heading 4", insert: "#### " },
  { label: "Page reference", insert: "[[]]", caret: 2 },
  { label: "Link", insert: "[]()", caret: 1 },
  { label: "Upload an asset", action: "upload-asset" },
  { label: "Voice recording", action: "record" },
  { label: "Draw.io diagram", action: "drawio" },
  { label: "Code block", action: "code-block" },
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

/** Languages actually bundled by highlight.js/common, ranked by canonical id,
 * readable name, and aliases. Accepting an alias always writes the canonical id
 * so rendering and future edits have one stable representation. */
export function codeLanguageItems(query: string): CodeLanguageItem[] {
  if (!query) return COMMON_CODE_LANGUAGES.slice();
  return COMMON_CODE_LANGUAGES
    .map((item, index) => ({
      item,
      index,
      score: Math.max(
        fuzzyScore(query, item.id),
        fuzzyScore(query, item.label),
        ...item.aliases.map((alias) => fuzzyScore(query, alias)),
      ),
    }))
    .filter(({ score }) => score > 0)
    .sort((a, b) => b.score - a.score || a.index - b.index)
    .map(({ item }) => item);
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
