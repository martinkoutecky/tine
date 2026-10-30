// Single frontend source for a block's HEADER FACETS (marker / priority / heading
// level / scheduled / deadline / properties), read off the ONE lsdoc parse — never
// re-derived by a hand-rolled regex (that was `blockView`, which could disagree with
// Rust `doc.rs` and lsdoc). Mirrors the Rust projection (crates/tine-core/src/doc.rs
// header_facets + planning_dates), so the chip a block shows matches what the backend
// computed for search/query/carry.
//
// PERFORMANCE (P1): the chip renders for every mounted block, incl. off-screen, so we
// must NOT parse every block on load. The store SEEDS this cache from the backend
// BlockDto (one Rust parse, shipped) — so a page load is all cache hits, ZERO frontend
// parses. A miss happens only for a raw the backend hasn't seen yet (the block being
// edited); that one derives from a single wasm lsdoc parse. Robust to every mutation:
// any raw → lookup → hit (backend-known) or one parse (novel).
import { parseBlock } from "./parse";
import { propertyKeyNorm } from "./block";
import { DONE_MARKERS } from "../markers";
import type { Block, Format, Inline } from "./ast";
import { facetKey as keyOf, seededFacetsOf } from "./seededFacets";
export { seedFacets, clearSeededFacets, seededFacetCount } from "./seededFacets";

export interface Facets {
  hasId?: boolean;
  marker: string | null;
  done: boolean;
  priority: "A" | "B" | "C" | null;
  headingLevel: number | null;
  headingAuto: boolean;
  scheduled: string | null;
  deadline: string | null;
  tags: string[];
  properties: [string, string][];
}

const EMPTY: Facets = {
  marker: null,
  done: false,
  priority: null,
  headingLevel: null,
  headingAuto: false,
  scheduled: null,
  deadline: null,
  tags: [],
  properties: [],
};

/** Parse a whole block body. `parseBlock` (→ wasm `parse_block_json`) ALREADY
 *  re-bullets like Rust `render::parse_block` (prepends `"- "`/`"* "` to
 *  `raw.trim_start()`), so lsdoc breaks the marker/priority/heading onto
 *  `blocks[0]` — we must NOT prepend a second bullet (that double-bullets, nesting
 *  the body as a list item). */
export function parseBody(raw: string, format: Format): Block[] {
  return parseBlock(raw, format === "org");
}

// Loaded nodes own backend seeds (seededFacets.ts); large pages keep all hits.
// Locally derived edits have a small transient LRU.
const derived = new Map<string, Facets>();
const DERIVED_MAX = 1024;

/** Build a `Facets` from a backend BlockDto's shipped fields (no parse). */
export function facetsFromDto(d: {
  has_id?: boolean;
  marker?: string;
  priority?: string;
  heading_level?: number;
  scheduled?: string;
  deadline?: string;
  tags?: string[];
  properties?: [string, string][];
}): Facets {
  const marker = d.marker ?? null;
  const p = d.priority;
  const headingProperty = headingPropertyState(d.properties ?? []);
  return {
    hasId: d.has_id,
    marker,
    done: marker != null && DONE_MARKERS.has(marker),
    priority: p === "A" || p === "B" || p === "C" ? p : null,
    headingLevel: d.heading_level ?? headingProperty.level,
    headingAuto: headingProperty.auto,
    scheduled: d.scheduled ?? null,
    deadline: d.deadline ?? null,
    tags: d.tags ?? [],
    properties: d.properties ?? [],
  };
}

function headingPropertyState(properties: readonly [string, string][]): {
  level: number | null;
  auto: boolean;
} {
  const value = properties
    .find(([key]) => propertyKeyNorm(key) === "heading")?.[1]
    ?.trim()
    .toLowerCase();
  return {
    level: value && /^[1-6]$/.test(value) ? Number(value) : null,
    auto: value === "true",
  };
}

/** One heading-level invariant for every frontend renderer. Explicit ATX or
 * numeric-property state wins; boolean auto state is derived from the current
 * tree depth and is never persisted as a computed number. OG parity:
 * `src/main/frontend/components/block.cljs:1948-1959` at `6e7afa8eb`. */
export function effectiveHeadingLevel(facets: Pick<Facets, "headingLevel" | "headingAuto">, depth: number): number | null {
  if (facets.headingLevel !== null) return facets.headingLevel;
  return facets.headingAuto ? Math.min(Math.max(0, depth) + 1, 6) : null;
}

