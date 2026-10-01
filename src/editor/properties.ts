// Pure helpers for reading/editing `key:: value` property lines — a block's
// continuation lines or a page's pre-block. No store/DOM, so unit-testable.

import { transitionFence, displayMathOpenAfter, closesDisplayMath, type FenceState } from "./fences";
import { blockRegions, editBlock, parserReady } from "../render/parse";

import { property_line_json, page_regions_json } from "../render/wasm/lsdoc_wasm.js";
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

const PAGE_HEADER_KEY = /^[\p{L}\p{M}\p{N}_./-]+$/u;

/** Parse one canonical Markdown page-header property line. This grammar is
 * intentionally separate from ordinary block properties: page headers may use
 * Unicode/plugin keys, but must start at column zero and cannot absorb prose,
 * headings or fences into metadata. The value is returned byte-for-byte after
 * the exact `::` delimiter (including its optional conventional space). */
export function parsePageHeaderPropertyLine(line: string): { key: string; value: string } | null {
  const delimiter = line.indexOf("::");
  if (delimiter <= 0) return null;
  const key = line.slice(0, delimiter);
  if (key.startsWith("#") || !PAGE_HEADER_KEY.test(key)) return null;
  return { key, value: line.slice(delimiter + 2) };
}

/** A complete canonical page header: one or more property lines, with blank
 * separators permitted only between properties (never at either edge). */
export function isPageHeaderPropertiesOnly(raw: string): boolean {
  if (!raw || raw.startsWith("\n") || raw.endsWith("\n")) return false;
  const lines = raw.split("\n");
  let sawProperty = false;
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    if (line === "") {
      if (!sawProperty || i === lines.length - 1) return false;
      continue;
    }
    if (!parsePageHeaderPropertyLine(line)) return false;
    sawProperty = true;
  }
  return sawProperty;
}

/** Keep the canonical page-header predicate shared by display and edit paths so
 * a candidate cannot be hidden in one place but edited as ordinary text in
 * another. */
export function isPropertiesOnly(raw: string): boolean {
  return isPageHeaderPropertiesOnly(raw);
}

/** Whether the textarea caret is on a complete `key:: value` line. This is
 * deliberately line-local: an empty line after a run of page properties is the
 * double-Enter exit sentinel, not another property line. */
export function caretOnPropertyLine(raw: string, caret: number): boolean {
  const c = Math.max(0, Math.min(caret, raw.length));
  const lineStart = raw.lastIndexOf("\n", c - 1) + 1;
  const nextNewline = raw.indexOf("\n", c);
  const lineEnd = nextNewline === -1 ? raw.length : nextNewline;
  return parsePageHeaderPropertyLine(raw.slice(lineStart, lineEnd)) !== null;
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
  if (!parsePageHeaderPropertyLine(raw.split("\n", 1)[0])) {
    const content = raw.replace(/^\n+|\n+$/g, "") || null;
    return { properties: null, content, remainder: raw };
  }

  // Extend through property lines and blank runs only when another property
  // follows. A blank before prose belongs to the exact suffix, not the header.
  let pos = 0;
  let headerEnd = 0;
  while (pos < raw.length) {
    const nl = raw.indexOf("\n", pos);
    const end = nl === -1 ? raw.length : nl;
    const line = raw.slice(pos, end);
    if (!parsePageHeaderPropertyLine(line)) break;
    headerEnd = end;
    if (nl === -1) break;
    let next = nl + 1;
    while (next < raw.length) {
      const nextNl = raw.indexOf("\n", next);
      const nextEnd = nextNl === -1 ? raw.length : nextNl;
      if (raw.slice(next, nextEnd) !== "") break;
      next = nextNl === -1 ? raw.length : nextNl + 1;
    }
    const nextNl = raw.indexOf("\n", next);
    const nextEnd = nextNl === -1 ? raw.length : nextNl;
    if (next >= raw.length || !parsePageHeaderPropertyLine(raw.slice(next, nextEnd))) break;
    pos = next;
  }
  const properties = raw.slice(0, headerEnd);
  const remainder = raw.slice(headerEnd) || null;
  const content = remainder?.replace(/^\n+|\n+$/g, "") || null;
  return { properties, content, remainder };
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

  const after = text.slice(lineEnd + 1);
  const nextNewline = after.indexOf("\n");
  const nextLine = nextNewline === -1 ? after : after.slice(0, nextNewline);
  const before = text.slice(0, lineStart);
  if (kind === "math") {
    if (!displayMathOpenAfter(before) || !closesDisplayMath(nextLine)) return null;
  } else {
    let fence: FenceState | null = null;
    for (const line of before.split("\n")) {
      fence = transitionFence(fence, line).next;
    }
    if (!fence || !transitionFence(fence, nextLine).closes) return null;
  }
  const afterClosing = nextNewline === -1 ? "" : after.slice(nextNewline + 1);
  if (afterClosing.trim() !== "") return null;
  return text.slice(0, lineStart - 1) + text.slice(lineEnd);
}

