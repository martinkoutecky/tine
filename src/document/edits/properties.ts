import type { PageDto } from "../../types";
import { ordered_list_glyph } from "../../render/wasm/lsdoc_wasm.js";
import { scheduleParts, planningTimestamp } from "../../editor/repeat";
import { blockRegions, editBlock, parserReady } from "../../render/parse";
import { bumpCollapseEpochs, doc, formatForBlock, pageByName, setDoc, freshId, type ReadonlyFeedPage } from "../model";
import { facetsOf } from "../../render/facets";
import { propertyKeyNorm } from "../../render/block";
import { pushUndo } from "../history";
import { orgRawWithProperty } from "./identity";
import { markDirty, noteTitleIdentityIntent } from "../save/engine";
import { isPropertiesOnly, upsertPropertyLine, splitPagePreamble, isPageHeaderPropertiesOnly, splitProps, joinProps, isBuiltinHidden, pagePropertyEntries, pagePartsWithProperty } from "../../editor/properties";
import { produce } from "solid-js/store";
import { type Format } from "../../types";
import { graphRewriteFrozen } from "../graphRewriteState";
import { pushToast } from "../../toasts";

/** Pure Markdown property rewrite for one compound store mutation. It scans only
 * the canonical head (title — or, when the first line is itself a property, no
 * title — planning, contiguous properties) plus the legacy trailing property
 * block, so a `key::` lookalike in body text is never touched or reordered. A
 * line of a code/src/example block (`literalBlockOfLine`) is never a head or
 * trailing property, and a new property never lands inside one: when the head
 * position is inside a block that opens on the title line (a whole-block code
 * fence), the property goes at the end instead (C3 L13; a named OG divergence,
 * see editor/literalLines.ts). Keys match case-insensitively, like
 * blockProperty/facetsOf: the first head match is replaced in place with the
 * file's spelling, and every other match (head or trailing, any case) is
 * removed. Existing order is kept. */
function markdownRawWithProperty(raw: string, key: string, value: string | null): string {
  return editBlock(raw, "md", { kind: "property", key, value });
}

/** Current value of a block property, read through the ONE lsdoc-backed
 *  recognizer (facetsOf) — a raw line scan here returned property-lookalikes
 *  from code fences/body text and silently suppressed real config writes
 *  (review finding). Case-insensitive key match, like OG. */
export function blockProperty(id: string, key: string): string | null {
  const node = doc.byId[id];
  if (!node) return null;
  const lower = key.toLowerCase();
  for (const [k, v] of facetsOf(node.raw, formatForBlock(id)).properties) {
    if (k.toLowerCase() === lower) return v.trim();
  }
  return null;
}

/** Whether a block lives on a read-only page (the org round-trip gate) — sheet
 *  write paths outside the block editor must consult this before mutating. */
export function blockPageReadOnly(id: string): boolean {
  const n = doc.byId[id];
  return graphRewriteFrozen() || (n ? (pageByName(n.page)?.readOnly ?? false) : false);
}

/** Store mutation boundary. UI affordances also hide on read-only pages, but
 * every write API must enforce this itself because menus/shortcuts/sheets can
 * call the store without entering the textarea. Guide pages are virtual and
 * equally non-writable. */
export function pageWritable(name: string): boolean {
  const page = pageByName(name);
  return !graphRewriteFrozen() && !!page && !page.readOnly && !page.guide;
}

export function blockWritable(id: string): boolean {
  const node = doc.byId[id];
  return !!node && pageWritable(node.page);
}

/** Set (or remove, when value is null) a `key:: value` block property. Property
 *  lines live immediately after the first line, before body text, matching OG's
 *  block-property placement and keeping every property writer on one path. */
export function setBlockProperty(id: string, key: string, value: string | null) {
  const node = doc.byId[id];
  if (!node || !blockWritable(id)) return;
  let raw: string;
  try {
    raw = editBlock(node.raw,formatForBlock(id),{kind:"property",key,value});
  } catch (e) { pushToast(String(e),"error"); return; }
  if (raw === node.raw) return;
  pushUndo(`prop:${id}:${key}`, [node.page]);
  setDoc("byId",id,"raw",raw);
  markDirty(node.page,"save-block");
}