/** A parser-owned negative identity fact for this exact loaded raw and format.
 * O(raw bytes) key lookup, no parse. Unknown/possible identities use blockRegions.
 * Unprojected drafts stay unknown; graph resets clear these facts with facets. */
export function knownIdentityAbsent(raw: string, format: Format): boolean {
  return seededFacetsOf(raw, format)?.hasId === false;
}

/** A block's header facets. Seeded (loaded) blocks and recently-derived edits ⇒ no
 *  parse; a never-seen raw ⇒ one wasm lsdoc parse (cached in the small `derived` LRU). */
export function facetsOf(raw: string, format: Format): Facets {
  const s = seededFacetsOf(raw, format);
  if (s) return s;
  const k = keyOf(raw, format);
  const d = derived.get(k);
  if (d) {
    derived.delete(k); // LRU bump
    derived.set(k, d);
    return d;
  }
  const f = deriveFacets(raw, format);
  if (derived.size >= DERIVED_MAX) derived.delete(derived.keys().next().value!);
  derived.set(k, f);
  return f;
}

function deriveFacets(raw: string, format: Format): Facets {
  const blocks = parseBody(raw, format);
  const head = blocks[0];
  let marker: string | null = null;
  let priority: "A" | "B" | "C" | null = null;
  let headingLevel: number | null = null;
  if (head && (head.kind === "bullet" || head.kind === "heading")) {
    marker = head.marker ?? null;
    const p = head.priority;
    priority = p === "A" || p === "B" || p === "C" ? p : null;
    // Both Bullet and Heading carry the ATX heading level in `.size` (Heading's
    // `.level` is the nesting depth, NOT the heading size).
    const size = head.size ?? null;
    if (size != null && size >= 1 && size <= 6) headingLevel = size;
  }
  const properties: [string, string][] = [];
  for (const b of blocks) if (b.kind === "properties") properties.push(...b.props);
  const headingProperty = headingPropertyState(properties);
  if (headingLevel == null) headingLevel = headingProperty.level;
  const { scheduled, deadline } = planningDates(blocks, raw);
  return {
    marker,
    done: marker != null && DONE_MARKERS.has(marker),
    priority,
    headingLevel,
    headingAuto: headingProperty.auto,
    scheduled,
    deadline,
    tags: tagsOf(blocks),
    properties,
  };
}

export function tagIdentityKey(tag: string): string {
  return tag.trim().toLowerCase();
}

function pushTag(out: string[], seen: Set<string>, tag: string): void {
  const t = tag.trim();
  const key = tagIdentityKey(t);
  if (!t || seen.has(key)) return;
  seen.add(key);
  out.push(t);
}

export function inlineText(inlines: readonly Inline[]): string {
  let out = "";
  for (const i of inlines) {
    switch (i.k) {
      case "plain":
      case "code":
      case "verbatim":
        out += i.text;
        break;
      case "emphasis":
      case "subscript":
      case "superscript":
        out += inlineText(i.children);
        break;
      case "tag":
        out += inlineText(i.children);
        break;
      case "link":
        out += i.label && i.label.length ? inlineText(i.label) : i.url.type === "complex" ? i.url.link ?? "" : i.url.v;
        break;
      case "nested_link":
        out += i.content;
        break;
      case "target":
        out += i.text;
        break;
      case "entity":
        out += i.unicode;
        break;
      case "latex":
        out += i.body;
        break;
      case "hiccup":
        out += i.v;
        break;
    }
  }
  return out;
}

function collectTagsFromInline(inlines: readonly Inline[], out: string[], seen: Set<string>): void {
  for (const i of inlines) {
    if (i.k === "tag") {
      pushTag(out, seen, inlineText(i.children));
      collectTagsFromInline(i.children, out, seen);
    } else if (i.k === "emphasis" || i.k === "subscript" || i.k === "superscript") {
      collectTagsFromInline(i.children, out, seen);
    } else if (i.k === "link" && i.label) {
      collectTagsFromInline(i.label, out, seen);
    }
  }
}

function tagsOf(blocks: readonly Block[]): string[] {
  const out: string[] = [];
  const seen = new Set<string>();
  for (const b of blocks) {
    if ((b.kind === "bullet" || b.kind === "heading") && b.htags) {
      for (const tag of b.htags) pushTag(out, seen, tag);
    }
    if ("inline" in b && Array.isArray(b.inline)) collectTagsFromInline(b.inline, out, seen);
  }
  return out;
}

