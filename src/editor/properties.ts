import { msg, render } from "../i18n";
// Pure helpers for reading/editing `key:: value` property lines — a block's
// continuation lines or a page's pre-block. No store/DOM, so unit-testable.

import { displayMathOpenAfter, closesDisplayMath, fenceExitTrim } from "./fences";
import type { Format } from "../render/ast";
import { blockRegions, editorOffsetOfRaw, editorText, editorTextJoin, hidePolicyHides, parserReady, rawByteOfEditor } from "../render/parse";
import { utf8ToUtf16Cursor } from "../render/utf16Cursor";
import { reportUiFailure } from "../uiFailure";

import { property_line_json, page_regions_json, page_header_json } from "../render/wasm/lsdoc_wasm.js";
import type { RegionProperty } from "../render/parse";

// Deferred catalog messages; property getters resolve in the consuming computation.
const UI_MESSAGES = {
  "editor-aliases": msg("editor-aliases"),
  "editor-other-names-this-page-answers-to-in-links-comma-separated": msg("editor-other-names-this-page-answers-to-in-links-comma-separated"),
  "editor-tags": msg("editor-tags"),
  "editor-page-level-tags-comma-separated": msg("editor-page-level-tags-comma-separated"),
  "editor-display-title": msg("editor-display-title"),
  "editor-override-the-shown-title-the-file-name-stays-the-same": msg("editor-override-the-shown-title-the-file-name-stays-the-same"),
  "editor-icon": msg("editor-icon"),
  "editor-an-emoji-character-shown-with-the-title": msg("editor-an-emoji-character-shown-with-the-title"),
  "editor-public": msg("editor-public"),
  "editor-include-this-page-when-exporting-publishing-public-pages": msg("editor-include-this-page-when-exporting-publishing-public-pages"),
};

/** Native accepted line grammar; callers choose placement and authoring policy. */
export function acceptedPropertyLine(line: string): { key: string; value: string } | null {
  if (!line) return null;
  const pair = JSON.parse(property_line_json(line)) as [string, string] | null;
  return pair ? { key: pair[0], value: pair[1] } : null;
}

/** Whether the properties panel may write `key` (GH #164): letters, marks,
 *  digits, `_`, `.`, `/` or `-` — the intersection of Tine's Markdown page
 *  header, Org drawer/directive readers and lsdoc. The Rust block-property
 *  reader now accepts this class too. Syntactic only: machine-managed keys (`id`,
 *  `collapsed`, `tine.*`) pass here and callers refuse them separately.
 *  Pinned by crates/tine-core/tests/fixtures/editable-property-keys.txt, which
 *  the Rust readers test too. Cost O(key). */
export function isEditablePropertyKey(key: string): boolean {
  return /^[\p{L}\p{M}\p{N}_./-]+$/u.test(key);
}

/** A Markdown page header the parser accepted (Rust `block_regions::page_header`): the leading run of
 * accepted `key:: value` properties, joined only by empty lines. Named Tine policies over lsdoc's
 * answer: the run starts at the first byte, lines start at column zero, and a key does not start
 * with `#` (a `#tag::` line is prose that happens to parse). No-space `key::value`, indented lines
 * and fenced lines are not properties because lsdoc says so (I-12). Offsets here are UTF-16: `end`
 * is the end of the last header line (newline excluded), `line` is the 0-based line index. */
interface PageHeader {
  end: number;
  entries: { key: string; value: string; line: number }[];
}
const NO_HEADER: PageHeader = { end: 0, entries: [] };
let lastHeader: { raw: string; header: PageHeader } | undefined;

function pageHeader(raw: string): PageHeader {
  // Every header line holds `::`; without one there is nothing to parse.
  if (!raw.includes("::") || !parserReady()) return NO_HEADER;
  if (lastHeader?.raw === raw) return lastHeader.header;
  let json: string;
  try {
    json = page_header_json(raw);
  } catch (error) {
    // A parser trap (the glue already recovered a fresh instance) must not take the page down:
    // read it as "no header" for this text, the same answer as before the parser is ready, and say so (I-9).
    reportUiFailure("page-header-read", error);
    return NO_HEADER;
  }
  const parsed = JSON.parse(json) as { end: number; entries: PageHeader["entries"] };
  const header = { end: utf8ToUtf16Cursor(raw)(parsed.end), entries: parsed.entries.map(({ key, value, line }) => ({ key, value, line })) };
  lastHeader = { raw, header };
  return header;
}