/** Where a page's properties are read from and written to, in file order:
 *  Org → the pre-block; Markdown → the pre-block (when non-empty, or when there
 *  is no header root) then a first root that IS the page header (a transient
 *  header editor, or a properties-only root). A transient header editor is the
 *  whole source: the pre-block then holds only the post-header remainder.
 *  `exclude` names a root the caller shows as an ordinary block instead. */
type PropertyPart = { kind: "preBlock"; text: string } | { kind: "root"; id: string; text: string };
function pagePropertyParts(page: ReadonlyFeedPage, exclude: string | null): PropertyPart[] {
  const first = page.format === "md" ? doc.byId[page.roots[0]] : undefined;
  const root = first && first.id !== exclude && (first.originatedFromPageHeader || isPropertiesOnly(first.raw)) ? first : null;
  const rootPart: PropertyPart[] = root ? [{ kind: "root", id: root.id, text: root.raw }] : [];
  if (root?.originatedFromPageHeader) return rootPart;
  return page.preBlock || !root ? [{ kind: "preBlock", text: page.preBlock ?? "" }, ...rootPart] : rootPart;
}

/** Every page-property line of this loaded page as `[key, value]`, in file
 *  order with duplicates — exactly what the page header renders (Page.tsx calls
 *  this with `exclude` = a first root it shows as a block). Grammar:
 *  editor/properties.ts `pagePropertyEntries`. Reactive for loaded pages (reads
 *  the store). A DTO preamble reads only that preamble; it
 *  never consults a similarly named loaded page. Cost O(source bytes). */
export function pageHeaderProperties(page: ReadonlyFeedPage | Pick<PageDto, "pre_block" | "format">, exclude: string | null = null): [string, string][] {
  if ("pre_block" in page) {
    return pagePropertyEntries(page.pre_block, page.format ?? "md").map((e) => [e.key, e.value]);
  }
  return pagePropertyParts(page, exclude).flatMap((part) =>
    pagePropertyEntries(part.text, page.format).map((e): [string, string] => [e.key, e.value]));
}

/** This page's properties, the ONE answerer the properties panel and
 *  {@link readPageProperty} use: {@link pageHeaderProperties} with the first
 *  spelling of each key (case-insensitive) kept. Markdown: the fence-aware
 *  canonical header of the pre-block, then a properties-only / header-editor
 *  first root — never prose or fenced `key::` lookalikes. Org: `#+key:`
 *  directives (space optional) and `:PROPERTIES:` drawer lines, keys lowercased.
 *  Every listed key is written back to the line it was read from by
 *  {@link setPageProperty}. Missing/unloaded page → []. Reactive. Cost
 *  O(pre-block + first-root bytes). */
export function readPageProperties(pageName: string): [string, string][] {
  const page = pageByName(pageName);
  if (!page) return [];
  const seen = new Set<string>();
  return pageHeaderProperties(page).filter(([key]) => !seen.has(key.toLowerCase()) && !!seen.add(key.toLowerCase()));
}

/** Value of page property `key` (case-insensitive) as {@link readPageProperties}
 *  lists it, or null. Cost as readPageProperties. */
export function readPageProperty(pageName: string, key: string): string | null {
  const lower = key.toLowerCase();
  return readPageProperties(pageName).find(([k]) => k.toLowerCase() === lower)?.[1] ?? null;
}

/** Set/clear a page property on the line {@link readPageProperties} lists it
 *  from (first match replaced in place, case-insensitive duplicates removed); a
 *  new key is prepended to the first property source (pre-block, or the
 *  header root when that is the whole source). Missing/read-only pages and
 *  no-op writes are ignored. Records undo and schedules a guarded page save.
 *  Cost O(property text) now, then one page save. A childed transient header
 *  remains intact if its last key clears; that invalid draft cannot save until
 *  its children move out. */
