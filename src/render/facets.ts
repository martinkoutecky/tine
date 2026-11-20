// Single frontend source for a block's HEADER FACETS (marker / priority / heading
// level / scheduled / deadline / properties), read off the ONE lsdoc parse — never
// re-derived by a hand-rolled regex (that was `blockView`, which could disagree with
// Rust `doc.rs` and lsdoc). Uses the same planning tokens as the native projection,
// so the chip a block shows matches what the backend computes for
// search/query/carry.
//
// PERFORMANCE (P1): the chip renders for every mounted block, incl. off-screen, so we
// must NOT parse every block on load. The store SEEDS this cache from the backend
// BlockDto (one Rust parse, shipped) — so a page load is all cache hits, ZERO frontend
// parses. A miss happens only for a raw the backend hasn't seen yet (the block being
// edited); that one derives from a single wasm lsdoc parse. Robust to every mutation:
// any raw → lookup → hit (backend-known) or one parse (novel).
import { blockRegions, parseBlock, renderPlanningOf } from "./parse";
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
 *  OG's ECMAScript left trim), so lsdoc breaks the marker/priority/heading onto
 *  `blocks[0]` — we must NOT prepend a second bullet (that double-bullets, nesting
 *  the body as a list item). */
export function parseBody(raw: string, format: Format, untitled = false): Block[] {
  return parseBlock(raw, format === "org", untitled);
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

/** The heading state a block's own `heading::` property declares (what the editor needs when the property is
 * hidden from its text), in the shape `effectiveHeadingLevel` takes. */
export function headingPropertyFacets(properties: readonly [string, string][]): Pick<Facets, "headingLevel" | "headingAuto"> {
  const { level, auto } = headingPropertyState(properties);
  return { headingLevel: level, headingAuto: auto };
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
  // GH #895: reuse the cached bundle's ownership of a swallowed author row.
  if (format === "md" && raw.includes("::") && raw.includes(">") && !properties.some(([key]) => propertyKeyNorm(key) === "tine.author")) {
    for (const p of blockRegions(raw, format).properties) if (p.primary && propertyKeyNorm(p.key) === "tine.author") properties.push([p.key, p.value]);
  }
  const headingProperty = headingPropertyState(properties);
  if (headingLevel == null) headingLevel = headingProperty.level;
  let scheduled: string | null = null;
  let deadline: string | null = null;
  for (const p of renderPlanningOf(blocks)) {
    if (!p.top_level) continue;
    if (p.kind === "Scheduled") scheduled ??= p.value;
    else deadline ??= p.value;
  }
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

const isBreak = (i: Inline | undefined) => i?.k === "break" || i?.k === "hardbreak";

/** Hide the bundle's planning tokens while retaining body text in the same inline flow. */
export function stripPlanningLines(blocks: Block[], raw: string): Block[] {
  const planning = renderPlanningOf(blocks);
  // Preserve the existing body filter for blocks with uppercase keywords.
  if (planning.length === 0 || (!raw.includes("SCHEDULED:") && !raw.includes("DEADLINE:"))) return blocks;
  const byStart = new Map(planning.filter((p) => p.top_level).map((p) => [p.span_start, p]));
  if (byStart.size === 0) return blocks;
  return blocks.map((b) => {
    if (b.kind !== "paragraph" && b.kind !== "bullet" && b.kind !== "heading") return b;
    const remove = new Set<number>();
    let previous: { index: number; lineStart: number } | undefined;
    for (const [index, inline] of b.inline.entries()) {
      if (inline.k !== "timestamp" || !inline.span) continue;
      const line = byStart.get(inline.span[0]);
      if (!line) continue;
      remove.add(index);
      // Blanks between timestamps on the same source line go with them.
      if (previous?.lineStart === line.line_start) {
        for (let k = previous.index + 1; k < index; k++) {
          const between = b.inline[k];
          if (between.k === "plain" && between.text.trim() === "") remove.add(k);
        }
      }
      previous = { index, lineStart: line.line_start };
      if (line.has_trailing_body) continue;
      if (isBreak(b.inline[index + 1])) remove.add(index + 1);
      else if (isBreak(b.inline[index - 1])) remove.add(index - 1);
    }
    return remove.size ? { ...b, inline: b.inline.filter((_, index) => !remove.has(index)) } : b;
  });
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