/** A complete page header: the whole text is header properties, with empty lines permitted only
 * between properties (never at either edge). */
export function isPageHeaderPropertiesOnly(raw: string): boolean {
  const header = pageHeader(raw);
  return header.entries.length > 0 && header.end === raw.length;
}

/** The first line that keeps `raw` from being a complete page header, as
 * `{ line, text }` (1-based line number, trimmed text), or null when `raw` is a
 * complete header or has no such line. The line is the first non-empty one after
 * the leading run of accepted properties (the whole first line when `raw` does
 * not start with one): the same boundary {@link isPageHeaderPropertiesOnly}
 * judges, so a message built from it names what that predicate refused. */
export function pageHeaderBadLine(raw: string): { line: number; text: string } | null {
  const header = pageHeader(raw);
  if (header.entries.length > 0 && header.end === raw.length) return null;
  let offset = header.entries.length > 0 ? header.end : 0;
  for (const line of raw.slice(offset).split("\n")) {
    if (line.trim() !== "") return { line: raw.slice(0, offset).split("\n").length, text: line.trim() };
    offset += line.length + 1;
  }
  return null;
}

/** Properties that describe the block they sit on, never a page: a first
 *  bullet carrying one stays an outline block. An empty numbered-list item is
 *  exactly `logseq.order-list-type:: number`, and folding it into the page
 *  header turned the list into page properties and jammed every later save
 *  (GH #540). The `tine.*` keys below are the table/grid/board/query block's
 *  own view configuration (GH #210): such a block has rows as children and its
 *  title is typed after its view is chosen, so folding it into the preamble made
 *  every later edit an invalid page header. After "Show children as: Outline"
 *  removes `tine.view` the leftover config keys must not turn the block into a
 *  page header either, so the whole family is scoped. `tine.fields`, `tine.type`
 *  and `tine.tag-table` are NOT here: they are also page-level schema keys
 *  (SheetTable / key pages write them as page properties), so a first bullet
 *  holding only those is a legitimate page header. Intentional difference from
 *  OG: OG never authors `tine.*` (Martin, D-20261018-H210). Mirror of Rust
 *  `BLOCK_SCOPED_PROPERTY_KEYS` (tine-store model.rs, which carries the OG
 *  provenance for the rest); `src/i12PageHeader.test.ts` keeps them equal. */
export const BLOCK_SCOPED_PROPERTY_KEYS: readonly string[] = [
  "id",
  "heading",
  "collapsed",
  "background-color",
  "logseq.order-list-type",
  "tine.view",
  "tine.header",
  "tine.col-widths",
  "tine.col-aggregates",
  "tine.group-by",
  "tine.group-field",
  "tine.table-widths",
  "tine.columns",
  "tine.block-columns",
  "tine.filter",
  "tine.sort",
  "tine.sample",
  "tine.result-kinds",
];

/** The ONE predicate for "this first root's raw is a page header" (Rust twin:
 * `is_page_header_candidate`, model.rs). Display (`firstPropertiesId`, Block),
 * edit (`pagePropertyParts`, `beginPageHeaderEdit`, `setRaw` title intent) and
 * save-time promotion (`isPromotablePageHeaderRoot`) all call it, so a block
 * cannot be hidden as the header in one place and edited as ordinary text in
 * another: canonical page-header properties and no block-scoped key. */
export function isPropertiesOnly(raw: string): boolean {
  return isPageHeaderPropertiesOnly(raw) && !pageHeaderKeys(raw).some((key) => BLOCK_SCOPED_PROPERTY_KEYS.includes(key));
}

/** Whether a first root renders as the page header. A marked root IS the preamble
 * (`beginPageHeaderEdit`), so the draft rule decides, scoped keys included (`id::`,
 * page-level `tine.table-widths::`); only an unmarked first bullet goes through
 * `isPropertiesOnly`, which keeps block-scoped keys out of the header (GH #210 F4). */