export function setPageProperty(pageName: string, key: string, value: string | null) {
  const idx = doc.pages.findIndex((x) => x.name === pageName);
  if (idx < 0 || !pageWritable(pageName)) return;
  const page = doc.pages[idx];
  const parts = pagePropertyParts(page, null);
  const next = pagePartsWithProperty(parts.map((part) => part.text), page.format, key, value);
  if (next.every((text, i) => text === parts[i].text)) return;
  if (key.toLowerCase() === "title") noteTitleIdentityIntent(pageName);
  pushUndo(`pageprop:${pageName}:${key}`, [pageName]);
  setDoc(produce((s) => {
    parts.forEach((part, i) => {
      if (next[i] === part.text) return;
      if (part.kind === "preBlock") {
        s.pages[idx].preBlock = next[i].trim() === "" ? null : next[i];
        return;
      }
      const node = s.byId[part.id];
      if (node.originatedFromPageHeader && next[i] === "" && node.children.length === 0) {
        if (s.pages[idx].roots[0] === part.id) s.pages[idx].roots.shift();
        delete s.byId[part.id];
      } else node.raw = next[i];
    });
  }));
  markDirty(pageName, "save-block");
}

/** Materialize an existing canonical Markdown page header as Tine's ordinary
 * first-root editor. This is representation-only: no undo entry, dirty flag or
 * save is created until the user actually changes the node. */
export function beginPageHeaderEdit(pageName: string): string | null {
  const page = pageByName(pageName);
  if (!page || page.format !== "md" || !pageWritable(pageName)) return null;
  const first = doc.byId[page.roots[0]];
  if (first && (first.originatedFromPageHeader || (!page.preBlock && isPropertiesOnly(first.raw)))) {
    return first.id;
  }

  const split = splitPagePreamble(page.preBlock);
  if (!split.properties || !isPageHeaderPropertiesOnly(split.properties)) return null;
  const id = freshId();
  setDoc(
    produce((s) => {
      const index = s.pages.findIndex((p) => p.name === pageName);
      s.pages[index].preBlock = split.remainder;
      s.byId[id] = {
        id,
        raw: split.properties!,
        collapsed: false,
        parent: null,
        page: pageName,
        children: [],
        originatedFromPageHeader: true,
      };
      s.pages[index].roots.unshift(id);
    })
  );
  return id;
}

/** Remove a deleted transient header root after its editor exits. Invalid
 * drafts intentionally remain present and editable; pageToDto keeps them from
 * reaching native persistence. */
export function finishPageHeaderEdit(id: string): void {
  reportInvalidPageHeaderOnExit(id);
  const node = doc.byId[id];
  if (!node?.originatedFromPageHeader || node.raw !== "" || node.children.length > 0 || graphRewriteFrozen()) return;
  setDoc(
    produce((s) => {
      const page = s.pages.find((p) => p.name === node.page);
      if (page?.roots[0] === id) page.roots.shift();
      delete s.byId[id];
    })
  );
}

/** Report a still-invalid header when its editor closes, once per edit exit. */
export function reportInvalidPageHeaderOnExit(id: string): void {
  const node = doc.byId[id];
  if (!node?.originatedFromPageHeader) return;
  const raw = node.raw.replace(/\n+$/, "");
  if (node.children.length > 0 || (raw !== "" && !isPageHeaderPropertiesOnly(raw)))
    pushToast("Page-header properties must contain only valid key:: value lines before they can be saved.", "error");
}

/** A successful save folded the first root into the file's preamble. Keep the
 * live editor's origin in sync so the next in-progress edit stays a header. */
export function adoptFoldedPageHeader(pageName: string, preBlock: string): void {
  const page = pageByName(pageName);
  if (!page || page.preBlock) return;
  const first = doc.byId[page.roots[0]];
  if (!first || first.children.length > 0 || first.originatedFromPageHeader) return;
  if (splitPagePreamble(preBlock).properties !== first.raw.replace(/\n+$/, "")) return;
  setDoc("byId", first.id, "originatedFromPageHeader", true);
}

/** Turn ordinary text before the first Markdown bullet into a real first block
 * only when the user chooses to edit it (GH #85). Until then the preamble stays
 * byte-preserved and an unrelated save cannot silently add an outline marker. */
export function promotePagePreamble(pageName: string): string | null {
  const page = pageByName(pageName);
  if (!page || page.format !== "md" || !pageWritable(pageName)) return null;
  const { properties, content } = splitPagePreamble(page.preBlock);
  if (!content) return null;
  pushUndo(`promote-preamble:${pageName}`, [pageName]);
  const id = freshId();
  setDoc(
    produce((s) => {
      const index = s.pages.findIndex((p) => p.name === pageName);
      s.pages[index].preBlock = properties;
      s.byId[id] = { id, raw: content, collapsed: false, parent: null, page: pageName, children: [] };
      const markedHeader = s.byId[s.pages[index].roots[0]]?.originatedFromPageHeader;
      s.pages[index].roots.splice(markedHeader ? 1 : 0, 0, id);
    })
  );
  markDirty(pageName, ["save-block", "insert-blocks"]);
  return id;
}

