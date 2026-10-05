// Helpers to derive a block's *rendered* view from its raw text. Raw stays
// authoritative (round-trip); these are computed projections.

import type { Format } from "./ast";
import { MARKERS, headerTokens } from "../markers";
import { acceptedPropertyLine, pagePropertyEntries } from "../editor/properties";
import { blockRegions, parserReady } from "./parse";
import { utf8ToUtf16Cursor } from "./utf16Cursor";
import {
  split_linkable_property as splitLinkableProperty,
  is_render_hidden_prop as isRenderHiddenPropNative,
} from "./wasm/lsdoc_wasm.js";
import { propertyKeyNorm } from "../propertyKey";
export { splitLinkableProperty };

export { MARKERS };

export { propertyKeyNorm };

// Property keys NOT shown as rendered chips (id/uuid/collapsed + Logseq internals
// + display-only keys, `tine.*`, `logseq.table.*`, and the user's
// `:block-hidden-properties`). ONE answerer for Block.tsx's live chip filter,
// body.tsx's renderProps and the static export: crates/tine-core/src/render_facets.rs,
// reached through the WASM bridge (I-12, no TS twin of the key list).
//
// Deliberately SEPARATE (different concepts — do not merge): editor/properties.ts
// BUILTIN_HIDDEN (hide from the edit textarea), query.rs INTERNAL_PROPS (don't
// offer as a query filter), components/Page.tsx PAGE_PROPS_HIDDEN (page-prop area).

/** Whether a property key is hidden from the rendered chips: a built-in internal
 *  key (case-insensitive) OR one the user listed in `:block-hidden-properties`. */
export function isRenderHiddenProp(key: string, userHidden: readonly string[] = []): boolean {
  return isRenderHiddenPropNative(key, userHidden as string[]);
}

export function isPropertyLine(line: string): boolean {
  return acceptedPropertyLine(line) !== null;
}

/** A page-property text's properties as `[key, value]` pairs, in file order,
 *  duplicates kept. The grammar is editor/properties.ts `pagePropertyEntries`
 *  (fence-aware Markdown header; Org `#+KEY:` directives and `:PROPERTIES:`
 *  drawer lines, keys lowercased), the one answerer every page-property reader
 *  derives from. Cost O(text). */
export function pageProperties(
  preBlock: string | null | undefined,
  format: Format = "md"
): [string, string][] {
  return pagePropertyEntries(preBlock, format).map((entry) => [entry.key, entry.value]);
}

/** The alias names declared in already-read `[key, value]` page properties
 *  (`alias::` in markdown, `#+ALIAS:` / `:alias:` in org), comma-separated.
 *  Empty if none. Read the properties through the one answerer
 *  (`pageHeaderProperties` for a loaded page). */
export function aliasNamesOf(properties: [string, string][]): string[] {
  const out: string[] = [];
  for (const [k, v] of properties) {
    const key = propertyKeyNorm(k);
    if (key !== "alias" && key !== "aliases") continue;
    if (isQuotedPagePropertyValue(v)) continue;
    out.push(...splitLinkableProperty(v)
      .map(normalizeImplicitPageName)
      .filter(Boolean));
  }
  return out;
}

/** Built-in page-property values that Logseq treats as page references even
 * without explicit `[[...]]` syntax. Custom properties stay ordinary text. */
export function isImplicitPageRefProperty(key: string): boolean {
  const normalized = propertyKeyNorm(key);
  return normalized === "alias" || normalized === "aliases" || normalized === "tags";
}

/** A whole quoted property value is literal text, including its commas. */
export function isQuotedPagePropertyValue(value: string): boolean {
  const trimmed = value.trim();
  return trimmed.length >= 2 && trimmed.startsWith('"') && trimmed.endsWith('"');
}

/** Normalize one implicit page value for alias resolution / display. */
export function normalizeImplicitPageName(value: string): string {
  let trimmed = value.trim();
  if (trimmed.startsWith("#[[") && trimmed.endsWith("]]")) trimmed = trimmed.slice(3, -2);
  else if (trimmed.startsWith("[[") && trimmed.endsWith("]]")) trimmed = trimmed.slice(2, -2);
  else if (trimmed.startsWith("#")) trimmed = trimmed.slice(1);
  return trimmed.trim();
}