export function rendersAsPageHeader(raw: string, originatedFromPageHeader: boolean | undefined): boolean {
  return originatedFromPageHeader ? isPageHeaderPropertiesOnly(canonicalPageHeaderDraft(raw)) : isPropertiesOnly(raw);
}

/** A marked page-header draft (`beginPageHeaderEdit`) as the file holds it: the
 * blank lines Enter leaves at its edges dropped. Trailing ones come from Enter
 * after the last property; leading ones from Enter (or a pasted or IME newline)
 * at the very start, which OG's page-properties editor also takes as a newline
 * (`dwim-in-properties`) and its save then drops (`wrap-parse-block`,
 * `string/triml`, `handler/editor.cljs:298`). GH #210 F1: without the leading
 * trim that draft was invalid and the page stopped saving. Inner text, inner
 * blank lines and a first line's own indentation are kept. O(raw). */
export function canonicalPageHeaderDraft(raw: string): string {
  return withoutLeadingBlankLines(raw).replace(/\n+$/, "");
}

/** `raw` without its leading blank lines: the part of
 * {@link canonicalPageHeaderDraft} a header's live text may lose on exit or
 * be read through, while its trailing newline is still the caret's line. */
export function withoutLeadingBlankLines(raw: string): string {
  return raw.replace(/^(?:[ \t]*\r?\n)+/, "");
}

/** Whether the textarea caret is on a complete `key:: value` line. This is
 * deliberately line-local: an empty line after a run of page properties is the
 * double-Enter exit sentinel, not another property line. The line is lsdoc's accepted
 * property line under the page-header policies (column zero, no `#` key). */
export function caretOnPropertyLine(raw: string, caret: number): boolean {
  const c = Math.max(0, Math.min(caret, raw.length));
  const lineStart = raw.lastIndexOf("\n", c - 1) + 1;
  const nextNewline = raw.indexOf("\n", c);
  const line = raw.slice(lineStart, nextNewline === -1 ? raw.length : nextNewline);
  const property = acceptedPropertyLine(line);
  return property !== null && line.startsWith(property.key) && !property.key.startsWith("#");
}

/** Split a Markdown page preamble into real page-property lines and ordinary
 * content. Property-looking text inside a fenced code block stays content. */
export function splitPagePreamble(raw: string | null | undefined): {
  properties: string | null;
  content: string | null;
  /** Exact suffix following the canonical header, including separator newlines.
   * Re-concatenating `properties + remainder` reproduces the original bytes. */
  remainder: string | null;
} {
  if (!raw) return { properties: null, content: null, remainder: null };
  const { end, entries } = pageHeader(raw);
  const remainder = entries.length ? raw.slice(end) || null : raw;
  return {
    properties: entries.length ? raw.slice(0, end) : null,
    content: remainder?.replace(/^\n+|\n+$/g, "") || null,
    remainder,
  };
}

/** Which accepted primary properties an editor hides: the outline kernel's
 *  `editor_text::HidePolicy` (twin 13). The kernel owns the key lists. */
export type HidePolicy =
  | { readonly kind: "builtin" | "sheet_cell" | "all" }
  | { readonly kind: "keys"; readonly keys: readonly string[]; readonly norm?: boolean };
/** A normal block editor: the built-in `id::`, `collapsed::` and
 *  `logseq.order-list-type::`, kept in the file but never in the textarea (as OG). */
export const BUILTIN_HIDE: HidePolicy = Object.freeze({ kind: "builtin" });
/** A sheet cell: the built-ins plus every `tine.*` key. */
export const SHEET_CELL_HIDE: HidePolicy = Object.freeze({ kind: "sheet_cell" });
/** Every property (annotation blocks edit only their text). */
export const HIDE_ALL: HidePolicy = Object.freeze({ kind: "all" });
/** Only the block's `id::` (copies and pastes that must not duplicate it). */
export const HIDE_ID: HidePolicy = Object.freeze({ kind: "keys", keys: ["id"] });
/** Exactly `keys` (lower-cased); `norm` compares through `propertyKeyNorm`. */
export const hideKeys = (keys: readonly string[], norm = false): HidePolicy => Object.freeze({ kind: "keys", keys, norm });
const policyJson = new WeakMap<HidePolicy, string>();
function policyText(policy: HidePolicy): string {
  let json = policyJson.get(policy);
  if (json === undefined) policyJson.set(policy, (json = JSON.stringify(policy)));
  return json;
}
/** Whether `policy` hides property `key` (as the parser spelled it). */
export function hides(policy: HidePolicy, key: string): boolean {
  return hidePolicyHides(policyText(policy), key);
}