/** Toggle a property: set it to `value`, or remove it if already that value. */
export function toggleBlockProperty(id: string, key: string, value: string) {
  setBlockProperty(id, key, blockProperty(id, key) === value ? null : value);
}

const ORDER_KEY = "logseq.order-list-type";
export function isOrdered(id: string | null | undefined): boolean {
  const node = id ? doc.byId[id] : undefined;
  return !!node && orderedFromProperties(facetsOf(node.raw, formatForBlock(id!)).properties);
}

function orderedFromProperties(properties: readonly (readonly [string, string])[]): boolean {
  // Same key fold as Rust `DocBlock::property` (the static export's `own_ordered`): I-12.
  return properties.find(([key]) => propertyKeyNorm(key) === ORDER_KEY)?.[1].trim() === "number";
}

export function orderListTypeFromRaw(raw: string, format: Format): string | null {
  for (const [key, value] of facetsOf(raw, format).properties) {
    if (propertyKeyNorm(key) === ORDER_KEY) return value.trim();
  }
  return null;
}

/** The one format-aware raw transform for the block-level list property.
 * `splitProps`/`joinProps` are the audited metadata path: they preserve visible
 * body bytes, ignore property lookalikes inside fences, and emit Org drawers.
 * OG writes both the in-memory property and serialized content at
 * `src/main/frontend/modules/outliner/core.cljs:420-433` (6e7afa8eb). */
export function rawWithOrderListType(raw: string, value: string | null, format: Format): string {
  const { visible } = splitProps(raw, (key) => propertyKeyNorm(key) === ORDER_KEY, format);
  if (value === null) return visible;
  const property = format === "org" ? `:${ORDER_KEY}: ${value}` : `${ORDER_KEY}:: ${value}`;
  return joinProps(visible, property, format);
}

/** Preserve a source's explicit list type; otherwise inherit the target's.
 * This is OG's common move/insert rule (`outliner/core.cljs:420-433,536-555`
 * at 6e7afa8eb), shared by drag and every structural outline insertion below. */
export function rawWithInheritedOrderListType(raw: string, format: Format, targetId: string | null | undefined): string {
  if (orderListTypeFromRaw(raw, format) !== null) return raw;
  const targetType = targetId ? blockProperty(targetId, ORDER_KEY) : null;
  return targetType === null ? raw : rawWithOrderListType(raw, targetType, format);
}

function setOwnNumberedList(id: string, enabled: boolean, visibleText?: string): boolean {
  const node = doc.byId[id];
  if (!node || !blockWritable(id)) return false;
  const format = formatForBlock(id);
  const base = visibleText === undefined
    ? node.raw
    : joinProps(visibleText, splitProps(node.raw, isBuiltinHidden, format).hidden, format);
  const next = rawWithOrderListType(base, enabled ? "number" : null, format);
  if (next === node.raw) return false;
  pushUndo(`own-numbered:${id}`, [node.page]);
  setDoc("byId", id, "raw", next);
  markDirty(node.page, "save-block");
  return true;
}

/** Make this block an own numbered-list item. When `visibleText` is supplied,
 * replacing the editor trigger and writing the property are one store mutation. */
export function makeOwnNumberedList(id: string, visibleText?: string): boolean {
  return setOwnNumberedList(id, true, visibleText);
}

export function removeOwnNumberedList(id: string): boolean {
  if (!isOrdered(id)) return false;
  return setOwnNumberedList(id, false);
}

export function toggleOwnNumberedList(id: string): boolean {
  return setOwnNumberedList(id, !isOrdered(id));
}

/** Empty Enter stops only a non-nested own list: an ordered parent keeps the
 * ordinary insert/inherit path. Transcribed from OG
 * `src/main/frontend/handler/editor.cljs:2498-2502` (6e7afa8eb). */