const UTF8_ENCODER = new TextEncoder();
const UTF8_DECODER = new TextDecoder();

type PlanningSourceLine = { text: string; hasTrailingBody: boolean; lineStart: number };

const isPlanningTimestamp = (i: Inline): i is Extract<Inline, { k: "timestamp" }> =>
  i.k === "timestamp" && (i.ts === "Scheduled" || i.ts === "Deadline");

/** The line-leading planning timestamps of ONE inline flow, by index in `inlines`. lsdoc's span (from the re-bulleted
 *  parser input `"- " + raw.trimStart()`) is mapped back to the original raw bytes. A timestamp is line-leading when
 *  only horizontal whitespace, or an EARLIER line-leading planning timestamp of the same line, precedes it, so
 *  `SCHEDULED: <a> DEADLINE: <b>` on one line is two planning timestamps (OG shows both as chips and nothing in the
 *  body). `hasTrailingBody` is about the text after the LAST timestamp of such a run: it is ordinary body content.
 *  Deliberate OG divergence: a parser-recognized mid-text `Discuss SCHEDULED: <…>` remains content in Tine. */
function planningLinesOf(inlines: readonly Inline[], raw: string): Map<number, PlanningSourceLine> {
  const found = new Map<number, PlanningSourceLine & { end: number; lineEnd: number }>();
  const bytes = UTF8_ENCODER.encode(raw);
  const leadBytes = UTF8_ENCODER.encode(raw.slice(0, raw.length - raw.trimStart().length)).length;
  let last: { lineStart: number; end: number } | null = null;
  for (let index = 0; index < inlines.length; index++) {
    const i = inlines[index];
    if (!isPlanningTimestamp(i)) continue;
    const span = i.span;
    if (!span || span[0] < 2 || span[1] < span[0]) continue;
    const start = span[0] - 2 + leadBytes;
    const end = span[1] - 2 + leadBytes;
    if (start < 0 || end < start || end > bytes.length) continue;
    let lineStart = start;
    while (lineStart > 0 && bytes[lineStart - 1] !== 0x0a) lineStart--;
    let lineEnd = end;
    while (lineEnd < bytes.length && bytes[lineEnd] !== 0x0a) lineEnd++;
    const before = UTF8_DECODER.decode(bytes.slice(lineStart, start));
    const afterEarlier = last !== null && last.lineStart === lineStart && UTF8_DECODER.decode(bytes.slice(last.end, start)).trim() === "";
    if (before.trim() !== "" && !afterEarlier) continue;
    found.set(index, { text: UTF8_DECODER.decode(bytes.slice(start, end)), hasTrailingBody: false, lineStart, end, lineEnd });
    last = { lineStart, end };
  }
  // Trailing body is judged once per run, after its last timestamp.
  const lastOfLine = new Map<number, number>();
  for (const f of found.values()) lastOfLine.set(f.lineStart, Math.max(lastOfLine.get(f.lineStart) ?? 0, f.end));
  const out = new Map<number, PlanningSourceLine>();
  for (const [index, f] of found) {
    const runEnd = lastOfLine.get(f.lineStart)!;
    out.set(index, { text: f.text, lineStart: f.lineStart, hasTrailingBody: UTF8_DECODER.decode(bytes.slice(runEnd, f.lineEnd)).trim() !== "" });
  }
  return out;
}

function angleText(line: string): string | null {
  const lt = line.indexOf("<");
  const gt = lt < 0 ? -1 : line.indexOf(">", lt + 1);
  return lt >= 0 && gt > lt ? line.slice(lt + 1, gt) : null;
}

export type PlanningChip = { ts: "Scheduled" | "Deadline"; value: string | null; hasTrailingBody: boolean };

/** The parser-confirmed standalone SCHEDULED/DEADLINE timestamps of one inline flow, in the exact terms the view's
 *  date chip uses (`planningDates`): lsdoc's Timestamp, line-leading as `planningLinesOf` defines. `value` is the
 *  text between the angle brackets. The CM6 editor renders the same chip from this, so view and edit cannot
 *  disagree on what a planning line is. */
