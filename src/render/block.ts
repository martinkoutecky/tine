// Helpers to derive a block's *rendered* view from its raw text. Raw stays
// authoritative (round-trip); these are computed projections.

import type { Format } from "./ast";
import { MARKERS, matchLeadingMarker } from "../markers";
import { pagePropertyEntries } from "../editor/properties";

export { MARKERS };

export function propertyKeyNorm(key: string): string {
  return key.trim().toLowerCase().replace(/[ _]/g, "-");
}

// Property keys NOT shown as rendered chips (id/uuid/collapsed + Logseq internals
// + display-only keys). Single source for the two render paths — Block.tsx's live
// chip filter and body.tsx's renderProps — which had drifted ~15 keys apart, and
// only Block.tsx honored the user's `:block-hidden-properties`. Lowercased; compare
// via isRenderHiddenProp so the match is case-insensitive (OG treats keys so).
//
// Deliberately SEPARATE (different concepts — do not merge): editor/properties.ts
// BUILTIN_HIDDEN (hide from the edit textarea), query.rs INTERNAL_PROPS (don't
// offer as a query filter), components/Page.tsx PAGE_PROPS_HIDDEN (page-prop area).
export const RENDER_HIDDEN_PROPS: ReadonlySet<string> = new Set([
  "id", "collapsed", "hl-page", "hl-color", "hl-type", "ls-type",
  "background-color", "logseq.order-list-type",
  "heading", "title", "filters", "created-at", "updated-at", "last-modified-at",
  "query-table", "query-properties", "query-sort-by", "query-sort-desc", "logseq.tldraw.shape",
].map(propertyKeyNorm));

/** Whether a property key is hidden from the rendered chips: a built-in internal
 *  key (case-insensitive) OR one the user listed in `:block-hidden-properties`. */
export function isRenderHiddenProp(key: string, userHidden: readonly string[] = []): boolean {
  const normalized = propertyKeyNorm(key);
  return normalized.startsWith("tine.")
    // Table v2 reads this configuration from the block rather than presenting it
    // as content. OG likewise resolves `logseq.table.*` view props from the block
    // property map (og/deps/shui/src/logseq/shui/table/v2.cljs:37-50).
    || normalized.startsWith("logseq.table.")
    || RENDER_HIDDEN_PROPS.has(normalized)
    || userHidden.some((k) => propertyKeyNorm(k) === normalized);
}

const PROP_RE = /^[A-Za-z0-9_./-]+::\s?.*$/;

export function isPropertyLine(line: string): boolean {
  const idx = line.indexOf("::");
  if (idx <= 0) return false;
  const key = line.slice(0, idx).trim();
  return key.length > 0 && /^[A-Za-z0-9_./-]+$/.test(key) && PROP_RE.test(line);
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
    out.push(...v
      .split(/[,，]/)
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

const PLANNING_LINE = /^\s*(SCHEDULED|DEADLINE):\s*<[^>]+>\s*$/;

/** A block's *visible body* lines: the readable text the reader sees, with the
 *  marker / priority / heading prefix stripped from the first line and the
 *  property / SCHEDULED / DEADLINE / drawer / CLOCK lines removed. Fence-aware (a
 *  `key::` or `SCHEDULED:` inside a code fence stays as content).
 *
 *  This is ONLY the body text — for short labels (breadcrumbs, search, sidebar
 *  titles) and the reference-panel inline render. The block-header FACTS
 *  (marker / priority / heading / scheduled / deadline / properties) are NOT
 *  derived here; they come from the one lsdoc parse via `render/facets` `facetsOf`.
 *  So there's no second facet recognizer — just one body-text extractor. */
export function visibleBody(raw: string): string[] {
  // Recognize against the whole raw, as the source block does. Looking only at
  // line one mistakes `TODO\nbody` for a task and misses leading blank lines.
  const markerMatch = matchLeadingMarker(raw);
  const body = markerMatch ? raw.slice(markerMatch.end).replace(/^ /, "") : raw;
  const lines: string[] = [];
  let inDrawer = false;
  let fence: string | null = null;
  for (const line of body.split("\n")) {
    const fm = /^\s*(`{3,}|~{3,})/.exec(line);
    if (fm) {
      const ch = fm[1][0];
      if (fence === null) fence = ch;
      else if (ch === fence) fence = null;
      lines.push(line);
      continue;
    }
    if (fence !== null) {
      lines.push(line);
      continue;
    }
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
    if (PLANNING_LINE.test(line)) continue; // shown as a date badge, not body text
    if (isPropertyLine(line)) continue; // shown as a chip, not body text
    lines.push(line);
  }
  if (lines.length === 0) lines.push("");
  // Strip the remaining priority / heading prefix from the first line.
  let first = lines[0];
  const pm = /^\[#[ABC]\]\s?/.exec(first);
  if (pm) first = first.slice(pm[0].length);
  const hm = /^(#{1,6}) /.exec(first);
  if (hm) first = first.slice(hm[1].length + 1);
  lines[0] = first;
  while (lines.length > 1 && lines[0].trim() === "") lines.shift();
  return lines;
}