export function stopOwnNumberedListOnEmptyEnter(id: string, visibleText: string): boolean {
  const node = doc.byId[id];
  if (!node || visibleText.trim() !== "" || !isOrdered(id) || isOrdered(node.parent)) return false;
  return removeOwnNumberedList(id);
}
/** The ordered-list label for a block whose `logseq.order-list-type` is `number`
 *  (else null) — the block's OWN bullet, like OG. The index counts this block
 *  plus the run of consecutive ordered siblings immediately before it; the glyph
 *  cycles number → letter → roman by the depth of consecutive ordered ancestors
 *  (mod 3), so nested ordered lists read 1. → a. → i. like OG. The optional
 *  ownProperties reading avoids reparsing the editor buffer; sibling and ancestor
 *  work remains O(consecutive ordered siblings + ordered ancestors). */
export function orderedListMarker(id: string, ownProperties?: readonly (readonly [string, string])[]): string | null {
  const node = doc.byId[id];
  if (!node || !(ownProperties ? orderedFromProperties(ownProperties) : isOrdered(id))) return null;
  const siblings = node.parent
    ? doc.byId[node.parent]?.children
    : doc.pages.find((p) => p.name === node.page)?.roots;
  let idx = 1;
  if (siblings) {
    for (let i = siblings.indexOf(id) - 1; i >= 0 && isOrdered(siblings[i]); i--) idx++;
  }
  let depth = 0;
  for (let p = node.parent; isOrdered(p); p = doc.byId[p!]?.parent ?? null) depth++;
  return ordered_list_glyph(idx, depth);
}

/** Flip the `[ ]`/`[x]` checkbox of ONE list item: the token at `column` (a UTF-16 offset in the raw
 *  line `lineIndex`). Both coordinates come from the item's own lsdoc source span (render/body.tsx
 *  `toggleAstCheckbox`), so the line's other `[ ]`/`[x]` text (a literal in the label) is never touched
 *  and two items with the same label flip independently. Pure `[ ]`↔`[x]` text swap — round-trips
 *  as standard markdown (and renders/ticks in OG + mobile). A position that is not a checkbox token is a no-op. */
export function toggleListItemAtIndex(id: string, lineIndex: number, column: number) {
  const node = doc.byId[id];
  if (!node || !blockWritable(id)) return;
  const lines = node.raw.split("\n");
  const ln = lines[lineIndex];
  const token = ln?.slice(column, column + 3);
  if (ln === undefined || token === undefined || !/^\[[ xX]\]$/.test(token)) return;
  const next = ln.slice(0, column) + (token === "[ ]" ? "[x]" : "[ ]") + ln.slice(column + 3);
  pushUndo(`listcheck:${id}`, [node.page]);
  lines[lineIndex] = next;
  setDoc("byId", id, "raw", lines.join("\n"));
  markDirty(node.page, "save-block");
}

export type HeadingState = number | true | null;

// The ATX marker is the parser's to find (I-12): `header.heading` is the `#` count lsdoc accepted for
// the block's first line, so `#tag`, a `#` run inside prose and a heading-looking line in a later
// line are never mistaken for one. Only the whitespace after the known-length marker is removed here.
const markdownHeadingLevel = (raw: string): number =>
  parserReady() && raw.includes("#") ? blockRegions(raw, "md").header.heading ?? 0 : 0;
const afterMarkdownHeading = (raw: string, level: number): string => raw.slice(level).replace(/^[ \t]+/, "");
const clearMarkdownHeading = (raw: string): string => {
  const level = markdownHeadingLevel(raw);
  return level ? afterMarkdownHeading(raw, level) : raw;
};
const setMarkdownHeading = (raw: string, level: number): string => {
  const prefix = `${"#".repeat(level)} `;
  const current = markdownHeadingLevel(raw);
  return current ? prefix + afterMarkdownHeading(raw, current) : prefix + raw.trimStart();
};

/** Pure format-aware heading transition shared by single-block and selection
 * commands so their Markdown/Org serialization cannot drift apart. */
export function rawWithHeading(raw: string, format: Format, state: HeadingState): string {
  const level = typeof state === "number" && state >= 1 && state <= 6 ? state : null;
  if (format === "org") {
    return orgRawWithProperty(raw, "heading", state === true ? "true" : level === null ? null : String(level));
  }
  if (state === true) return markdownRawWithProperty(clearMarkdownHeading(raw), "heading", "true");
  if (level !== null) return setMarkdownHeading(markdownRawWithProperty(raw, "heading", null), level);
  return markdownRawWithProperty(clearMarkdownHeading(raw), "heading", null);
}

