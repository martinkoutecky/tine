// Pure helpers for reading/editing `key:: value` property lines — a block's
// continuation lines or a page's pre-block. No store/DOM, so unit-testable.

import { displayMathOpenAfter, closesDisplayMath, fenceExitTrim } from "./fences";
import { blockRegions, editBlock, parserReady } from "../render/parse";
import { utf8ToUtf16Cursor } from "../render/utf16Cursor";

import { property_line_json, page_regions_json, page_header_json } from "../render/wasm/lsdoc_wasm.js";
import type { RegionProperty } from "../render/parse";

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
  const parsed = JSON.parse(page_header_json(raw)) as { end: number; entries: PageHeader["entries"] };
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

/** Keep the canonical page-header predicate shared by display and edit paths so
 * a candidate cannot be hidden in one place but edited as ordinary text in
 * another. */
export function isPropertiesOnly(raw: string): boolean {
  return isPageHeaderPropertiesOnly(raw);
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

// Built-in properties hidden from the editor by default (like OG): `id::`,
// `collapsed::`, and `logseq.order-list-type::` (the numbered-list marker) are
// kept in the file for persistence but never shown in the edit textarea.
// Annotation (PDF highlight) blocks instead hide ALL properties and edit only
// their text.
const BUILTIN_HIDDEN = new Set(["id", "collapsed", "logseq.order-list-type"]);
/** Hide just the built-in `id::`/`collapsed::` properties (normal blocks). */
export const isBuiltinHidden = (key: string): boolean => BUILTIN_HIDDEN.has(key);
/** Hide metadata that should not surface while editing through a sheet cell. */
export const isSheetCellHidden = (key: string): boolean =>
  isBuiltinHidden(key) || key.toLowerCase().startsWith("tine.");
/** Hide every property (annotation blocks edit only their text). */
export const hideAll = (_key: string): boolean => true;

/** For a multi-line editor that normally keeps Enter inside it, return the text
 * with its trailing sentinel blank line removed when the caret is on the
 * double-Enter exit line. Blank lines in the middle remain ordinary content. */
export function multilineExitTrim(
  text: string,
  caret: number,
  kind: "calc" | "fence" | "math" | "properties"
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
  if (kind === "fence") return fenceExitTrim(text, lineStart, lineEnd);

  const after = text.slice(lineEnd + 1);
  const nextNewline = after.indexOf("\n");
  const nextLine = nextNewline === -1 ? after : after.slice(0, nextNewline);
  if (!displayMathOpenAfter(text.slice(0, lineStart)) || !closesDisplayMath(nextLine)) return null;
  const afterClosing = nextNewline === -1 ? "" : after.slice(nextNewline + 1);
  if (afterClosing.trim() !== "") return null;
  return text.slice(0, lineStart - 1) + text.slice(lineEnd);
}

/** The two on-disk block formats. Markdown keeps built-in props as trailing
 *  `key:: value` lines; org keeps them inside a `:PROPERTIES:`/`:END:` drawer. */
export type PropFormat = "md" | "org";

type LineClass = "v" | "h" | "d"; // visible | hidden-payload | dropped(org wrapper)

/** Present only primary properties accepted by the block-region door. Lines
 * are transport coordinates here, never evidence that text is metadata. */
function classifyLines(
  lines: string[],
  isHidden: (key: string) => boolean,
  format: PropFormat
): LineClass[] {
  const cls: LineClass[] = new Array(lines.length).fill("v");
  if (!parserReady()) return cls;
  const raw = lines.join("\n");
  const regions = blockRegions(raw, format);
  if (regions.quarantined) return cls;
  const starts = [0];
  const encoder = new TextEncoder();
  for (let i = 0; i < lines.length - 1; i++) starts.push(starts[i] + encoder.encode(lines[i]).length + 1);
  const lineAt = (byte: number) => {
    let lo = 0, hi = starts.length;
    while (lo + 1 < hi) {
      const mid = (lo + hi) >>> 1;
      if (starts[mid] <= byte) lo = mid;
      else hi = mid;
    }
    return lo;
  };
  const own = regions.properties.filter((p) => p.primary);
  for (const p of own) if (isHidden(p.key.toLowerCase())) cls[lineAt(p.line[0])] = "h";
  if (format === "org") {
    for (const [index, range] of regions.property_regions.entries()) {
      const entries = own.filter((p) => p.region === index);
      if (!entries.length || !entries.every((p) => isHidden(p.key.toLowerCase()))) continue;
      cls[lineAt(range[0])] = "d";
      cls[lineAt(Math.max(range[0], range[1] - 1))] = "d";
    }
  }
  return cls;
}

/** Split a block's raw into the editor-visible text and the hidden property
 *  lines. Fence-aware: a `key:: value` line inside a ```/~~~ code fence stays
 *  visible content — it must NOT be pulled out as metadata and reattached
 *  outside the fence (which would corrupt the code on focus+blur). `isHidden`
 *  selects which property keys are hidden (e.g. {@link isBuiltinHidden} or
 *  {@link hideAll}). `format` (default `"md"`) enables org `:PROPERTIES:` drawer
 *  handling. Inverse of {@link joinProps}. */
export function splitProps(
  raw: string,
  isHidden: (key: string) => boolean,
  format: PropFormat = "md"
): { visible: string; hidden: string } {
  const { visible, hidden } = splitPropsInternal(raw, isHidden, format);
  return { visible, hidden };
}

function splitPropsInternal(
  raw: string,
  isHidden: (key: string) => boolean,
  format: PropFormat,
  rawOffset?: number
): { visible: string; hidden: string; visibleOffset?: number } {
  const lines = raw.split("\n");
  const cls = classifyLines(lines, isHidden, format);
  const vis: string[] = [];
  const hid: string[] = [];
  const target = rawOffset == null ? null : Math.max(0, Math.min(rawOffset, raw.length));
  let visibleLen = 0;
  let visibleOffset: number | null = null;
  let rawPos = 0;
  for (let i = 0; i < lines.length; i++) {
    const l = lines[i];
    const rawStart = rawPos;
    const rawEnd = rawStart + l.length;
    if (cls[i] === "v") {
      const lineVisibleStart = visibleLen + (vis.length > 0 ? 1 : 0);
      const lineVisibleEnd = lineVisibleStart + l.length;
      if (target != null && visibleOffset == null && target >= rawStart && target <= rawEnd) {
        visibleOffset = lineVisibleStart + (target - rawStart);
      }
      vis.push(l);
      visibleLen = lineVisibleEnd;
    } else {
      // "h" (hidden payload) or "d" (dropped org wrapper): not shown. A caret
      // inside it maps to where the removed text would have appeared.
      if (target != null && visibleOffset == null && target >= rawStart && target <= rawEnd) {
        visibleOffset = visibleLen;
      }
      if (cls[i] === "h") hid.push(l);
    }
    rawPos = rawEnd + 1;
  }
  return {
    visible: vis.join("\n"),
    hidden: hid.join("\n"),
    visibleOffset: target == null ? undefined : (visibleOffset ?? visibleLen),
  };
}

/** Map a UTF-16 offset in raw block text into the textarea's visible buffer,
 *  using the same fence-aware hidden-property split as {@link splitProps}. When
 *  the raw offset falls inside a hidden property line, it maps to the edit point
 *  where that removed line would have appeared. */
export function rawOffsetToVisibleOffset(
  raw: string,
  rawOffset: number,
  isHidden: (key: string) => boolean,
  format: PropFormat = "md"
): number {
  return splitPropsInternal(raw, isHidden, format, rawOffset).visibleOffset ?? 0;
}

/** Reattach hidden property lines to the visible text — the inverse of
 *  {@link splitProps}. Markdown appends them below the body (that's where its
 *  `id::`/`collapsed::` live). Org folds them back into a `:PROPERTIES:` drawer
 *  at OG's canonical spot (into an existing drawer if the visible text still has
 *  one, else native placement after the title and accepted planning — matching
 *  {@link rawWithBlockId}). A metadata-only block (empty
 *  visible) is just its hidden lines — no spurious leading newline. */
export function joinProps(visible: string, hidden: string, format: PropFormat = "md"): string {
  if (!hidden) return visible;
  if (format !== "org") return visible ? `${visible}\n${hidden}` : hidden;
  return editBlock(visible, format, { kind: "reattach_properties", hidden });
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
  { key: "alias", label: "Aliases", hint: "Other names this page answers to in [[links]] (comma-separated)", kind: "list" },
  { key: "tags", label: "Tags", hint: "Page-level tags (comma-separated)", kind: "list" },
  { key: "title", label: "Display title", hint: "Override the shown title (the file name stays the same)", kind: "text" },
  { key: "icon", label: "Icon", hint: "An emoji/character shown with the title", kind: "text" },
  { key: "public", label: "Public", hint: "Include this page when exporting/publishing public pages", kind: "bool" },
];