/** For a multi-line editor that normally keeps Enter inside it, return the text
 * with its trailing sentinel blank line removed when the caret is on the
 * double-Enter exit line. Blank lines in the middle remain ordinary content. */
export function multilineExitTrim(
  text: string,
  caret: number,
  kind: "calc" | "fence" | "math" | "properties",
  format: Format = "md",
): string | null {
  const c = Math.max(0, Math.min(caret, text.length));
  const lineStart = text.lastIndexOf("\n", c - 1) + 1;
  let lineEnd = text.indexOf("\n", c);
  if (lineEnd === -1) lineEnd = text.length;
  if (text.slice(lineStart, lineEnd).trim() !== "" || lineStart === 0) return null;

  if (kind === "calc" || kind === "properties") {
    if (text.slice(lineEnd).trim() !== "") return null;
    return text.slice(0, lineStart - 1);
  }
  if (kind === "fence") return fenceExitTrim(text, lineStart, lineEnd, format);

  const after = text.slice(lineEnd + 1);
  const nextNewline = after.indexOf("\n");
  const nextLine = nextNewline === -1 ? after : after.slice(0, nextNewline);
  if (!displayMathOpenAfter(text.slice(0, lineStart), format) || !closesDisplayMath(nextLine)) return null;
  const afterClosing = nextNewline === -1 ? "" : after.slice(nextNewline + 1);
  if (afterClosing.trim() !== "") return null;
  return text.slice(0, lineStart - 1) + text.slice(lineEnd);
}

/** The two on-disk block formats. Markdown keeps built-in props as trailing
 *  `key:: value` lines; org keeps them inside a `:PROPERTIES:`/`:END:` drawer. */
export type PropFormat = "md" | "org";

/** Split a block's raw into the editor-visible text and the hidden property
 *  lines (the kernel's `editor_text`). Which lines are metadata is the parser's
 *  answer: a `key:: value` line inside a code fence stays visible content. Org
 *  drops a `:PROPERTIES:` wrapper whose every entry is hidden. Before the parser
 *  is ready nothing is hidden. Inverse of {@link joinProps}. */
export function splitProps(
  raw: string,
  hide: HidePolicy,
  format: PropFormat = "md"
): { visible: string; hidden: string } {
  if (!parserReady()) return { visible: raw, hidden: "" };
  return editorText(raw, format, policyText(hide));
}

/** Map a UTF-16 offset in raw block text into the textarea's visible buffer,
 *  using the same split as {@link splitProps}. When the raw offset falls inside
 *  a hidden property line, it maps to the edit point where that removed line
 *  would have appeared. */
export function rawOffsetToVisibleOffset(
  raw: string,
  rawOffset: number,
  hide: HidePolicy,
  format: PropFormat = "md"
): number {
  if (!parserReady()) return Math.max(0, Math.min(rawOffset, raw.length));
  return editorOffsetOfRaw(raw, format, policyText(hide), rawOffset);
}

/** The raw UTF-8 byte offset of a textarea offset (the kernel verbs' unit). */
export function visibleOffsetToRawByte(raw: string, visibleOffset: number, hide: HidePolicy, format: PropFormat = "md"): number {
  return rawByteOfEditor(raw, format, policyText(hide), visibleOffset);
}

/** Reattach hidden property lines to the visible text — the inverse of
 *  {@link splitProps}. Markdown appends them below the body; Org folds them back
 *  into a `:PROPERTIES:` drawer at OG's canonical spot. A metadata-only block
 *  (empty visible) is just its hidden lines. */
export function joinProps(visible: string, hidden: string, format: PropFormat = "md"): string {
  // The kernel places the rows: at the end of the block's own property group,
  // else before a final quote (its lazy continuation would swallow them), else
  // below the body (REG-HIDDEN-PROPS-STAY-IN-THE-GROUP, GH #684, #721 F3).
  return editorTextJoin(visible, hidden, format);
}