/** Switch between boolean automatic headings and explicit numeric headings.
 * Markdown writes ATX prefixes for numeric state and `heading:: true` for auto;
 * Org writes both states through its property drawer. Each transition clears the
 * incompatible representation. OG parity:
 * `src/main/frontend/handler/editor.cljs:3822-3862` and
 * `src/main/frontend/commands.cljs:623-638` at `6e7afa8eb`; the format-aware
 * property writer is `handler/editor.cljs:888-904` at the same commit. */
export function setHeading(id: string, state: HeadingState) {
  const node = doc.byId[id];
  if (!node || !blockWritable(id)) return;
  const next = rawWithHeading(node.raw, formatForBlock(id), state);
  if (next === node.raw) return;
  pushUndo(`heading:${id}`, [node.page]);
  setDoc("byId", id, "raw", next);
  markDirty(node.page, "save-block");
}

const pad2 = (n: number) => String(n).padStart(2, "0");

/** Read a block's SCHEDULED/DEADLINE date as {y,m,d} (m 0-based), or null. */
/** Normalize an org time token to zero-padded `HH:mm`. mldoc/OG accept an
 *  unpadded hour (`9:05`) and drop seconds; we canonicalize to `09:05` so a
 *  native `<input type="time">` can pre-fill it and the on-disk form matches OG's
 *  rendered (zero-padded) canonical form. Returns null if it isn't `H:mm`. */
function normalizeHHmm(t: string): string | null {
  const m = /^(\d{1,2}):(\d{2})/.exec(t);
  if (!m) return null;
  return `${pad2(+m[1])}:${m[2]}`;
}

export function readSchedule(
  id: string,
  which: "scheduled" | "deadline"
): { y: number; m: number; d: number; time: string | null; repeater: string | null } | null {
  const node = doc.byId[id];
  if (!node) return null;
  const kind = which === "scheduled" ? "Scheduled" : "Deadline";
  const p = blockRegions(node.raw, formatForBlock(id)).planning.find(p => p.kind === kind);
  if (!p) return null;
  return scheduleParts(p.date);
}

/** Set or clear a block's SCHEDULED/DEADLINE org-timestamp (line 2, like OG).
 *  `repeater` is an org recurrence cookie (`+1w`, `.+1w`, `++1w`) or null; `time`
 *  is a `HH:mm` clock time or null. Both are written inside the `<…>` in OG's fixed
 *  order — `<yyyy-MM-dd EEE[ HH:mm][ repeater]>` — the repeater is consumed by
 *  repeat.ts on completion. */
export function setSchedule(
  id: string,
  which: "scheduled" | "deadline",
  date: { y: number; m: number; d: number } | null,
  repeater?: string | null,
  time?: string | null
) {
  const node = doc.byId[id];
  if (!node || !blockWritable(id)) return;
  const whichKind = which === "scheduled" ? "Scheduled" : "Deadline";
  let value: string | null = null;
  if (date) {
    const hhmm = time ? normalizeHHmm(time) : null;
    value = planningTimestamp({...date, time:hhmm, repeater});
  }
  let raw: string;
  try { raw = editBlock(node.raw, formatForBlock(id), {kind:"planning",which:whichKind,value}); }
  catch (e) { pushToast(String(e), "error"); return; }
  if (raw === node.raw) return;
  pushUndo(`sched:${id}:${which}`, [node.page]);
  setDoc("byId", id, "raw", raw);
  markDirty(node.page, "save-block");
}

/** A block's raw with `collapsed:: true` added or removed so the persisted
 *  property matches the collapsed state. OG stores collapse in the file as a
 *  block property, so mirroring it here makes a collapse survive a relaunch and
 *  show up collapsed in OG / the mobile app. Fence-aware via splitProps. */
