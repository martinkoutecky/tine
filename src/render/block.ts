// Helpers to derive a block's *rendered* view from its raw text. Raw stays
// authoritative (round-trip); these are computed projections.

import type { Format } from "./ast";
import { MARKERS } from "../markers";
import { parsePageHeaderPropertyLine, splitPagePreamble } from "../editor/properties";

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

/** A page's pre-block properties as `[key, value]` pairs. Markdown reads
 *  `key:: value` lines; org reads `#+KEY: value` file directives plus a top
 *  `:PROPERTIES:` … `:END:` drawer's `:key: value` lines (org keys lowercased,
 *  as OG/mldoc stores them). Order preserved. */
export function pageProperties(
  preBlock: string | null | undefined,
  format: Format = "md"
): [string, string][] {
  if (!preBlock) return [];
  const out: [string, string][] = [];
  if (format === "org") {
    let inDrawer = false;
    for (const line of preBlock.split("\n")) {
      const t = line.trim();
      if (/^:PROPERTIES:$/i.test(t)) {
        inDrawer = true;
        continue;
      }
      if (/^:END:$/i.test(t)) {
        inDrawer = false;
        continue;
      }
      const dir = /^#\+([A-Za-z0-9_-]+):\s*(.*)$/.exec(t);
      if (dir) {
        out.push([dir[1].toLowerCase(), dir[2].trim()]);
        continue;
      }
      if (inDrawer) {
        const d = /^:([A-Za-z0-9_-]+):\s*(.*)$/.exec(t);
        if (d) out.push([d[1].toLowerCase(), d[2].trim()]);
      }
    }
  } else {
    const header = splitPagePreamble(preBlock).properties;
    if (!header) return out;
    for (const line of header.split("\n")) {
      const property = parsePageHeaderPropertyLine(line);
      if (property) out.push([property.key, property.value.trim()]);
    }
  }
  return out;
}

/** The alias names declared by a page's pre-block (`alias::` in markdown,
 *  `#+ALIAS:` / `:alias:` in org), comma-separated. Empty if none. */
export function aliasNames(
  preBlock: string | null | undefined,
  format: Format = "md"
): string[] {
  const out: string[] = [];
  for (const [k, v] of pageProperties(preBlock, format)) {
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
  const lines: string[] = [];
  let inDrawer = false;
  let fence: string | null = null;
  for (const line of raw.split("\n")) {
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
  // Strip the marker / priority / heading prefix from the first line (chrome that
  // facetsOf surfaces separately).
  let first = lines[0];
  for (const m of MARKERS) {
    if (first === m || first.startsWith(m + " ")) {
      first = first.slice(m.length).replace(/^ /, "");
      break;
    }
  }
  const pm = /^\[#[ABC]\]\s?/.exec(first);
  if (pm) first = first.slice(pm[0].length);
  const hm = /^(#{1,6}) /.exec(first);
  if (hm) first = first.slice(hm[1].length + 1);
  lines[0] = first;
  while (lines.length > 1 && lines[0].trim() === "") lines.shift();
  return lines;
}