/** Whether a textarea caret offset is inside a fenced code region. The fence
 *  delimiter lines themselves are outside; the content lines between them are
 *  inside, including an unterminated fence while the user is editing. */
export function caretInFence(raw: string, offset: number): boolean {
  const target = Math.max(0, Math.min(offset, raw.length));
  let fence: FenceState | null = null;
  let pos = 0;
  while (pos <= raw.length) {
    const nl = raw.indexOf("\n", pos);
    const end = nl === -1 ? raw.length : nl;
    const line = raw.slice(pos, end);
    const t = transitionFence(fence, line);
    if (target <= end) return fence !== null && !t.closes;
    fence = t.next;
    if (nl === -1) break;
    pos = end + 1;
  }
  return fence !== null;
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
  const accepted = new Map(blockRegions(raw).properties.filter((p) => p.primary)
    .map((p) => [decoder.decode(bytes.subarray(0, p.line[0])).split("\n").length - 1, p]));
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

const pageRegionsCache = new Map<string, RegionProperty[]>();

/** Accepted whole-preamble properties, in file order. Markdown's canonical
 * column-zero header policy is retained; Org uses parser-owned directives and
 * drawers, excluding literal src/example regions. Cost O(preamble bytes) cold,
 * O(property count) warm; bounded to 64 retained preambles. */
export function pagePropertyEntries(text: string | null | undefined, format: PropFormat): PagePropertyEntry[] {
  if (!text) return [];
  if (format === "md") {
    return (splitPagePreamble(text).properties?.split("\n") ?? []).flatMap((line, i) => {
      const p = parsePageHeaderPropertyLine(line);
      return p ? [{ key: p.key, value: p.value.trim(), line: i }] : [];
    });
  }
  const source = text;
  let regions = pageRegionsCache.get(source);
  if (!regions) {
    regions = JSON.parse(page_regions_json(source, true)) as RegionProperty[];
    if (pageRegionsCache.size >= 64) pageRegionsCache.delete(pageRegionsCache.keys().next().value!);
    pageRegionsCache.set(source, regions);
  }
  const bytes = new TextEncoder().encode(source);
  const decoder = new TextDecoder();
  return regions.map((p) => ({
    key: format === "org" ? p.key.toLowerCase() : p.key,
    value: p.value,
    line: decoder.decode(bytes.subarray(0, p.line[0])).split("\n").length - 1,
  }));
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
    pagePropertyEntries(text, format).filter((e) => e.key.toLowerCase() === lower).map((e) => ({ part, line: e.line })));
  const lines = parts.map((text) => (text === "" ? [] : text.split("\n")));
  const dropped = parts.map(() => new Set<number>());
  hits.forEach(({ part, line }, i) => {
    if (i > 0 || !v) {
      dropped[part].add(line);
      return;
    }
    const old = lines[part][line];
    const indent = old.slice(0, old.length - old.trimStart().length);
    lines[part][line] = format === "org"
      ? `${indent}${old.trimStart().startsWith("#+") ? `#+${lower}: ` : `:${lower}: `}${v}`
      : `${parsePageHeaderPropertyLine(old)!.key}:: ${v}`;
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