export function rawWithCollapsed(raw: string, collapsed: boolean, format: Format): string {
  if (format === "org") return orgRawWithProperty(raw, "collapsed", collapsed ? "true" : null);
  const { visible, hidden } = splitProps(raw, isBuiltinHidden, format);
  const nextHidden = upsertPropertyLine(hidden, "collapsed", collapsed ? "true" : null) ?? "";
  return joinProps(visible, nextHidden, format);
}

/** Set a block's collapsed state AND mirror it into its raw `collapsed::` so it
 *  persists — the on-disk markdown is the source of truth on the next load. */
export function writeCollapsed(id: string, collapsed: boolean) {
  const n = doc.byId[id];
  if (!n || !blockWritable(id)) return;
  const nextRaw = rawWithCollapsed(n.raw, collapsed, formatForBlock(id));
  setDoc("byId", id, "collapsed", collapsed);
  if (nextRaw !== n.raw) setDoc("byId", id, "raw", nextRaw);
  bumpCollapseEpochs([id]);
}

/** Expand every collapsed ancestor of `id` so the block itself renders, as one
 *  undo step and a persisted edit (like expanding by hand: `collapsed::` is on
 *  disk). Returns true if anything changed; false when nothing is collapsed
 *  above it or any collapsed ancestor is read-only. A collapsed parent renders
 *  no children, so navigating to a hidden block otherwise never reveals it
 *  (GH #258). Cost O(depth). */
export function expandAncestors(id: string): boolean {
  const target = doc.byId[id];
  if (!target) return false;
  const collapsedAncestors: string[] = [];
  let parent = target.parent;
  while (parent !== null && parent !== undefined) {
    const node = doc.byId[parent];
    if (!node) break;
    if (node.collapsed) collapsedAncestors.push(parent);
    parent = node.parent;
  }
  if (collapsedAncestors.length === 0) return false;
  if (!collapsedAncestors.every((ancestor) => blockWritable(ancestor))) return false;
  pushUndo("reveal-block", [target.page]);
  for (const ancestor of collapsedAncestors) writeCollapsed(ancestor, false);
  markDirty(target.page, "save-block");
  return true;
}

/** Collapse or expand a block and its entire descendant subtree. */
export function setCollapsedDeep(id: string, collapsed: boolean) {
  if (!blockWritable(id)) return;
  pushUndo("collapse-all", [doc.byId[id].page]);
  const walk = (bid: string) => {
    const n = doc.byId[bid];
    if (!n) return;
    if (n.children.length) writeCollapsed(bid, collapsed);
    n.children.forEach(walk);
  };
  walk(id);
  markDirty(doc.byId[id].page, "save-block");
}

/** Every descendant that can itself be folded, including descendants hidden by
 * a collapsed ancestor. Iterative model traversal avoids both DOM dependence and
 * call-stack growth on a deeply nested outline. The guide's own block is excluded. */
export function collapsibleDescendantIds(id: string): string[] {
  const root = doc.byId[id];
  if (!root) return [];
  const result: string[] = [];
  const stack = [...root.children].reverse();
  while (stack.length) {
    const childId = stack.pop()!;
    const child = doc.byId[childId];
    if (!child) continue;
    if (child.children.length) result.push(childId);
    for (let i = child.children.length - 1; i >= 0; i--) stack.push(child.children[i]);
  }
  return result;
}

/** Persist one collapse value across every collapsible descendant, but never the
 * guide parent itself. One snapshot + one store transaction makes the operation
 * one Undo step and avoids a reactive update per node on large subtrees. */
export function setCollapsedDescendants(id: string, collapsed: boolean) {
  const root = doc.byId[id];
  if (!root || !blockWritable(id)) return;
  const changes = collapsibleDescendantIds(id)
    .map((childId) => {
      const child = doc.byId[childId];
      if (!child || child.collapsed === collapsed) return null;
      return {
        id: childId,
        raw: rawWithCollapsed(child.raw, collapsed, formatForBlock(childId)),
      };
    })
    .filter((change): change is { id: string; raw: string } => change !== null);
  if (!changes.length) return;
  pushUndo("collapse-descendants", [root.page]);
  setDoc(
    produce((state) => {
      for (const change of changes) {
        const child = state.byId[change.id];
        if (!child) continue;
        child.collapsed = collapsed;
        child.raw = change.raw;
      }
    })
  );
  bumpCollapseEpochs(changes.map((change) => change.id));
  markDirty(root.page, "save-block");
}