export function planningChipsOf(inlines: readonly Inline[], raw: string): Map<Inline, PlanningChip> {
  const out = new Map<Inline, PlanningChip>();
  for (const [index, line] of planningLinesOf(inlines, raw)) {
    const i = inlines[index];
    if (isPlanningTimestamp(i)) out.set(i, { ts: i.ts as "Scheduled" | "Deadline", value: angleText(line.text), hasTrailingBody: line.hasTrailingBody });
  }
  return out;
}

const isBreak = (i: Inline | undefined) => i?.k === "break" || i?.k === "hardbreak";

/** Remove genuine line-leading planning timestamps from the body AST while
 *  preserving any following text in the SAME paragraph. lsdoc represents both
 *  same-line and next-line body text in the planning timestamp's inline flow, so
 *  filtering the whole AST block either leaks the timestamp or deletes body text. */
export function stripPlanningLines(blocks: Block[], raw: string): Block[] {
  // Normal blocks must retain the pre-fix one-filter hot path: scrolling a large
  // page mounts thousands of bodies, and allocating a second AST array for every
  // block measurably regresses that path. This is only a cheap candidate gate;
  // lsdoc Timestamp + source-span boundaries still decide semantics below.
  if (!raw.includes("SCHEDULED:") && !raw.includes("DEADLINE:")) return blocks;
  return blocks.map((b) => {
    if (b.kind !== "paragraph" && b.kind !== "bullet" && b.kind !== "heading") return b;
    const planning = planningLinesOf(b.inline, raw);
    if (planning.size === 0) return b;

    const remove = new Set(planning.keys());
    let previous: [number, PlanningSourceLine] | null = null;
    for (const [index, line] of planning) {
      // The blanks between two timestamps of one line go with them.
      if (previous && previous[1].lineStart === line.lineStart) {
        for (let k = previous[0] + 1; k < index; k++) {
          const between = b.inline[k];
          if (between.k === "plain" && between.text.trim() === "") remove.add(k);
        }
      }
      previous = [index, line];
      if (line.hasTrailingBody) continue;
      const next = b.inline[index + 1];
      const prev = b.inline[index - 1];
      if (isBreak(next)) remove.add(index + 1);
      else if (isBreak(prev)) remove.add(index - 1);
    }
    return { ...b, inline: b.inline.filter((_, index) => !remove.has(index)) };
  });
}

/** SCHEDULED/DEADLINE display text for date chrome. The Timestamp comes from lsdoc;
 *  its parser-provided byte span proves the token starts a source line (or follows another
 *  planning timestamp of that line), so mid-text and inline-code lookalikes remain
 *  ordinary body content. */
function planningDates(blocks: Block[], raw: string): { scheduled: string | null; deadline: string | null } {
  let scheduled: string | null = null;
  let deadline: string | null = null;
  for (const b of blocks) {
    if (b.kind !== "paragraph" && b.kind !== "bullet" && b.kind !== "heading") continue;
    for (const [index, line] of planningLinesOf(b.inline, raw)) {
      const i = b.inline[index];
      if (!isPlanningTimestamp(i)) continue;
      const value = angleText(line.text);
      if (i.ts === "Scheduled" && scheduled === null) scheduled = value;
      if (i.ts === "Deadline" && deadline === null) deadline = value;
    }
  }
  return { scheduled, deadline };
}

export { EMPTY as EMPTY_FACETS };

/** OG graph_parser/block.cljs extract-blocks keeps the earliest parsed property
 * group. PropertyRows folds duplicates to their last value (extract-properties).
 * Presentation only: raw text and query/edit facets retain every property. Cost
 * O(raw bytes) for a native empty-facet lookup; otherwise O(block text) for raw
 * input (cached lsdoc parse), O(AST nodes) for parsed input. No writes. */
export function renderedProperties(raw: string | readonly Block[], format: Format): [string, string][] {
  if (typeof raw === "string") {
    const native = seededFacetsOf(raw, format);
    // hasId is present on complete native projections, absent on unprojected
    // drafts. An empty native property set proves there is no display group;
    // positive sets still use the AST's earliest-group presentation policy.
    // Reuse the raw-keyed fact so edits cannot inherit a stale negative (I-12).
    if (native?.hasId !== undefined && native.properties.length === 0) return [];
  }
  const blocks = typeof raw === "string" ? parseBody(raw, format) : raw;
  const group = blocks.find((block) => block.kind === "properties");
  return group?.kind === "properties" ? group.props : [];
}