/** A block's *visible body* lines: the readable text the reader sees, with the
 *  marker / priority / heading prefix stripped from the first line and the
 *  property / planning / drawer (LOGBOOK, PROPERTIES, CLOCK) lines removed. EVERY
 *  decision is the parser's (I-12): header facts from `headerTokens`, metadata
 *  line extents from lsdoc's block regions (`blockRegions`); code containers are
 *  content because lsdoc forms no metadata inside them. Nothing here recognizes a
 *  property, planning line or drawer by its text. Before the parser is ready no
 *  fact is known and the lines come back unchanged.
 *
 *  This is ONLY the body text — for short labels (breadcrumbs, search, sidebar
 *  titles) and the reference-panel inline render. The block-header FACTS
 *  (marker / priority / heading / scheduled / deadline / properties) are NOT
 *  derived here; they come from the one lsdoc parse via `render/facets` `facetsOf`.
 *  So there's no second facet recognizer — just one body-text extractor. */
export function visibleBody(raw: string): string[] {
  // Recognize against the whole raw, as the source block does. Looking only at
  // line one mistakes `TODO\nbody` for a task and misses leading blank lines.
  const { marker, priority } = headerTokens(raw);
  const from = marker ? marker.end + (raw[marker.end] === " " ? 1 : 0) : 0;
  // The accepted priority token (only whitespace can precede it) is facet, not body text.
  const cut = priority && priority.start >= from
    ? { start: priority.start, end: priority.end + (/\s/.test(raw[priority.end] ?? "") ? 1 : 0) }
    : null;
  const meta = metadataLineRanges(raw, from);
  const lines: string[] = [];
  let pos = 0;
  for (const line of raw.split("\n")) {
    const start = pos;
    const end = start + line.length;
    pos = end + 1;
    if (end < from) continue; // before the header token (a skipped leading line)
    if (start >= from && meta.some(([a, b]) => a < pos && start < b)) continue;
    let text = line;
    const head = Math.max(from - start, 0);
    if (cut && cut.start >= start && cut.start <= end) {
      text = line.slice(head, cut.start - start) + line.slice(Math.min(cut.end, end) - start);
    } else if (head > 0) {
      text = line.slice(head);
    }
    lines.push(text);
  }
  if (lines.length === 0) lines.push("");
  // Strip the heading prefix from the first line (`TODO ## x` keeps its heading after the marker).
  const heading = /^(#{1,6}) /.exec(lines[0]);
  if (heading) lines[0] = lines[0].slice(heading[1].length + 1);
  while (lines.length > 1 && lines[0].trim() === "") lines.shift();
  return lines;
}

/** Whether `raw` can hold anything the block regions would remove: more than one
 *  line or a colon (property separator, planning keyword). Admission only (skips the parse of the
 *  common one-line label); lsdoc decides the rest. */
function needsRegions(raw: string): boolean {
  return raw.includes("\n") || raw.includes(":");
}

/** UTF-16 `[start, end)` extents (newline included) of every metadata line lsdoc
 *  accepted in `raw` at or after UTF-16 offset `from`: properties, planning lines
 *  and drawers. Empty before the parser is ready or for a quarantined block. */
function metadataLineRanges(raw: string, from: number): [number, number][] {
  if (!parserReady() || !needsRegions(raw)) return [];
  const regions = blockRegions(raw, "md");
  if (regions.quarantined) return [];
  const bytes = [
    ...regions.property_regions,
    ...regions.planning.map((p) => p.line),
    ...regions.drawers.map((d) => d.range),
  ].sort((x, y) => x[0] - y[0]);
  const at = utf8ToUtf16Cursor(raw);
  const out: [number, number][] = [];
  for (const [a, b] of bytes) {
    const range: [number, number] = [at(a), at(b)];
    if (range[1] > from) out.push(range);
  }
  return out;
}
