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
import { DONE_MARKERS } from "../markers";
import type { Block, Format, Inline } from "./ast";

export interface Facets {
  marker: string | null;
  done: boolean;
  priority: "A" | "B" | "C" | null;
  headingLevel: number | null;
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

// Two tiers so a page bigger than any cap can't thrash the cache into parse-all-on-
// load (audit P2 — a 4097-block page evicted its own seeds before they were read):
//  - `seeded`: backend-shipped facets for the CURRENTLY-LOADED blocks. NEVER LRU-
//    evicted (so an arbitrarily large page is still all hits); cleared wholesale on
//    graph switch / store reset (`clearSeededFacets`). Bounded by the loaded graph,
//    which is already in memory.
//  - `derived`: facets computed locally for a raw the backend hasn't shipped (the
//    block being edited). Small LRU — transient.
const seeded = new Map<string, Facets>();
const derived = new Map<string, Facets>();
const DERIVED_MAX = 1024;
const keyOf = (raw: string, format: Format) => format + "\0" + raw;

/** Build a `Facets` from a backend BlockDto's shipped fields (no parse). */
export function facetsFromDto(d: {
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
  return {
    marker,
    done: marker != null && DONE_MARKERS.has(marker),
    priority: p === "A" || p === "B" || p === "C" ? p : null,
    headingLevel: d.heading_level ?? null,
    scheduled: d.scheduled ?? null,
    deadline: d.deadline ?? null,
    tags: d.tags ?? [],
    properties: d.properties ?? [],
  };
}

/** Seed the never-evicted tier from the backend-computed facets — no parse. */
export function seedFacets(raw: string, format: Format, f: Facets): void {
  seeded.set(keyOf(raw, format), f);
}

/** Drop all backend-seeded facets — call on graph switch / full store reset so the
 *  previous graph's blocks don't linger (store `resetStore`). */
export function clearSeededFacets(): void {
  seeded.clear();
}

/** A block's header facets. Seeded (loaded) blocks and recently-derived edits ⇒ no
 *  parse; a never-seen raw ⇒ one wasm lsdoc parse (cached in the small `derived` LRU). */
export function facetsOf(raw: string, format: Format): Facets {
  const k = keyOf(raw, format);
  const s = seeded.get(k);
  if (s) return s;
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
  const { scheduled, deadline } = planningDates(blocks, raw);
  return {
    marker,
    done: marker != null && DONE_MARKERS.has(marker),
    priority,
    headingLevel,
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

/** Map an lsdoc span from the re-bulleted parser input (`"- " + raw.trimStart()`)
 *  back to the original raw bytes. Return the spanned source only when it occupies
 *  a whole source line (surrounding horizontal whitespace is allowed). This is the
 *  crucial distinction between a real planning line and mid-text
 *  `Discuss SCHEDULED: <…>`: lsdoc recognizes both as Timestamp, while only the
 *  former belongs in header chrome. */
function standaloneSourceLine(raw: string, span: readonly [number, number] | undefined): string | null {
  if (!span || span[0] < 2 || span[1] < span[0]) return null;
  const trimmed = raw.trimStart();
  const leading = raw.slice(0, raw.length - trimmed.length);
  const leadBytes = UTF8_ENCODER.encode(leading).length;
  const bytes = UTF8_ENCODER.encode(raw);
  const start = span[0] - 2 + leadBytes;
  const end = span[1] - 2 + leadBytes;
  if (start < 0 || end < start || end > bytes.length) return null;

  let lineStart = start;
  while (lineStart > 0 && bytes[lineStart - 1] !== 0x0a) lineStart--;
  let lineEnd = end;
  while (lineEnd < bytes.length && bytes[lineEnd] !== 0x0a) lineEnd++;
  const before = UTF8_DECODER.decode(bytes.slice(lineStart, start));
  const after = UTF8_DECODER.decode(bytes.slice(end, lineEnd));
  if (before.trim() !== "" || after.trim() !== "") return null;
  return UTF8_DECODER.decode(bytes.slice(start, end));
}

function planningLineText(i: Inline, raw: string): string | null {
  if (i.k !== "timestamp" || (i.ts !== "Scheduled" && i.ts !== "Deadline")) return null;
  return standaloneSourceLine(raw, i.span);
}

function angleText(line: string): string | null {
  const lt = line.indexOf("<");
  const gt = lt < 0 ? -1 : line.indexOf(">", lt + 1);
  return lt >= 0 && gt > lt ? line.slice(lt + 1, gt) : null;
}

/** Remove genuine whole-line planning timestamps from the body AST while preserving
 *  any following text in the SAME paragraph. lsdoc represents
 *  `title\nSCHEDULED\nbody` as a planning Timestamp + Break + body Plain in one
 *  Paragraph, so filtering the whole AST block either leaks the timestamp into the
 *  body or deletes the body. Parser spans let us remove only the source line. */
export function stripPlanningLines(blocks: Block[], raw: string): Block[] {
  // Normal blocks must retain the pre-fix one-filter hot path: scrolling a large
  // page mounts thousands of bodies, and allocating a second AST array for every
  // block measurably regresses that path. This is only a cheap candidate gate;
  // lsdoc Timestamp + source-span boundaries still decide semantics below.
  if (!raw.includes("SCHEDULED:") && !raw.includes("DEADLINE:")) return blocks;
  return blocks.map((b) => {
    if (b.kind !== "paragraph" && b.kind !== "bullet" && b.kind !== "heading") return b;
    const planning = new Set<number>();
    b.inline.forEach((i, index) => {
      if (planningLineText(i, raw) !== null) planning.add(index);
    });
    if (planning.size === 0) return b;

    const remove = new Set(planning);
    for (const index of planning) {
      const next = b.inline[index + 1];
      const prev = b.inline[index - 1];
      if (next?.k === "break" || next?.k === "hardbreak") remove.add(index + 1);
      else if (prev?.k === "break" || prev?.k === "hardbreak") remove.add(index - 1);
    }
    return { ...b, inline: b.inline.filter((_, index) => !remove.has(index)) };
  });
}

/** SCHEDULED/DEADLINE display text for date chrome. The Timestamp comes from lsdoc;
 *  its parser-provided byte span proves the token occupies a whole source line, so
 *  mid-text and inline-code lookalikes remain ordinary body content. */
function planningDates(blocks: Block[], raw: string): { scheduled: string | null; deadline: string | null } {
  let scheduled: string | null = null;
  let deadline: string | null = null;
  for (const b of blocks) {
    if (b.kind !== "paragraph" && b.kind !== "bullet" && b.kind !== "heading") continue;
    for (const i of b.inline) {
      const line = planningLineText(i, raw);
      if (line === null || i.k !== "timestamp") continue;
      const value = angleText(line);
      if (i.ts === "Scheduled" && scheduled === null) scheduled = value;
      if (i.ts === "Deadline" && deadline === null) deadline = value;
    }
  }
  return { scheduled, deadline };
}

export { EMPTY as EMPTY_FACETS };