/** First value for `key` (case-insensitive) in a property block, or null. */
export function readPropertyValue(block: string | null, key: string): string | null {
  if (!block) return null;
  const property = blockRegions(block).properties.find((p) => p.primary && p.key.toLowerCase() === key.toLowerCase());
  if (property) return property.value;
  return null;
}

/** Add / replace / remove a `key:: value` line. A null or empty value removes
 *  the key. Replace the first matching line in place and preserve every
 *  unrelated line and blank separator byte-for-byte; page-property grouping
 *  and order are user data, not disposable formatting. Duplicate matching
 *  keys retain the prior single-value behavior and collapse to the first slot.
 *  Returns null when no nonblank content remains. */
export function upsertPropertyLine(
  block: string | null,
  key: string,
  value: string | null
): string | null {
  const v = value == null ? null : value.trim();
  const lines = block == null || block === "" ? [] : block.split("\n");
  const out: string[] = [];
  let matched = false;
  const raw = block ?? "";
  const decoder = new TextDecoder();
  const bytes = new TextEncoder().encode(raw);
  const lineIndices = new Map<number, number>();
  let byteStart = 0;
  const encoder = new TextEncoder();
  lines.forEach((line, i) => { lineIndices.set(byteStart, i); byteStart += encoder.encode(line).length + 1; });
  const regions = blockRegions(raw);
  if (regions.quarantined) throw new Error("Structural edit refused: block parsing is quarantined");
  const accepted = new Map(regions.properties.filter((p) => p.primary)
    .map((p) => [lineIndices.get(p.line[0]), p]));
  for (const [index, line] of lines.entries()) {
    const p = accepted.get(index);
    if (p && p.key.toLowerCase() === key.toLowerCase()) {
      if (!matched && v) out.push(`${line.slice(0, decoder.decode(bytes.subarray(p.line[0], p.key_range[1])).length)}:: ${v}${line.endsWith("\r") ? "\r" : ""}`);
      matched = true;
      continue;
    }
    out.push(line);
  }
  // Actual Logseq `frontend.util.page-property/insert-property` prepends a new
  // page property and replaces an existing one in place.  Keep that ordering
  // contract instead of inventing a Tine-local append rule.
  if (!matched && v) out.unshift(`${key}:: ${v}`);
  return out.some((line) => line.trim() !== "") ? out.join("\n") : null;
}

/** One page property line: `key` (Org keys lowercased, as OG/mldoc store
 *  them), trimmed `value`, and `line`, its 0-based line index in the text. */
export interface PagePropertyEntry {
  key: string;
  value: string;
  line: number;
}

let lastPageEntries: { raw: string; entries: PagePropertyEntry[] } | undefined;

/** Accepted whole-preamble properties, in file order. Markdown's are the parser-accepted page header
 * (`pageHeader`); Org uses parser-owned directives and drawers, excluding literal src/example
 * regions. Both cost O(text) cold and O(properties) for a repeated read of one retained source. */
export function pagePropertyEntries(text: string | null | undefined, format: PropFormat): PagePropertyEntry[] {
  if (!text) return [];
  if (format === "md") return pageHeader(text).entries.map((e) => ({ ...e }));
  const source = text;
  if (lastPageEntries?.raw === source) return lastPageEntries.entries.map((e) => ({ ...e }));
  const regions = JSON.parse(page_regions_json(source, true)) as RegionProperty[];
  const bytes = new TextEncoder().encode(source);
  const decoder = new TextDecoder();
  let byte = 0, line = 0;
  const entries = regions.map((p) => {
    line += decoder.decode(bytes.subarray(byte, p.line[0])).split("\n").length - 1;
    byte = p.line[0];
    return { key: p.key.toLowerCase(), value: p.value, line };
  });
  lastPageEntries = { raw: source, entries };
  return entries.map((e) => ({ ...e }));
}

/** The lowercased keys of a Markdown page header text, in file order (the parser's header, see
 *  {@link pagePropertyEntries}). */
export function pageHeaderKeys(raw: string): string[] {
  return pagePropertyEntries(raw, "md").map((e) => e.key.toLowerCase());
}

