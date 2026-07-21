// Pure autocomplete logic for the block editor: detect a `[[`, `#`, `/`, `<`, or property
// trigger at the caret, and apply a chosen completion. No DOM — unit-testable.

import { TEMPLATE_VARS } from "./templateVars";
import { isBareTagPrefix, tagRef } from "../tags";
import { propertyKeyNorm } from "../render/block";

export type TriggerKind =
  | "page"
  | "tag"
  | "command"
  | "advanced-command"
  | "block"
  | "code-language"
  | "property-name"
  | "property-value";

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
  /** Canonical key owning a property-value query. */
  property?: string;
}

/** Existing canonical property identity, re-exported for editor authoring. */
export const propertyKeyFold = propertyKeyNorm;

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
export function detectTrigger(
  raw: string,
  caret: number,
  propertyValueKey?: string | null,
): Trigger | null {
  // No trigger spans a newline: the `[[` inner forbids it, `#tag`/`/command`
  // are anchored at line start or after whitespace, and `<command` starts its
  // own logical line. So only the CURRENT line's
  // prefix can matter — slicing just that (not the whole `raw[0..caret]`) avoids
  // an O(block length) allocation per keystroke on long blocks. Returned indices
  // are offset back into `raw` by `lineStart`, so callers see absolute positions.
  const lineStart = raw.lastIndexOf("\n", caret - 1) + 1;
  const before = raw.slice(lineStart, caret);

  // Property completion is line syntax, never inline prose/reference/fence
  // syntax. A value lifecycle exists only after this editor selected its key;
  // merely moving into an existing `key:: value` line must not pop a menu.
  // OG parity: handler/editor.cljs:1907-1924 (name trigger) and :2211-2226
  // (chosen key immediately transitions to value search), checkout 6e7afa8eb.
  if (!insideFenceBefore(raw, lineStart)) {
    if (propertyValueKey) {
      const delimiter = before.indexOf("::");
      if (delimiter > 0) {
        const sourceKey = before.slice(0, delimiter);
        if (
          /^[A-Za-z0-9_./-]+$/.test(sourceKey) &&
          propertyKeyFold(sourceKey) === propertyKeyFold(propertyValueKey)
        ) {
          const afterDelimiter = delimiter + 2;
          const valueOffset = afterDelimiter + (before[afterDelimiter] === " " ? 1 : 0);
          const query = before.slice(valueOffset);
          return {
            kind: "property-value",
            query,
            start: lineStart + valueOffset,
            end: caret,
            property: propertyKeyFold(propertyValueKey),
          };
        }
      }
    }

    // Match the persisted parser's property-key alphabet. In particular, a
    // whitespace-separated prose phrase ending in `::` is not property syntax.
    const propertyName = /^([A-Za-z0-9_./-]*)::$/.exec(before);
    if (propertyName) {
      return {
        kind: "property-name",
        query: propertyName[1],
        start: lineStart,
        end: caret,
      };
    }
  }

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

  // #tag — `#` at start or after whitespace, followed by the same hard-stop
  // contract as lsdoc's bare-tag lexer. Do not use `\w`: it is ASCII-only and
  // closes the picker after CJK/Indic/emoji IME commits. Logseq OG 6e7afa8
  // (`handle-last-input` + `close-autocomplete-if-outside`) likewise starts on
  // the marker boundary and keeps arbitrary committed tag text active.
  const hash = before.lastIndexOf("#");
  if (hash !== -1 && (hash === 0 || /\s/u.test(before[hash - 1]))) {
    const query = before.slice(hash + 1);
    if (isBareTagPrefix(query)) {
      return { kind: "tag", query, start: lineStart + hash, end: caret };
    }
  }

  // /command — `/` at start or after whitespace.
  const cmd = /(^|\s)\/([\w-]*)$/.exec(before);
  if (cmd) {
    const start = lineStart + before.length - cmd[2].length - 1;
    return { kind: "command", query: cmd[2], start, end: caret };
  }

  // Advanced BEGIN/END sections — Tine intentionally requires a logical line
  // start, so ordinary prose such as `word<quote` remains literal. OG opens its
  // block-command action when `<` is typed (og/src/main/frontend/handler/editor.cljs:1901-1905,
  // checkout 6e7afa8eb); this narrower boundary is the frozen Tine contract.
  const advanced = /^<([\w-]*)$/.exec(before);
  if (advanced) {
    return { kind: "advanced-command", query: advanced[1], start: lineStart, end: caret };
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

/** OG accept-range parity (GH #199): when a completion ends in a ref closer
 *  (`]]`/`))`/`}}`), the replacement must extend past the caret to swallow that
 *  closer even when the user typed text between the caret and it (editing inside
 *  an existing ref). Scans the CURRENT LINE only, forward from `end`. Returns the
 *  index just past the closer, or `end` unchanged when there is no matching closer
 *  ahead on this line. */
export function refCompletionEnd(value: string, end: number, insertedText: string): number {
  for (const pair of ["]]", "))", "}}"] as const) {
    if (!insertedText.endsWith(pair)) continue;
    const lineEnd = value.indexOf("\n", end);
    const closer = value.indexOf(pair, end);
    return closer !== -1 && (lineEnd === -1 || closer < lineEnd) ? closer + pair.length : end;
  }
  return end;
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

export type LinkAutocompletePolicy = "adaptive" | "existing" | "typed";

/** An autocomplete row whose canonical page name is still available at this
 * boundary. `PageEntry` does not expose the alias that matched in the backend,
 * so alias hits deliberately use this canonical deterministic fallback. */
export interface NamedAutocompleteItem<T> {
  name: string;
  item: T;
}

function canonicalName(name: string): string {
  return name.normalize("NFC").toLowerCase();
}

function canonicalCompare<T extends NamedAutocompleteItem<unknown>>(a: T, b: T): number {
  const an = canonicalName(a.name);
  const bn = canonicalName(b.name);
  return an.length - bn.length || (an < bn ? -1 : an > bn ? 1 : 0);
}

/** OG 1.0.0 page/tag default-action ordering. Blank queries retain their trigger
 * lifecycle but deliberately expose no rows; the editor also skips quickSwitch
 * for them. Nonblank results are canonical-name ordered so graph/index order is
 * never an accidental Enter policy. */
export function orderAcItems<T>(
  matches: readonly NamedAutocompleteItem<T>[],
  createItem: NamedAutocompleteItem<T>,
  opts: { query: string; policy: LinkAutocompletePolicy },
): T[] {
  const query = canonicalName(opts.query.trim());
  if (!query) return [];
  const exact = matches.filter((match) => canonicalName(match.name) === query).sort(canonicalCompare);
  if (exact.length) return exact.map((match) => match.item);

  const prefix = matches.filter((match) => canonicalName(match.name).startsWith(query)).sort(canonicalCompare);
  const fuzzy = matches.filter((match) => !canonicalName(match.name).startsWith(query)).sort(canonicalCompare);
  const ordered = [...prefix, ...fuzzy];
  if (opts.policy === "typed") return [createItem.item, ...ordered.map((match) => match.item)];
  if (opts.policy === "existing") return [...ordered.map((match) => match.item), createItem.item];
  return prefix.length
    ? [prefix[0].item, createItem.item, ...prefix.slice(1).map((match) => match.item), ...fuzzy.map((match) => match.item)]
    : [createItem.item, ...fuzzy.map((match) => match.item)];
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
  | "priority-c"
  | "page-reference"
  | "insert-link"
  | "heading-auto"
  | "heading-1"
  | "heading-2"
  | "heading-3"
  | "heading-4";

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
  /** Immutable pre-v0.5.10 typed-query tiebreaker. Never derive this from the
   * visible bare-menu order: `/A` and every existing fuzzy tie depend on it. */
  readonly matchTieOrder: number;
  /** Explicit empty-slash menu order. This is intentionally independent of
   * `matchTieOrder`; changing `/` must not perturb `/query`. */
  readonly bareOrder: number;
}

type CommandDefinition = Omit<Command, "matchTieOrder" | "bareOrder">;

const COMMAND_DEFINITIONS: readonly CommandDefinition[] = [
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
  { label: "Heading (Auto)", action: "heading-auto" },
  { label: "Heading 1", action: "heading-1" },
  { label: "Heading 2", action: "heading-2" },
  { label: "Heading 3", action: "heading-3" },
  { label: "Heading 4", action: "heading-4" },
  { label: "Page reference", action: "page-reference" },
  { label: "Link", action: "insert-link" },
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

const BARE_ORDER = new Map<string, number>([
  "Page reference", "Link", "Upload an asset", "Voice recording", "Draw.io diagram",
  "Heading (Auto)", "Heading 1", "Heading 2", "Heading 3", "Heading 4",
  "Today", "Current time",
  "TODO", "DOING", "LATER", "NOW", "DONE", "WAITING", "WAIT", "IN-PROGRESS", "CANCELED", "Scheduled", "Deadline",
  "Priority A", "Priority B", "Priority C",
  "Grid", "Table", "Board",
  "Code block", "Calculator", "Quote",
  "Admonition: note", "Admonition: tip", "Admonition: important", "Admonition: warning", "Admonition: caution",
  "Divider", "Query", "Query (visual builder)", "Embed", "Math block", "Page properties",
].map((label, index) => [label, index]));

/** One registry drives rendering, matching, selection and tests. The old
 * definition order is frozen into matchTieOrder before the bare menu is sorted. */
export const COMMANDS: readonly Command[] = Object.freeze(COMMAND_DEFINITIONS.map((command, matchTieOrder) =>
  Object.freeze({
    ...command,
    matchTieOrder,
    bareOrder: BARE_ORDER.get(command.label) ?? 1000 + matchTieOrder,
  }),
));

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
  if (!query) return COMMANDS.slice().sort((a, b) => a.bareOrder - b.bareOrder);
  return COMMANDS.map((c) => ({ c, s: commandScore(query, c) }))
    .filter((x) => x.s > 0)
    .sort((a, b) => b.s - a.s || a.c.matchTieOrder - b.c.matchTieOrder)
    .map((x) => x.c);
}

export interface AdvancedBlockCommand {
  label: string;
  /** Paired Org section text replacing the active `<query` span. */
  insert: string;
  /** Caret position on the deliberately blank middle line. */
  caret: number;
  readonly matchTieOrder: number;
}

type AdvancedBlockDefinition = {
  label: string;
  type: string;
  optional?: string;
};

function advancedBlockDefinition({ label, type, optional }: AdvancedBlockDefinition): Omit<AdvancedBlockCommand, "matchTieOrder"> {
  const suffix = optional ? ` ${optional}` : "";
  const opening = `#+BEGIN_${type}${suffix}\n`;
  // OG ->block (commands.cljs:175-177): for Src the caret lands at the END of
  // the opening line (to type the language); every other type lands on the
  // blank middle line.
  const caret = type === "SRC" ? opening.length - 1 : opening.length;
  return { label, insert: `${opening}\n#+END_${type}`, caret };
}

/** OG ->block (commands.cljs:159-177): on a MARKDOWN page the Src command
 *  inserts a fenced code block instead of the org-style section; caret after
 *  the opening fence, ready for the language. All other commands and the org
 *  format keep the paired BEGIN/END text. */
export function advancedBlockInsertion(
  command: AdvancedBlockCommand,
  format: "md" | "org",
): { insert: string; caret: number } {
  if (format === "md" && command.label === "Src") {
    return { insert: "```\n\n```", caret: 3 };
  }
  return { insert: command.insert, caret: command.caret };
}

// OG parity: og/src/main/frontend/commands.cljs:155-180 builds paired BEGIN/END
// syntax, and :188-218 defines this exact section-command set at 6e7afa8eb.
// `Properties` is intentionally absent: OG's org-only row invokes ->properties,
// not ->block, so it is not a BEGIN/END advanced section.
export const ADVANCED_BLOCK_COMMANDS: readonly AdvancedBlockCommand[] = Object.freeze([
  { label: "Quote", type: "QUOTE" },
  { label: "Src", type: "SRC" },
  { label: "Query", type: "QUERY" },
  { label: "Latex export", type: "EXPORT", optional: "latex" },
  { label: "Note", type: "NOTE" },
  { label: "Tip", type: "TIP" },
  { label: "Important", type: "IMPORTANT" },
  { label: "Caution", type: "CAUTION" },
  { label: "Pinned", type: "PINNED" },
  { label: "Warning", type: "WARNING" },
  { label: "Example", type: "EXAMPLE" },
  { label: "Export", type: "EXPORT" },
  { label: "Verse", type: "VERSE" },
  { label: "Ascii", type: "EXPORT", optional: "ascii" },
  { label: "Center", type: "CENTER" },
  { label: "Comment", type: "COMMENT" },
].map((definition, matchTieOrder) => Object.freeze({
  ...advancedBlockDefinition(definition),
  matchTieOrder,
})));

/** Fuzzy `<` menu matching uses the same OG-style scorer as slash commands. */
export function filterAdvancedBlockCommands(query: string): AdvancedBlockCommand[] {
  if (!query) return ADVANCED_BLOCK_COMMANDS.slice();
  return ADVANCED_BLOCK_COMMANDS
    .map((command) => ({ command, score: fuzzyScore(query, command.label) }))
    .filter(({ score }) => score > 0)
    .sort((a, b) => b.score - a.score || a.command.matchTieOrder - b.command.matchTieOrder)
    .map(({ command }) => command);
}