/** Set (or, for a null/blank value, remove) page property `key` across `parts`
 *  — the texts a page's properties are read from, in file order, each parsed on
 *  its own by {@link pagePropertyEntries}. The first case-insensitive match is
 *  replaced in place (Markdown keeps the file's key spelling; Org writes the
 *  lowercased key), every other match is removed, and a new key is prepended
 *  to `parts[0]` (Logseq's insert-property order). Unrelated lines keep their
 *  bytes. A removal that empties an Org drawer drops the drawer; one that
 *  removes a Markdown header's first line also drops the blank separators that
 *  would otherwise detach the rest of the header. Returns the new texts, same
 *  length as `parts`. Cost O(total text). */
export function pagePartsWithProperty(parts: string[], format: PropFormat, key: string, value: string | null): string[] {
  const v = value?.trim() || null;
  const lower = key.toLowerCase();
  const hits = parts.flatMap((text, part) =>
    pagePropertyEntries(text, format).filter((e) => e.key.toLowerCase() === lower).map((e) => ({ part, line: e.line, key: e.key })));
  const lines = parts.map((text) => (text === "" ? [] : text.split("\n")));
  const dropped = parts.map(() => new Set<number>());
  hits.forEach(({ part, line, key: spelling }, i) => {
    if (i > 0 || !v) {
      dropped[part].add(line);
      return;
    }
    const old = lines[part][line];
    const indent = old.slice(0, old.length - old.trimStart().length);
    lines[part][line] = format === "org"
      ? `${indent}${old.trimStart().startsWith("#+") ? `#+${lower}: ` : `:${lower}: `}${v}`
      : `${spelling}:: ${v}`;
  });
  if (!hits.length && v) lines[0].unshift(format === "org" ? `#+${lower}: ${v}` : `${key}:: ${v}`);
  return lines.map((all, part) => {
    if (!dropped[part].size) return all.join("\n");
    const entryLines = new Set(pagePropertyEntries(parts[part], format).map((e) => e.line));
    let kept = all.map((line, i) => ({ line, i })).filter(({ i }) => !dropped[part].has(i));
    if (format === "org") {
      // A drawer is "emptied" only when one of OUR removals lay inside it.
      const emptied = (open?: { line: string; i: number }, end?: { line: string; i: number }) =>
        !!open && !!end && /^:PROPERTIES:$/i.test(open.line.trim()) && /^:END:$/i.test(end.line.trim())
        && [...dropped[part]].some((d) => open.i < d && d < end.i);
      kept = kept.filter((_, k) => !emptied(kept[k], kept[k + 1]) && !emptied(kept[k - 1], kept[k]));
    } else if (dropped[part].has(0)) {
      const first = kept.findIndex(({ line }) => line.trim() !== "");
      if (first > 0 && entryLines.has(kept[first].i)) kept = kept.slice(first);
    }
    return kept.map(({ line }) => line).join("\n");
  });
}

/** The page-level properties we surface in the page-properties panel, with a
 *  one-line description and whether the value is a boolean toggle. */
export interface PagePropSpec {
  key: string;
  label: string;
  hint: string;
  kind: "text" | "bool" | "list";
}
export const PAGE_PROP_SPECS: PagePropSpec[] = [
  { key: "alias", get label() { return render(UI_MESSAGES["editor-aliases"]); }, get hint() { return render(UI_MESSAGES["editor-other-names-this-page-answers-to-in-links-comma-separated"]); }, kind: "list" },
  { key: "tags", get label() { return render(UI_MESSAGES["editor-tags"]); }, get hint() { return render(UI_MESSAGES["editor-page-level-tags-comma-separated"]); }, kind: "list" },
  { key: "title", get label() { return render(UI_MESSAGES["editor-display-title"]); }, get hint() { return render(UI_MESSAGES["editor-override-the-shown-title-the-file-name-stays-the-same"]); }, kind: "text" },
  { key: "icon", get label() { return render(UI_MESSAGES["editor-icon"]); }, get hint() { return render(UI_MESSAGES["editor-an-emoji-character-shown-with-the-title"]); }, kind: "text" },
  { key: "public", get label() { return render(UI_MESSAGES["editor-public"]); }, get hint() { return render(UI_MESSAGES["editor-include-this-page-when-exporting-publishing-public-pages"]); }, kind: "bool" },
];
