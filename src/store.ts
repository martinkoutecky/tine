// The live editing tree. The frontend owns this during a session; all
// keystrokes and structural ops mutate it synchronously (zero IPC). Persistence
// is a debounced per-page save to Rust. See plan §"block editor model".
//
// Supports multiple pages at once (the journals feed): a single global `byId`
// map, each node tagged with its owning `page`, and an ordered `pages` list
// each with its own roots. A single-page route is just a feed of length one.

import { createStore, produce, unwrap } from "solid-js/store";
import { createSignal, createMemo, createRoot } from "solid-js";
import type { BlockDto, Format, PageDto, PageKind } from "./types";
import { parseOutline, type OutlineNode } from "./editor/outline";
import type { ExportNode } from "./editor/exportText";
import { backend } from "./backend";
import { isConflicted, clearConflict, rightSidebar, conflicts, pushToast } from "./ui";
import { blockView } from "./render/block";
import { journalTitle } from "./journal";
import { upsertPropertyLine, readPropertyValue, splitProps, joinProps, isBuiltinHidden } from "./editor/properties";
import { copyIncludeSubtree, copyStripCollapsed } from "./copySettings";
import { trimBlockTrailingSpace } from "./editor/format";
import {
  markDirty,
  isDirty,
  scheduleSave,
  flushPage,
  flushAll,
  forceSave,
  addDirty,
  dirtyPages,
  setBaseRev,
  tombstone,
  untombstone,
  forgetSaveState,
  resetSaveState,
  isSaving,
} from "./persistence";
// The debounced persistence engine lives in persistence.ts; re-exported here so
// the rest of the app keeps importing the save API from the store.
export { markDirty, isDirty, scheduleSave, flushPage, flushAll, forceSave };

export interface Node {
  id: string;
  raw: string;
  collapsed: boolean;
  parent: string | null; // null = a root of its page
  page: string; // owning page name
  children: string[];
}

export interface FeedPage {
  name: string;
  kind: PageKind;
  title: string;
  preBlock: string | null;
  roots: string[];
  /** On-disk format (drives org vs markdown inline rendering). */
  format: Format;
  /** True for an org page Tine can't round-trip — shown but not editable. */
  readOnly: boolean;
  /** Graph-root-relative file this page was loaded from. Sent back on save so a
   *  page pinned to a SPECIFIC file (a duplicate-day stray, #21) saves to its own
   *  file, not the canonical one. Empty/absent for a brand-new page (resolved by
   *  name). */
  path?: string;
}

interface DocState {
  byId: Record<string, Node>;
  // The working set: every page currently loaded in the frontend — the main
  // view's pages PLUS any page a satellite surface (sidebar, query result,
  // embed) has pulled in on demand. All share one `byId` keyed by stable block
  // uuid, so a block rendered in two places is the SAME node and edits to it
  // propagate everywhere via SolidJS reactivity (OG's "everything is a block",
  // adapted to lazy loading — the Rust cache is the full graph DB).
  pages: FeedPage[];
  // Page names the MAIN content area shows, in order (a single page, or the
  // journals feed). A subset of `pages`.
  feed: string[];
  loaded: boolean;
}

export const [doc, setDoc] = createStore<DocState>({ byId: {}, pages: [], feed: [], loaded: false });

/** The pages shown in the main content area, in feed order. */
export function mainPages(): FeedPage[] {
  return doc.feed.map((n) => doc.pages.find((p) => p.name === n)).filter(Boolean) as FeedPage[];
}

/** A loaded page record by name (anywhere in the working set), or undefined. */
export function pageByName(name: string): FeedPage | undefined {
  return doc.pages.find((p) => p.name === name);
}

export const [editingId, setEditingId] = createSignal<string | null>(null);
// Which on-screen <Block> instance owns the active editor. One block uuid can
// render in several places at once (main view + sidebar + query result); without
// this they'd all mount a textarea for the same node and fight over its value.
// null = unscoped (e.g. keyboard nav) — any instance of editingId may edit.
export const [editingOwner, setEditingOwner] = createSignal<string | null>(null);
const [caretTarget, setCaretTarget] = createSignal<{ id: string; offset: number } | null>(null);

export function takeCaretFor(id: string): number | null {
  const t = caretTarget();
  if (t && t.id === id) {
    setCaretTarget(null);
    return t.offset;
  }
  return null;
}

let idCounter = 0;
function freshId(): string {
  return `b${Date.now().toString(36)}-${idCounter++}`;
}

// ---------------------------------------------------------------------------
// Loading / serializing
// ---------------------------------------------------------------------------

function flatten(
  dtos: BlockDto[],
  parent: string | null,
  pageName: string,
  byId: Record<string, Node>
): string[] {
  return dtos.map((d) => {
    // Cross-page id:: collision guard: if another LOADED page already owns this
    // id (two files share a persisted `id::` — copy-pasted raw, or a sync hiccup),
    // give this block a fresh store key instead of overwriting the other page's
    // node. Without this, the global byId entry is clobbered and saving one page
    // serializes the other's content. The block's raw (incl. its id:: line) is
    // untouched, so the file on disk is unchanged. Rust dedups ids WITHIN a page,
    // so this only fires across pages.
    const existing = byId[d.id];
    const key = existing && existing.page !== pageName ? `dup~${crypto.randomUUID()}` : d.id;
    const childIds = flatten(d.children, key, pageName, byId);
    byId[key] = {
      id: key,
      raw: d.raw,
      collapsed: d.collapsed,
      parent,
      page: pageName,
      children: childIds,
    };
    return key;
  });
}

function toFeedPage(dto: PageDto, byId: Record<string, Node>): FeedPage {
  const roots = flatten(dto.blocks, null, dto.name, byId);
  return {
    name: dto.name,
    kind: dto.kind,
    title: dto.title,
    preBlock: dto.pre_block,
    roots,
    format: dto.format ?? "md",
    readOnly: dto.read_only ?? false,
    path: dto.path,
  };
}

function removeNodeSubtree(s: DocState, id: string) {
  const n = s.byId[id];
  if (!n) return;
  for (const c of n.children) removeNodeSubtree(s, c);
  delete s.byId[id];
}

/** Drop a page's blocks from the shared byId map (before replacing it). Walks the
 *  page's own root subtrees — O(page size) — rather than sweeping all of `byId`
 *  (which made loading K pages into an N-node feed O(K·N)). */
function purgePageNodes(s: DocState, pageName: string) {
  const page = s.pages.find((p) => p.name === pageName);
  if (!page) return;
  for (const r of page.roots) removeNodeSubtree(s, r);
}

/** Merge a page into the working set, replacing any prior copy of that page.
 *  Other loaded pages (and their nodes) are left untouched — so a page open in
 *  the sidebar survives navigating the main view elsewhere. */
function upsertPage(dto: PageDto) {
  // A real page with this name exists again → lift any delete tombstone so edits
  // to the freshly-(re)created page save normally.
  untombstone(dto.name);
  const existing = doc.pages.find((p) => p.name === dto.name);
  // Self-write echo: the watcher re-reported our OWN just-saved content (Tine's
  // save normally suppresses this, but a synced/polled graph or a self-write-marker
  // gap can still surface it). A reload here rebuilds the page AND calls
  // invalidateUndoForPage, which would drop the undo entry we just pushed for the
  // edit that produced this exact content — that's the "delete a line, Ctrl+Z does
  // nothing" bug. If the incoming content is identical to what we already have, just
  // refresh the save baseline and keep the working copy + undo intact. A GENUINE
  // external change (content differs) still reloads + invalidates (data-safety #42).
  if (existing && pageContentMatches(dto, existing)) {
    setBaseRev(dto.name, dto.rev ?? null);
    return;
  }
  // Replacing an already-loaded copy means the page's content changed under us
  // (a conflict-resolution / watcher reload). Any undo entry predating this reload
  // is stale — replaying it would clobber the just-loaded (external) version, so
  // drop those entries. (A first load has no prior entries → no-op.)
  const replacing = !!existing;
  // Record the load baseline (the on-disk rev) so saves conflict against it.
  setBaseRev(dto.name, dto.rev ?? null);
  setDoc(
    produce((s) => {
      purgePageNodes(s, dto.name);
      const fp = toFeedPage(dto, s.byId);
      const i = s.pages.findIndex((p) => p.name === dto.name);
      if (i >= 0) s.pages[i] = fp;
      else s.pages.push(fp);
    })
  );
  if (replacing) invalidateUndoForPage(dto.name);
}

/** Whether a reload DTO carries the SAME content (page-property pre-block + every
 *  block's raw + tree shape, ignoring block ids) as the page already in memory —
 *  i.e. a self-write echo, not a real external change. Lets `upsertPage` skip a
 *  needless reload that would otherwise reset block identities and invalidate the
 *  undo history for content we already hold. */
function pageContentMatches(dto: PageDto, page: FeedPage): boolean {
  if ((dto.pre_block ?? null) !== (page.preBlock ?? null)) return false;
  const eq = (b: BlockDto, id: string): boolean => {
    const n = doc.byId[id];
    if (!n || n.raw !== b.raw || n.children.length !== b.children.length) return false;
    return b.children.every((cb, i) => eq(cb, n.children[i]));
  };
  return dto.blocks.length === page.roots.length && dto.blocks.every((b, i) => eq(b, page.roots[i]));
}

/** Load a page into the working set if it isn't already there (used by
 *  satellite surfaces — sidebar / query results / embeds — so they render the
 *  same live, editable nodes as the main view). Idempotent: never clobbers an
 *  already-loaded page's in-progress edits. */
export function ensurePageLoaded(dto: PageDto) {
  if (doc.pages.some((p) => p.name === dto.name)) return;
  upsertPage(dto);
  evictIfNeeded();
}

/** Drop a page from the working set + feed and clear its dirty/baseline/conflict
 *  state — WITHOUT touching disk. Use when the page no longer exists on disk and
 *  the user accepts that (e.g. resolving an external-deletion conflict with "use
 *  disk version"): otherwise the unsaved in-memory copy is left untracked — not
 *  dirty, not conflicted — and is silently lost at close. */
export function forgetPage(name: string) {
  forgetSaveState(name);
  clearConflict(name);
  // The page is leaving the working set; a stale undo snapshot must not be able to
  // re-add it (and, with baseRev gone, recreate an externally-deleted file).
  invalidateUndoForPage(name);
  setDoc(
    produce((s) => {
      purgePageNodes(s, name);
      const pi = s.pages.findIndex((p) => p.name === name);
      if (pi >= 0) s.pages.splice(pi, 1);
      const fi = s.feed.indexOf(name);
      if (fi >= 0) s.feed.splice(fi, 1);
    })
  );
}

/** Delete a page: tombstone it (so any pending/in-flight save can't recreate the
 *  file), drop its dirty/baseline/conflict state, remove it from the working set
 *  and feed, then delete on disk. Routing deletion through the store — rather than
 *  calling the backend directly — is what prevents a queued baseRev=null save from
 *  resurrecting a just-typed, never-saved page. Returns backend success. */
export async function deletePage(name: string, kind: PageKind): Promise<boolean> {
  // Capture the current (possibly unsaved) content first, so the recoverable trash
  // copy is the LATEST version — not the stale bytes on disk. If it can't be saved
  // (an unresolved external conflict), abort rather than trash a stale file.
  if ((isDirty(name) || isConflicted(name)) && !(await flushPage(name))) return false;
  // Tombstone first so any queued/in-flight save no-ops during the delete, but
  // DON'T drop the in-memory page until the backend actually deletes it — if the
  // delete fails, the page (and its unsaved edits) must survive.
  tombstone(name);
  try {
    await backend().deletePage(name, kind);
  } catch {
    untombstone(name); // delete failed — lift the tombstone; page + edits stay intact
    return false;
  }
  forgetPage(name); // success — now drop it from the working set + feed
  return true;
}

// Cap the working set so a long session browsing a big graph doesn't grow byId
// without bound. FIFO-evict pages that aren't pinned: the main feed, anything
// open in the right sidebar, the page being edited, and any page with unsaved
// edits are all kept (evicting a dirty page would lose those edits).
const WORKING_SET_CAP = 80;
function pinnedPages(): Set<string> {
  const pin = new Set<string>(doc.feed);
  for (const it of rightSidebar()) pin.add(it.kind === "page" ? it.name : it.page);
  for (const name of dirtyPages()) pin.add(name);
  // Conflicted pages hold unsaved edits that aren't in `dirty` (the save batch
  // removed them); evicting one would silently drop those edits.
  for (const name of conflicts()) pin.add(name);
  const ed = editingId();
  if (ed && doc.byId[ed]) pin.add(doc.byId[ed].page);
  return pin;
}

/** Replace a page in the working set from a fresh DTO (e.g. resolving a conflict
 *  with the disk version, or a watcher reload). Updates the main view and any
 *  satellite that shows it, since they share `byId`. */
export function reloadPage(dto: PageDto) {
  upsertPage(dto);
}

/** After a PDF highlight write changed an `hls__` page on disk, refresh its
 *  loaded copy (main view or sidebar) so its content AND save baseline (baseRev)
 *  track disk — otherwise a later editor save would conflict against the highlight
 *  write. Skips a page with unsaved edits / an open conflict: the caller flushes
 *  those FIRST so they're on disk and merged in, rather than clobbered here. */
export async function reloadHlsIfLoaded(name: string): Promise<void> {
  if (!pageByName(name)) return;
  if (isDirty(name) || isConflicted(name)) return;
  const dto = await backend().getPage(name, "page");
  if (dto) reloadPage(dto);
}
function evictIfNeeded() {
  if (doc.pages.length <= WORKING_SET_CAP) return;
  const pin = pinnedPages();
  setDoc(
    produce((s) => {
      // Oldest first (insertion order); stop once at the cap or only pinned left.
      for (let i = 0; i < s.pages.length && s.pages.length > WORKING_SET_CAP; ) {
        const name = s.pages[i].name;
        if (pin.has(name)) {
          i++;
          continue;
        }
        purgePageNodes(s, name);
        s.pages.splice(i, 1);
      }
    })
  );
}

/** Clear the entire working set. Used for test isolation and when switching
 *  graphs; normal navigation is additive (keeps satellite pages alive). Also
 *  cancels pending saves and clears dirty flags so nothing from the old graph
 *  can be written after a switch. */
export function resetStore() {
  // Cancel pending/in-flight saves and clear all save guard state (timers, graph
  // token, dirty/baseline/tombstone) so nothing from the old graph can be written
  // after the switch.
  resetSaveState();
  // Drop undo/redo history: it holds page snapshots from the OLD graph; an undo
  // after a graph switch would otherwise restore (and save) those into the new
  // graph, even creating a foreign page there.
  clearUndoHistory();
  setDoc({ byId: {}, pages: [], feed: [], loaded: false });
  setEditingId(null);
}

// A navigation/feed load must NOT replace a page that has unsaved edits (or an
// unresolved conflict) with a fresh disk DTO — e.g. you edited it in the sidebar,
// then opened it in the main view before the debounce saved. Keep the live dirty
// nodes; the disk version would otherwise be served and the next save could write
// it, silently dropping the edit. (reloadPage / "use disk version" still replace
// explicitly via upsertPage.)
function upsertUnlessDirty(dto: PageDto) {
  if (pageByName(dto.name) && (isDirty(dto.name) || isConflicted(dto.name))) return;
  upsertPage(dto);
}

/** Load a single page and make it the main view. */
export function loadSingle(dto: PageDto) {
  upsertUnlessDirty(dto);
  setDoc("feed", [dto.name]);
  setDoc("loaded", true);
  setEditingId(null);
  evictIfNeeded();
}

/** Load the journals feed as the main view. */
export function loadFeed(dtos: PageDto[]) {
  for (const d of dtos) upsertUnlessDirty(d);
  setDoc("feed", dtos.map((d) => d.name));
  setDoc("loaded", true);
  setEditingId(null);
  evictIfNeeded();
}

/** Append more pages to the journals feed (infinite scroll). */
export function appendFeed(dtos: PageDto[]) {
  for (const d of dtos) {
    if (doc.feed.includes(d.name)) continue;
    upsertUnlessDirty(d);
    setDoc("feed", [...doc.feed, d.name]);
  }
  evictIfNeeded();
}

function toDto(id: string): BlockDto {
  const n = doc.byId[id];
  // Trim a block's trailing space only here, at the disk-write boundary — OG
  // keeps the space while you edit and trims on save. (The live editor buffer
  // keeps it so backspacing to a trailing space doesn't eat the space out from
  // under the caret.) `trimBlockTrailingSpace` is idempotent and only touches
  // whitespace at the very end of the block, so a block with nothing to trim
  // serializes byte-identically — no churn, no property reordering.
  return { id: n.id, raw: trimBlockTrailingSpace(n.raw), collapsed: n.collapsed, children: n.children.map(toDto) };
}

export function pageToDto(pageName: string): PageDto | null {
  const p = doc.pages.find((x) => x.name === pageName);
  if (!p) return null;
  let blocks = p.roots.map(toDto);
  // Don't persist a lone placeholder block. A page that exists only for its
  // properties is loaded with one empty editable bullet (toLoadable); saving it
  // — e.g. after a page-property edit — must NOT write that bullet back as a
  // stray "- " and corrupt the round-trip. Symmetric with the load side;
  // reopening re-adds the editable bullet.
  if (blocks.length === 1 && blocks[0].raw.trim() === "" && blocks[0].children.length === 0) {
    blocks = [];
  }
  return {
    name: p.name,
    kind: p.kind,
    title: p.title,
    pre_block: p.preBlock,
    blocks,
    format: p.format,
    // Pin the save to the exact file this page came from (#21). Absent for a
    // brand-new page → the backend resolves the file by name, as before.
    path: p.path,
  };
}

// ---------------------------------------------------------------------------
// Tree helpers
// ---------------------------------------------------------------------------

function rootsOf(id: string): string[] {
  const n = doc.byId[id];
  if (n.parent !== null) return doc.byId[n.parent].children;
  const p = doc.pages.find((x) => x.name === n.page);
  return p ? p.roots : [];
}

function indexInSiblings(id: string): number {
  return rootsOf(id).indexOf(id);
}

/** Visible blocks in the MAIN view, in display order (drives editor arrow-nav),
 *  plus an id→index map. Memoized: it's recomputed only when the feed or a
 *  collapsed/children state changes (NOT on plain typing), and shared across the
 *  many callers in one tick. Scoped to the feed so navigation stays within the
 *  main content area, not satellite pages loaded for the sidebar/queries. */
const visibleData = createRoot(() =>
  createMemo(() => {
    const order: string[] = [];
    const index = new Map<string, number>();
    const walk = (ids: string[]) => {
      for (const id of ids) {
        index.set(id, order.length);
        order.push(id);
        const n = doc.byId[id];
        if (n && !n.collapsed && n.children.length) walk(n.children);
      }
    };
    for (const p of mainPages()) walk(p.roots);
    return { order, index };
  })
);
export function visibleOrder(): string[] {
  return visibleData().order;
}

// Visible (expanded) block order within a single page — the fallback for blocks
// that aren't part of the main routed view, e.g. the quick-capture scratch page,
// whose roots never appear in mainPages(). Without this, prevVisible/nextVisible
// (and therefore Backspace-merge and Up/Down nav) are dead in the capture window.
function pageVisibleOrder(pageName: string): string[] {
  const order: string[] = [];
  const page = doc.pages.find((p) => p.name === pageName);
  if (!page) return order;
  const walk = (ids: string[]) => {
    for (const id of ids) {
      order.push(id);
      const n = doc.byId[id];
      if (n && !n.collapsed && n.children.length) walk(n.children);
    }
  };
  walk(page.roots);
  return order;
}

export function prevVisible(id: string): string | null {
  const { order, index } = visibleData();
  const i = index.get(id);
  if (i !== undefined) return i > 0 ? order[i - 1] : null;
  const node = doc.byId[id];
  if (!node) return null;
  const ord = pageVisibleOrder(node.page);
  const j = ord.indexOf(id);
  return j > 0 ? ord[j - 1] : null;
}

export function nextVisible(id: string): string | null {
  const { order, index } = visibleData();
  const i = index.get(id);
  if (i !== undefined) return i < order.length - 1 ? order[i + 1] : null;
  const node = doc.byId[id];
  if (!node) return null;
  const ord = pageVisibleOrder(node.page);
  const j = ord.indexOf(id);
  return j >= 0 && j < ord.length - 1 ? ord[j + 1] : null;
}

export function depthOf(id: string): number {
  let d = 0;
  let p = doc.byId[id]?.parent ?? null;
  while (p !== null) {
    d++;
    p = doc.byId[p].parent;
  }
  return d;
}

// ---------------------------------------------------------------------------
// Undo / redo (snapshot-based; typing in one block coalesces to one step)
// ---------------------------------------------------------------------------

// A page-scoped structural snapshot, or a single-block raw patch (typing).
// Typing is by far the most frequent op, so it records an O(1) inverse instead
// of cloning anything. A structural op snapshots ONLY the pages it touches (its
// nodes + page objects), so the cost is O(edited page), not O(whole working set)
// — a structural edit no longer slows down as more journal days / sidebar / query
// pages get loaded. `pages: null` means "all loaded pages" (the safe fallback for
// an op that can't declare its scope).
interface SnapEntry {
  kind: "snap";
  pages: string[] | null; // affected page names (null = whole working set)
  pageObjs: FeedPage[]; // snapshot of those pages' FeedPage objects
  nodes: Record<string, Node>; // snapshot of nodes living on those pages
  dirty: string[]; // pages to re-save on undo/redo
}
interface RawEntry {
  kind: "raw";
  id: string;
  raw: string; // the block's text to restore
  page: string;
}
type UndoEntry = SnapEntry | RawEntry;
const undoStack: UndoEntry[] = [];
let redoStack: UndoEntry[] = [];
let lastUndoTag: string | null = null;

/** Discard all undo/redo history. Called on graph switch/reset so old-graph
 *  snapshots can't be replayed into a different graph. */
export function clearUndoHistory() {
  undoStack.length = 0;
  redoStack = [];
  lastUndoTag = null;
}

/** Does an undo entry reference page `name`? A raw entry by its `page`; a snap
 *  entry by its declared scope (a `null` scope = whole working set, so it touches
 *  every page including this one). */
function entryTouchesPage(e: UndoEntry, name: string): boolean {
  if (e.kind === "raw") return e.page === name;
  return e.pages === null || e.pages.includes(name);
}

/** Drop undo/redo entries that reference `name`. Called when a page's on-disk
 *  content is reloaded under us (external edit → new baseRev) or the page is
 *  forgotten/deleted: a snapshot taken before that reload is stale, and replaying
 *  it would mark the page dirty and let autosave overwrite the external version —
 *  or, for a forgotten/deleted page, resurrect the file. We drop the whole entry
 *  (not just the page's slice) because a snap can't be partially applied; this can
 *  cost an unrelated co-snapshotted page its undo step, which is the safe tradeoff
 *  (lose an undo vs. clobber a file). */
export function invalidateUndoForPage(name: string) {
  for (let i = undoStack.length - 1; i >= 0; i--) {
    if (entryTouchesPage(undoStack[i], name)) undoStack.splice(i, 1);
  }
  redoStack = redoStack.filter((e) => !entryTouchesPage(e, name));
  lastUndoTag = null; // don't coalesce a later edit onto a now-dropped entry
}

// Hand-rolled clones — Node/FeedPage are flat (primitives + a string[]), so a
// tailored copy is far cheaper than structuredClone (which probes types and
// walks for cycles). This runs on EVERY structural op (split/merge/indent/move/
// delete) for undo, so its cost is felt as general editor latency.
function cloneNode(n: Node): Node {
  return {
    id: n.id,
    raw: n.raw,
    collapsed: n.collapsed,
    parent: n.parent,
    page: n.page,
    children: n.children.slice(),
  };
}
function clonePages(src: FeedPage[]): FeedPage[] {
  return src.map((p) => ({
    name: p.name,
    kind: p.kind,
    title: p.title,
    preBlock: p.preBlock,
    roots: p.roots.slice(),
    format: p.format,
    readOnly: p.readOnly,
  }));
}
function snapEntry(affected?: string[] | null): SnapEntry {
  // null/omitted → snapshot the whole working set (safe fallback). Otherwise just
  // the named pages: their FeedPage objects + every node living on them.
  const names = affected ?? doc.pages.map((p) => p.name);
  const nameSet = new Set(names);
  const byId = unwrap(doc.byId);
  const pages = unwrap(doc.pages);
  const nodes: Record<string, Node> = {};
  // Collect each affected page's nodes by walking its root subtrees — O(nodes on
  // those pages), NOT O(whole loaded working set). A consistent pre-op tree has
  // every node-with-page-P reachable from P's roots (same invariant
  // purgePageNodes relies on), so this captures exactly the by-page set without
  // sweeping byId as sidebars/old journal days/query results accumulate.
  const visit = (id: string) => {
    const n = byId[id];
    if (!n || nodes[id]) return;
    nodes[id] = cloneNode(n);
    for (const c of n.children) visit(c);
  };
  for (const p of pages) {
    if (nameSet.has(p.name)) for (const r of p.roots) visit(r);
  }
  const pageObjs = clonePages(pages.filter((p) => nameSet.has(p.name)));
  return { kind: "snap", pages: affected ?? null, pageObjs, nodes, dirty: names };
}

/** Snapshot before a STRUCTURAL op. Pass the affected page name(s) so both the
 *  snapshot AND the undo re-save are scoped to just those pages; omit only when
 *  the op's page set isn't known (falls back to the whole working set — correct
 *  but O(loaded pages)). The affected set MUST include every page whose nodes the
 *  op changes, including a cross-page move's source AND destination, or undo
 *  would miss a page. `tag` resets the typing-coalesce marker. */
function pushUndo(tag: string, affected?: string[]) {
  undoStack.push(snapEntry(affected));
  if (undoStack.length > 200) undoStack.shift();
  redoStack = [];
  lastUndoTag = tag;
}

/** Record an O(1) inverse patch for a single-block text edit (typing). A typing
 *  burst in one block coalesces to a single entry holding the pre-burst text. */
function pushRawUndo(id: string, prevRaw: string) {
  const tag = `type:${id}`;
  if (tag === lastUndoTag) return; // mid-burst: keep the first (pre-burst) raw
  undoStack.push({ kind: "raw", id, raw: prevRaw, page: doc.byId[id].page });
  if (undoStack.length > 200) undoStack.shift();
  redoStack = [];
  lastUndoTag = tag;
}

/** Apply one entry and return its inverse (to push onto the opposite stack). */
function applyEntry(e: UndoEntry): UndoEntry {
  if (e.kind === "raw") {
    const node = doc.byId[e.id];
    const inverse: RawEntry = { kind: "raw", id: e.id, raw: node ? node.raw : "", page: e.page };
    if (node) {
      setDoc("byId", e.id, "raw", e.raw);
      addDirty(e.page);
    }
    return inverse;
  }
  // Capture the CURRENT state of the same page scope as the inverse (for redo).
  const inverse = snapEntry(e.pages);
  if (e.pages === null) {
    // Whole-working-set snapshot (fallback): replace byId + pages wholesale so the
    // store is always internally consistent. (A page loaded AFTER the snapshot is
    // dropped cleanly rather than left with dangling roots — but every op that can
    // touch multiple pages now declares its scope, so this path is a last resort.)
    setDoc(
      produce((s) => {
        const nodes: Record<string, Node> = {};
        for (const id in e.nodes) nodes[id] = cloneNode(e.nodes[id]);
        s.byId = nodes;
        s.pages = e.pageObjs.map((po) => clonePages([po])[0]);
      })
    );
  } else {
    // Scoped restore: touch ONLY the affected pages, so pages loaded/edited
    // concurrently on OTHER pages are left intact.
    const scope = e.pages;
    setDoc(
      produce((s) => {
        // Drop the affected pages' CURRENT nodes (incl. ones the op added) by
        // walking their current root subtrees — O(affected page sizes), not a
        // full byId sweep. Then reinstate the snapshot. (Same root-walk
        // purgePageNodes uses for upsert/forget.)
        for (const name of scope) purgePageNodes(s, name);
        for (const id in e.nodes) s.byId[id] = cloneNode(e.nodes[id]); // reinstate the snapshot
        for (const po of e.pageObjs) {
          const restored = clonePages([po])[0];
          const i = s.pages.findIndex((p) => p.name === po.name);
          if (i >= 0) s.pages[i] = restored;
          else s.pages.push(restored);
        }
      })
    );
  }
  for (const p of e.dirty) addDirty(p);
  return inverse;
}

export function undo() {
  if (!undoStack.length) return;
  redoStack.push(applyEntry(undoStack.pop()!));
  lastUndoTag = null;
  setEditingId(null);
  scheduleSave();
}

export function redo() {
  if (!redoStack.length) return;
  undoStack.push(applyEntry(redoStack.pop()!));
  lastUndoTag = null;
  setEditingId(null);
  scheduleSave();
}

// ---------------------------------------------------------------------------
// Mutations (each schedules a debounced save of the affected page)
// ---------------------------------------------------------------------------

export function setRaw(id: string, raw: string) {
  pushRawUndo(id, doc.byId[id].raw);
  setDoc("byId", id, "raw", raw);
  markDirty(doc.byId[id].page);
}

// Surface-aware edit focus. A block uuid can render in SEVERAL surfaces at once
// (the main pane and each right-sidebar item). `activeSurface` tracks which
// surface's editor currently holds the caret (set on textarea focus). An UNSCOPED
// edit (owner=null — a split or keyboard nav) mounts an editor in EVERY surface
// that renders the new block, and each one's onMount would call focus(); without
// arbitration the main pane wins and the caret "disappears" out of the sidebar.
// So we stamp the new block id with the surface that had the caret, and only that
// surface focuses it (see Block's onMount). Scoped edits (owner set on click) only
// ever mount one instance, so they need no stamp.
export const [activeSurface, setActiveSurface] = createSignal<string | null>(null);
const pendingFocusSurface = new Map<string, string>();
/** The surface that should take the caret for `id`, or undefined for "no
 *  constraint" (single-surface edit → focus normally). */
export function focusSurfaceFor(id: string): string | undefined {
  return pendingFocusSurface.get(id);
}
export function clearFocusSurface(id: string) {
  pendingFocusSurface.delete(id);
}

export function startEditing(id: string, offset: number, owner: string | null = null) {
  setSelAnchor(null);
  setSelFocus(null);
  setCaretTarget({ id, offset });
  if (owner === null) {
    // Unscoped: pin the caret to the surface that currently has it, so the new
    // block doesn't get its focus stolen by another surface rendering the same id.
    const s = activeSurface();
    if (s) pendingFocusSurface.set(id, s);
    else pendingFocusSurface.delete(id);
  } else {
    // Scoped to one rendered instance (a click) → exactly one editor mounts; drop
    // any stale stamp so it focuses immediately.
    pendingFocusSurface.delete(id);
  }
  setEditingId(id);
  setEditingOwner(owner);
}

/** Enter: split the block at `offset`. Built-in `id::`/`collapsed::` props are
 *  hidden from the editor (see editor/properties splitProps): the caret offset is
 *  in visible space, and hidden props stay with the ORIGINAL block across a split. */
export function splitBlock(id: string, offset: number) {
  pushUndo("split", [doc.byId[id].page]);
  const node = doc.byId[id];
  // The caret offset is in editor-visible space (hidden props aren't shown), so
  // split the visible text and keep the hidden props on the original block.
  const { visible, hidden } = splitProps(node.raw, isBuiltinHidden);
  const before = visible.slice(0, offset);
  const after = visible.slice(offset);
  const pageName = node.page;
  // Ordered-list items propagate: a block split off an ordered item is itself
  // ordered (OG inherits `:logseq.order-list-type`), toggleable per-block later.
  const ordered = isOrdered(id);
  const orderedAfter = ordered ? joinProps(after, `${ORDER_KEY}:: number`) : after;
  const orderedEmpty = ordered ? `${ORDER_KEY}:: number` : "";

  // Caret-at-start case (blank before, content after): create a NEW EMPTY block
  // *before* the current one. The current block keeps its uuid, its content, and
  // its children — its identity never changes. This mirrors OG's
  // insert-new-block-before-block-aux! and is what keeps a block stable when it's
  // shown elsewhere (sidebar / ref / query) and you press Enter at its head.
  // Without it, the content would migrate to a fresh uuid and any external view
  // tracking the original uuid would land on the now-empty block.
  if (before.trim() === "" && after.trim() !== "") {
    const emptyId = freshId();
    setDoc(
      produce((s) => {
        s.byId[emptyId] = {
          id: emptyId, raw: orderedEmpty, collapsed: false, parent: node.parent, page: pageName, children: [],
        };
        const sibs = node.parent === null
          ? s.pages[s.pages.findIndex((p) => p.name === pageName)].roots
          : s.byId[node.parent].children;
        sibs.splice(sibs.indexOf(id), 0, emptyId);
      })
    );
    startEditing(emptyId, 0);
    markDirty(pageName);
    return;
  }

  const newId = freshId();

  setDoc(
    produce((s) => {
      s.byId[id].raw = joinProps(before, hidden);
      const hasVisibleChildren = node.children.length > 0 && !node.collapsed;
      if (hasVisibleChildren) {
        s.byId[newId] = {
          id: newId, raw: orderedAfter, collapsed: false, parent: id, page: pageName, children: [],
        };
        s.byId[id].children.unshift(newId);
      } else {
        s.byId[newId] = {
          id: newId, raw: orderedAfter, collapsed: false, parent: node.parent, page: pageName, children: [],
        };
        const sibs = node.parent === null
          ? s.pages[s.pages.findIndex((p) => p.name === pageName)].roots
          : s.byId[node.parent].children;
        sibs.splice(sibs.indexOf(id) + 1, 0, newId);
      }
    })
  );
  startEditing(newId, 0);
  markDirty(pageName);
}

/** Tab: make the block the last child of its previous sibling. */
export function indentBlock(id: string, caretOffset: number) {
  const i = indexInSiblings(id);
  if (i <= 0) return;
  pushUndo("indent", [doc.byId[id].page]);
  const sibs = rootsOf(id);
  const newParent = sibs[i - 1];
  const pageName = doc.byId[id].page;
  setDoc(
    produce((s) => {
      const arr = s.byId[id].parent === null
        ? s.pages[s.pages.findIndex((p) => p.name === pageName)].roots
        : s.byId[s.byId[id].parent!].children;
      arr.splice(arr.indexOf(id), 1);
      s.byId[id].parent = newParent;
      s.byId[newParent].children.push(id);
      // Expand the new parent — and clear any persisted collapsed:: in its raw,
      // else a reload would re-collapse it and hide the just-indented child.
      const np = s.byId[newParent];
      np.raw = rawWithCollapsed(np.raw, false);
      np.collapsed = false;
    })
  );
  startEditing(id, caretOffset);
  markDirty(pageName);
}

/** Shift+Tab: move the block out to be the next sibling of its parent. */
export function outdentBlock(id: string, caretOffset: number) {
  const node = doc.byId[id];
  if (node.parent === null) return;
  pushUndo("outdent", [node.page]);
  const parentId = node.parent;
  const grandParent = doc.byId[parentId].parent;
  const pageName = node.page;

  setDoc(
    produce((s) => {
      const parent = s.byId[parentId];
      const idx = parent.children.indexOf(id);
      const following = parent.children.splice(idx);
      following.shift(); // drop id
      for (const f of following) s.byId[f].parent = id;
      s.byId[id].children.push(...following);
      s.byId[id].parent = grandParent;
      const gArr = grandParent === null
        ? s.pages[s.pages.findIndex((p) => p.name === pageName)].roots
        : s.byId[grandParent].children;
      gArr.splice(gArr.indexOf(parentId) + 1, 0, id);
    })
  );
  startEditing(id, caretOffset);
  markDirty(pageName);
}

/** Backspace at offset 0: merge into the previous visible block (same page). */
export function mergeWithPrev(id: string): boolean {
  const prev = prevVisible(id);
  if (prev === null) return false;
  const node = doc.byId[id];
  if (doc.byId[prev].page !== node.page) return false; // don't merge across pages
  pushUndo("merge", [node.page]);
  // Merge visible content only; keep the previous block's hidden props (it keeps
  // its identity) and drop the absorbed block's — otherwise the id::/collapsed::
  // lines would be concatenated mid-line and a block could end up with two ids.
  const prevSplit = splitProps(doc.byId[prev].raw, isBuiltinHidden);
  const curSplit = splitProps(node.raw, isBuiltinHidden);
  const curVisible = curSplit.visible;
  const joinOffset = prevSplit.visible.length;
  const pageName = node.page;

  // Preserve the absorbed block's `id::` if the survivor has none — otherwise
  // inbound ((id)) references to the absorbed block would orphan on merge.
  let hidden = prevSplit.hidden;
  const survivorHasId = /(?:^|\n)id:: /i.test(prevSplit.hidden);
  const absorbedId = /(?:^|\n)(id:: \S+)/i.exec(curSplit.hidden)?.[1];
  if (!survivorHasId && absorbedId) {
    hidden = hidden ? `${hidden}\n${absorbedId}` : absorbedId;
  }

  setDoc(
    produce((s) => {
      s.byId[prev].raw = joinProps(prevSplit.visible + curVisible, hidden);
      for (const c of node.children) s.byId[c].parent = prev;
      s.byId[prev].children.push(...node.children);
      const arr = node.parent === null
        ? s.pages[s.pages.findIndex((p) => p.name === pageName)].roots
        : s.byId[node.parent].children;
      arr.splice(arr.indexOf(id), 1);
      delete s.byId[id];
    })
  );
  startEditing(prev, joinOffset);
  markDirty(pageName);
  return true;
}

/** Insert a parsed outline (from a paste) as siblings right after `afterId`.
 *  Returns the last top-level inserted block id (to focus). */
export function insertOutlineAfter(afterId: string, nodes: OutlineNode[]): string {
  if (!nodes.length) return afterId;
  pushUndo("paste", [doc.byId[afterId].page]);
  const parent = doc.byId[afterId].parent;
  const pageName = doc.byId[afterId].page;
  let lastId = afterId;
  setDoc(
    produce((s) => {
      const create = (n: OutlineNode, par: string | null): string => {
        const id = freshId();
        const childIds = n.children.map((c) => create(c, id));
        s.byId[id] = { id, raw: n.raw, collapsed: false, parent: par, page: pageName, children: childIds };
        return id;
      };
      const created = nodes.map((n) => create(n, parent));
      const sibs =
        parent === null
          ? s.pages[s.pages.findIndex((p) => p.name === pageName)].roots
          : s.byId[parent].children;
      sibs.splice(sibs.indexOf(afterId) + 1, 0, ...created);
      lastId = created[created.length - 1];
    })
  );
  markDirty(pageName);
  return lastId;
}

/** Append a quick-capture (Logseq outline markdown, as produced by the capture
 *  window's editor — usually one bullet, but templates/multi-line paste can make
 *  several) at the END of today's journal, then flush immediately. This is the
 *  single writer for global quick-capture: routing through the live store (rather
 *  than a separate-process file append) means a capture can't race a main-view
 *  edit of today's journal into a conflict. Loads — or, if the day has no file
 *  yet, synthesizes — the journal first; never clobbers in-progress edits
 *  (`ensurePageLoaded` is a no-op when already loaded). Returns whether the write
 *  reached disk. */
export async function appendToTodayJournal(markdown: string): Promise<boolean> {
  return captureOutlineInto(journalTitle(new Date()), "journal", parseOutline(markdown));
}

/** In-app quick capture into a (new or existing) named PAGE — the heading-filled
 *  branch of the journal-top capture bar. Same single-writer guarantees as
 *  {@link appendToTodayJournal}: routes through the live store + immediate flush,
 *  so it can't race a main-view edit of the same page into a conflict. */
export async function captureToPage(title: string, markdown: string): Promise<boolean> {
  const name = title.trim();
  if (!name) return false;
  return captureOutlineInto(name, "page", parseOutline(markdown));
}

/** Append outline `nodes` at the END of the named page (loaded — or synthesized
 *  if it has no file yet — first), then flush immediately. Shared by the journal
 *  append and the new-page capture; never clobbers in-progress edits
 *  (`ensurePageLoaded` is a no-op when already loaded). Returns whether it landed. */
async function captureOutlineInto(name: string, kind: PageKind, nodes: OutlineNode[]): Promise<boolean> {
  if (!nodes.length) return false;
  if (!pageByName(name)) {
    const dto: PageDto =
      (await backend().getPage(name, kind)) ??
      { name, kind, title: name, pre_block: null, blocks: [], rev: null };
    ensurePageLoaded(dto);
  }
  const page = pageByName(name);
  if (!page) return false;
  if (page.roots.length) {
    // Append after the last top-level block (end of the page).
    insertOutlineAfter(page.roots[page.roots.length - 1], nodes);
  } else {
    // Empty (or brand-new) page: seed an empty anchor root, append after it, then
    // drop the anchor — reuses insertOutlineAfter's subtree creation rather than a
    // bespoke root builder.
    pushUndo("capture", [name]);
    const anchor = freshId();
    setDoc(
      produce((s) => {
        s.byId[anchor] = { id: anchor, raw: "", collapsed: false, parent: null, page: name, children: [] };
        s.pages[s.pages.findIndex((p) => p.name === name)].roots.push(anchor);
      })
    );
    markDirty(name);
    insertOutlineAfter(anchor, nodes);
    deleteBlock(anchor);
  }
  return await flushPage(name);
}

const PROP_LINE = /^([A-Za-z0-9_./-]+):: ?(.*)$/;

/** Current value of a `key:: value` block property, or null. */
export function blockProperty(id: string, key: string): string | null {
  const node = doc.byId[id];
  if (!node) return null;
  for (const line of node.raw.split("\n")) {
    const m = PROP_LINE.exec(line);
    if (m && m[1] === key) return m[2].trim();
  }
  return null;
}

/** Set (or remove, when value is null) a `key:: value` block property. Property
 *  lines live after the block's content lines, matching Logseq. */
export function setBlockProperty(id: string, key: string, value: string | null) {
  const node = doc.byId[id];
  if (!node) return;
  pushUndo(`prop:${id}:${key}`, [node.page]);
  const lines = node.raw.split("\n").filter((l) => {
    const m = PROP_LINE.exec(l);
    return !(m && m[1] === key);
  });
  if (value !== null) lines.push(`${key}:: ${value}`);
  setDoc("byId", id, "raw", lines.join("\n"));
  markDirty(node.page);
}

/** Read a page-level property from the page's pre-block (the leading
 *  `key:: value` lines), or null. */
export function readPageProperty(pageName: string, key: string): string | null {
  const p = doc.pages.find((x) => x.name === pageName);
  return p ? readPropertyValue(p.preBlock, key) : null;
}

/** Set or clear a page-level property in the page's pre-block. Persists through
 *  the normal dirty/save path (pageToDto includes pre_block); undo-safe because
 *  the page snapshot captures preBlock. */
export function setPageProperty(pageName: string, key: string, value: string | null) {
  const idx = doc.pages.findIndex((x) => x.name === pageName);
  if (idx < 0) return;
  pushUndo(`pageprop:${pageName}:${key}`, [pageName]);
  setDoc("pages", idx, "preBlock", upsertPropertyLine(doc.pages[idx].preBlock, key, value));
  markDirty(pageName);
}

/** Toggle a property: set it to `value`, or remove it if already that value. */
export function toggleBlockProperty(id: string, key: string, value: string) {
  setBlockProperty(id, key, blockProperty(id, key) === value ? null : value);
}

const ORDER_KEY = "logseq.order-list-type";
function isOrdered(id: string | null | undefined): boolean {
  return !!id && blockProperty(id, ORDER_KEY) === "number";
}
function toLetters(n: number): string {
  let s = "";
  while (n > 0) {
    const r = (n - 1) % 26;
    s = String.fromCharCode(97 + r) + s;
    n = Math.floor((n - 1) / 26);
  }
  return s || "a";
}
function toRoman(n: number): string {
  const map: [number, string][] = [
    [1000, "m"], [900, "cm"], [500, "d"], [400, "cd"], [100, "c"], [90, "xc"],
    [50, "l"], [40, "xl"], [10, "x"], [9, "ix"], [5, "v"], [4, "iv"], [1, "i"],
  ];
  let s = "";
  for (const [v, sym] of map) while (n >= v) { s += sym; n -= v; }
  return s || "i";
}

/** The ordered-list label for a block whose `logseq.order-list-type` is `number`
 *  (else null) — the block's OWN bullet, like OG. The index counts this block
 *  plus the run of consecutive ordered siblings immediately before it; the glyph
 *  cycles number → letter → roman by the depth of consecutive ordered ancestors
 *  (mod 3), so nested ordered lists read 1. → a. → i. like OG. */
export function orderedListMarker(id: string): string | null {
  const node = doc.byId[id];
  if (!node || !isOrdered(id)) return null;
  const siblings = node.parent
    ? doc.byId[node.parent]?.children
    : doc.pages.find((p) => p.name === node.page)?.roots;
  let idx = 1;
  if (siblings) {
    for (let i = siblings.indexOf(id) - 1; i >= 0 && isOrdered(siblings[i]); i--) idx++;
  }
  let depth = 0;
  for (let p = node.parent; isOrdered(p); p = doc.byId[p!]?.parent ?? null) depth++;
  const delta = depth % 3;
  return delta === 0 ? String(idx) : delta === 1 ? toLetters(idx) : toRoman(idx);
}

/** Tick/untick a checkbox on one line of an in-block `+ [ ]` markdown list,
 *  identified by its exact source line. Pure `[ ]`↔`[x]` text swap — round-trips
 *  as standard markdown (and renders/ticks in OG + mobile). */
export function toggleListItem(id: string, rawLine: string) {
  const node = doc.byId[id];
  if (!node) return;
  const lines = node.raw.split("\n");
  const idx = lines.indexOf(rawLine);
  if (idx < 0) return;
  const ln = lines[idx];
  const next = /\[ \]/.test(ln) ? ln.replace(/\[ \]/, "[x]") : ln.replace(/\[[xX]\]/, "[ ]");
  if (next === ln) return;
  pushUndo(`listcheck:${id}`, [node.page]);
  lines[idx] = next;
  setDoc("byId", id, "raw", lines.join("\n"));
  markDirty(node.page);
}

/** Set the block's heading level via the markdown `#` prefix (null clears it). */
export function setHeading(id: string, level: number | null) {
  const node = doc.byId[id];
  if (!node) return;
  pushUndo(`heading:${id}`, [node.page]);
  const lines = node.raw.split("\n");
  let first = (lines[0] ?? "").replace(/^#{1,6} /, "");
  if (level && level >= 1 && level <= 6) first = `${"#".repeat(level)} ${first}`;
  lines[0] = first;
  setDoc("byId", id, "raw", lines.join("\n"));
  markDirty(node.page);
}

const WEEKDAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const pad2 = (n: number) => String(n).padStart(2, "0");

/** Read a block's SCHEDULED/DEADLINE date as {y,m,d} (m 0-based), or null. */
export function readSchedule(
  id: string,
  which: "scheduled" | "deadline"
): { y: number; m: number; d: number; repeater: string | null } | null {
  const node = doc.byId[id];
  if (!node) return null;
  const tag = which === "scheduled" ? "SCHEDULED" : "DEADLINE";
  // Optionally capture an org repeater cookie (`+1w`, `.+1w`, `++1w`) after the
  // weekday, so re-opening the picker can pre-fill the existing recurrence.
  const m = new RegExp(
    `^${tag}:\\s*<(\\d{4})-(\\d{2})-(\\d{2})(?:\\s+[A-Za-z]{3})?(?:\\s+((?:\\.\\+|\\+\\+|\\+)\\d+[dwmy]))?`,
    "m"
  ).exec(node.raw);
  return m ? { y: +m[1], m: +m[2] - 1, d: +m[3], repeater: m[4] ?? null } : null;
}

/** Set or clear a block's SCHEDULED/DEADLINE org-timestamp (line 2, like OG).
 *  `repeater` is an org recurrence cookie (`+1w`, `.+1w`, `++1w`) or null — it's
 *  written inside the `<…>` and consumed by repeat.ts when the task is completed. */
export function setSchedule(
  id: string,
  which: "scheduled" | "deadline",
  date: { y: number; m: number; d: number } | null,
  repeater?: string | null
) {
  const node = doc.byId[id];
  if (!node) return;
  pushUndo(`sched:${id}:${which}`, [node.page]);
  const tag = which === "scheduled" ? "SCHEDULED" : "DEADLINE";
  const lines = node.raw.split("\n").filter((l) => !new RegExp(`^${tag}:`).test(l.trim()));
  if (date) {
    const wd = WEEKDAYS[new Date(date.y, date.m, date.d).getDay()];
    const rep = repeater ? ` ${repeater}` : "";
    const stamp = `${tag}: <${date.y}-${pad2(date.m + 1)}-${pad2(date.d)} ${wd}${rep}>`;
    lines.splice(Math.min(1, lines.length), 0, stamp);
  }
  setDoc("byId", id, "raw", lines.join("\n"));
  markDirty(node.page);
}

/** A block's raw with `collapsed:: true` added or removed so the persisted
 *  property matches the collapsed state. OG stores collapse in the file as a
 *  block property, so mirroring it here makes a collapse survive a relaunch and
 *  show up collapsed in OG / the mobile app. Fence-aware via splitProps. */
function rawWithCollapsed(raw: string, collapsed: boolean): string {
  const { visible, hidden } = splitProps(raw, isBuiltinHidden);
  const nextHidden = upsertPropertyLine(hidden, "collapsed", collapsed ? "true" : null) ?? "";
  return joinProps(visible, nextHidden);
}

/** Set a block's collapsed state AND mirror it into its raw `collapsed::` so it
 *  persists — the on-disk markdown is the source of truth on the next load. */
function writeCollapsed(id: string, collapsed: boolean) {
  const n = doc.byId[id];
  if (!n) return;
  const nextRaw = rawWithCollapsed(n.raw, collapsed);
  setDoc("byId", id, "collapsed", collapsed);
  if (nextRaw !== n.raw) setDoc("byId", id, "raw", nextRaw);
}

/** Collapse or expand a block and its entire descendant subtree. */
export function setCollapsedDeep(id: string, collapsed: boolean) {
  pushUndo("collapse-all", [doc.byId[id].page]);
  const walk = (bid: string) => {
    const n = doc.byId[bid];
    if (!n) return;
    if (n.children.length) writeCollapsed(bid, collapsed);
    n.children.forEach(walk);
  };
  walk(id);
  markDirty(doc.byId[id].page);
}

/** Ensure a block has a persistent `id::` uuid (assigned lazily, like OG) AND
 *  that it's durably on disk, returning the uuid — or null if it couldn't be
 *  saved (conflict/error). Used to make `((uuid))` references: the caller must
 *  not put a ref on the clipboard until the id is actually written, or quitting /
 *  resolving a conflict with "use disk version" would leave the ref dangling. */
export async function ensureBlockId(id: string): Promise<string | null> {
  const node = doc.byId[id];
  if (!node) return null;
  // Any existing id:: is the block's durable id — match its value (not just a
  // UUID shape), case-INSENSITIVELY (Rust's property("id") is case-insensitive, so
  // an `ID::` from another editor counts), so we never append a SECOND id:: that
  // Rust then ignores → dangling copied ref.
  const m = /(?:^|\n)id:: *(\S+)/i.exec(node.raw);
  const uuid = m ? m[1] : crypto.randomUUID();
  if (!m) {
    setDoc("byId", id, "raw", `${node.raw}\nid:: ${uuid}`);
    markDirty(node.page);
  }
  // Even a pre-existing id:: may not be on disk yet (added in-memory, not flushed);
  // flush and only hand back the uuid if the write actually landed.
  const ok = await flushPage(node.page);
  return ok ? uuid : null;
}

/** A live reference to a loaded block — its stable uuid + the page it lives on
 *  (so a satellite surface can load that page and render the same editable
 *  node). The uuid IS the store key, so no snapshot is needed. */
export function blockRef(id: string): { uuid: string; page: string; pageKind: PageKind } {
  const n = doc.byId[id];
  return { uuid: n.id, page: n.page, pageKind: pageByName(n.page)?.kind ?? "page" };
}

const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

/** Persist a block's `id::` equal to its current uuid, so its in-memory key and
 *  on-disk identity match — making a reference to it survive an app restart.
 *  No-op if it already has an id::. Only writes when the uuid is a real UUID
 *  (always true for blocks loaded from the graph); a freshly-created,
 *  not-yet-reloaded block is skipped rather than writing a non-UUID id::. */
export function ensureStableBlockId(id: string): void {
  const node = doc.byId[id];
  if (!node) return;
  if (/(?:^|\n)id:: *\S+/.test(node.raw)) return;
  if (!UUID_RE.test(id)) return;
  setDoc("byId", id, "raw", `${node.raw}\nid:: ${id}`);
  markDirty(node.page);
  // Persist now, not on the 400ms debounce: the user may quit right after
  // parking the block, and a pending timer is lost when the webview closes.
  void flushPage(node.page);
}

/** Like `blockRef`, but first persists the block's `id::` so the reference
 *  resolves after a restart. Used for parking a block durably: the right sidebar,
 *  a new tab, and zoom all stamp `id::` so the spot survives a relaunch (Martin's
 *  call — he wants these to persist; the `id::` is harmless in the file and is
 *  stripped from clipboard copies anyway, see `blockSubtreeMarkdown`). */
export function persistentBlockRef(id: string): { uuid: string; page: string; pageKind: PageKind } {
  ensureStableBlockId(id);
  return blockRef(id);
}

/** Make a freshly-inserted `((uuid))` reference durable: ensure the TARGET block
 *  (which may live on a page that isn't loaded — block search spans the whole
 *  graph) carries `id:: uuid` on disk, so the ref still resolves after a restart.
 *  The owning page is loaded only if absent (`ensurePageLoaded` never clobbers
 *  unsaved edits). A no-op if the block already has an `id::`. Fire-and-forget:
 *  the ref resolves in-session via the in-memory uuid even before this lands. */
export async function persistBlockRefTarget(
  uuid: string,
  page: string,
  kind: PageKind
): Promise<void> {
  if (!doc.byId[uuid]) {
    const dto = await backend().getPage(page, kind);
    if (dto) ensurePageLoaded(dto);
  }
  // Re-check: a concurrent navigation may have loaded the page meanwhile, or the
  // cache may have been rebuilt (external change) and reassigned the block a new
  // uuid — in which case there's nothing safe to stamp.
  if (doc.byId[uuid]) ensureStableBlockId(uuid);
}

/** Serialize a block (and, normally, its subtree) to Logseq markdown.
 *  - `stripId`: drop the internal `id::` property line (fence-aware) — OG does this
 *    when copying to the clipboard (`copy-to-clipboard-without-id-property!`) so a
 *    referenced block doesn't leak `id:: <uuid>` into pasted text. (Quick-capture
 *    writing to a journal FILE passes false to keep `id::`.)
 *  - `stripCollapsed`: also drop `collapsed::` (OG keeps it; opt-in cleaner copy).
 *  - `onlySelected`: when a Set is passed, recurse only into children that are in it
 *    (used by the "copy only the selected blocks, not the whole sub-tree" mode). */
export function blockSubtreeMarkdown(
  id: string,
  level = 0,
  stripId = false,
  stripCollapsed = false,
  onlySelected?: Set<string>
): string {
  const n = doc.byId[id];
  if (!n) return "";
  const tabs = "\t".repeat(level);
  const strip = stripId || stripCollapsed;
  const raw = strip
    ? splitProps(n.raw, (k) => (stripId && k === "id") || (stripCollapsed && k === "collapsed")).visible
    : n.raw;
  const lines = raw.split("\n");
  const out: string[] = [];
  out.push(`${tabs}- ${lines[0] ?? ""}`.replace(/\s+$/, ""));
  for (const line of lines.slice(1)) {
    out.push(line === "" ? "" : `${tabs}  ${line}`);
  }
  for (const c of n.children) {
    if (onlySelected && !onlySelected.has(c)) continue;
    out.push(blockSubtreeMarkdown(c, level + 1, stripId, stripCollapsed, onlySelected));
  }
  return out.join("\n");
}

/** Build an ExportNode forest (raw + children) for the given block ids and their
 *  subtrees — input to the configurable text exporter (Copy / Export modal). */
export function exportNodesFor(ids: string[]): ExportNode[] {
  const set = new Set(ids);
  // A multi-selection (selectedIds) is a flat slice of visible order, so it can
  // contain BOTH a parent and its descendants. Export only the selection's roots
  // — a kept node's subtree already carries its children, so emitting a selected
  // child again as a top-level node would duplicate it (the "1 2 3 1 2 3" bug).
  const hasSelectedAncestor = (id: string): boolean => {
    let p = doc.byId[id]?.parent ?? null;
    while (p !== null) {
      if (set.has(p)) return true;
      p = doc.byId[p]?.parent ?? null;
    }
    return false;
  };
  const toNode = (id: string): ExportNode | null => {
    const n = doc.byId[id];
    if (!n) return null;
    return { raw: n.raw, children: n.children.map(toNode).filter((x): x is ExportNode => x != null) };
  };
  return ids
    .filter((id) => !hasSelectedAncestor(id))
    .map(toNode)
    .filter((x): x is ExportNode => x != null);
}

/** Serialize a fetched BlockDto subtree to Logseq markdown (for pages not in the
 *  working set, e.g. copy-page-as-markdown). */
export function dtoSubtreeMarkdown(b: BlockDto, level = 0): string {
  const tabs = "\t".repeat(level);
  const lines = b.raw.split("\n");
  const out: string[] = [];
  out.push(`${tabs}- ${lines[0] ?? ""}`.replace(/\s+$/, ""));
  for (const line of lines.slice(1)) out.push(line === "" ? "" : `${tabs}  ${line}`);
  for (const c of b.children) out.push(dtoSubtreeMarkdown(c, level + 1));
  return out.join("\n");
}

/** Remove a block and its subtree. */
function deleteBlockInternal(id: string) {
  const node = doc.byId[id];
  if (!node) return;
  const pageName = node.page;
  setDoc(
    produce((s) => {
      const arr =
        node.parent === null
          ? s.pages[s.pages.findIndex((p) => p.name === pageName)].roots
          : s.byId[node.parent!].children;
      arr.splice(arr.indexOf(id), 1);
      const rm = (bid: string) => {
        for (const c of s.byId[bid].children) rm(c);
        delete s.byId[bid];
      };
      rm(id);
    })
  );
  if (editingId() === id) setEditingId(null);
  markDirty(pageName);
}

export function deleteBlock(id: string) {
  if (!doc.byId[id]) return;
  pushUndo("delete", [doc.byId[id].page]);
  deleteBlockInternal(id);
}

// ---------------------------------------------------------------------------
// Multi-block selection (Escape from editing; Shift+Arrows extend) + ops
// ---------------------------------------------------------------------------

const [selAnchor, setSelAnchor] = createSignal<string | null>(null);
const [selFocus, setSelFocus] = createSignal<string | null>(null);

export function selectedIds(): string[] {
  const a = selAnchor();
  const f = selFocus();
  if (!a || !f) return [];
  const order = visibleOrder();
  let i = order.indexOf(a);
  let j = order.indexOf(f);
  if (i < 0 || j < 0) return [];
  if (i > j) [i, j] = [j, i];
  return order.slice(i, j + 1);
}
// Memoized set of selected ids. `isSelected` is read in the render of EVERY
// block (Block.tsx classList), and selectedIds() rebuilds visibleOrder() each
// call — so without this, a selection over N visible blocks costs O(N²). The
// memo recomputes only when the anchor/focus or the visible tree changes.
const selectedSet = createRoot(() => createMemo(() => new Set(selectedIds())));
export function isSelected(id: string): boolean {
  return selectedSet().has(id);
}
export function selectBlock(id: string) {
  setEditingId(null);
  setSelAnchor(id);
  setSelFocus(id);
}
export function clearSelection() {
  setSelAnchor(null);
  setSelFocus(null);
}
/** Extend the current block selection's focus to `id` (mouse-drag / shift-click).
 *  Starts a fresh selection anchored at `id` if none is active. */
export function extendSelectionTo(id: string) {
  if (selAnchor() === null) setSelAnchor(id);
  setSelFocus(id);
}
export function hasSelection(): boolean {
  return selAnchor() !== null;
}
export function moveSelection(dir: 1 | -1, extend: boolean) {
  const f = selFocus();
  if (!f) return;
  const order = visibleOrder();
  const i = order.indexOf(f);
  const ni = i + dir;
  if (ni < 0 || ni >= order.length) return;
  const next = order[ni];
  setSelFocus(next);
  if (!extend) setSelAnchor(next);
  scrollBlockRowIntoView(next);
}

/** Keep the active end of a keyboard selection on screen: as the user holds
 *  Arrow / Shift+Arrow past the top or bottom edge, reveal the newly-focused
 *  block. Targets the block's own row (`.block-main`), not the whole `.ls-block`
 *  (which spans its children and could be taller than the viewport), and uses
 *  `block: "nearest"` so it's a no-op while the row is already visible — it only
 *  scrolls when the row crosses an edge, and never recenters mid-page. Run on the
 *  next frame so the focus class is on the DOM before we measure. */
function scrollBlockRowIntoView(id: string) {
  // No-op under the test/headless runtime (no rAF/DOM); only the real webview scrolls.
  if (typeof requestAnimationFrame !== "function" || typeof document === "undefined") return;
  requestAnimationFrame(() => {
    const sel = typeof CSS !== "undefined" && CSS.escape ? CSS.escape(id) : id;
    const row = document.querySelector(`.ls-block[data-block-id="${sel}"] > .block-main`);
    row?.scrollIntoView({ block: "nearest" });
  });
}

/** Top-level selected blocks (exclude those whose parent is also selected). */
function topSelected(): string[] {
  const ids = selectedIds();
  const set = new Set(ids);
  return ids.filter((id) => {
    const p = doc.byId[id]?.parent;
    return !(p && set.has(p));
  });
}

export function indentSelection() {
  const ids = topSelected();
  if (!ids.length) return;
  const first = ids[0];
  const sibs = rootsOf(first);
  const fi = sibs.indexOf(first);
  if (fi <= 0) return;
  const newParent = sibs[fi - 1];
  // Structural indent is single-page ONLY. The target (newParent) is on first's
  // page; moving a block from another feed day under it would be a cross-page
  // structural move (removal-before-add hazard) — and indenting under a different
  // day's block is nonsensical anyway. So move only the selected blocks that are
  // already on the target page.
  const destPage = doc.byId[newParent].page;
  const same = ids.filter((id) => doc.byId[id]?.page === destPage);
  if (!same.length) return;
  pushUndo("indent-sel", [destPage]);
  for (const id of same) moveBlockInternal(id, newParent, doc.byId[newParent].children.length);
  writeCollapsed(newParent, false);
}

export function outdentSelection() {
  const ids = topSelected();
  if (!ids.length) return;
  const parentId = doc.byId[ids[0]].parent;
  if (parentId === null) return;
  const grand = doc.byId[parentId].parent;
  // Single-page only (see indentSelection): outdent moves blocks to `grand`, on
  // ids[0]'s page — so restrict to the blocks already on that page.
  const destPage = doc.byId[parentId].page;
  const same = ids.filter((id) => doc.byId[id]?.page === destPage);
  if (!same.length) return;
  pushUndo("outdent-sel", [destPage]);
  let after = parentId;
  for (const id of same) {
    moveBlockInternal(id, grand, indexInSiblings(after) + 1);
    after = id;
  }
}

export function deleteSelection() {
  const ids = topSelected();
  if (!ids.length) return;
  const pages = new Set<string>();
  for (const id of ids) {
    const n = doc.byId[id];
    if (n) pages.add(n.page);
  }
  pushUndo("delete-sel", [...pages]);
  // One produce for the whole selection — deleting each block separately fires a
  // reactive update per block (15 reflows for 15 bullets); batching collapses it
  // to a single update so the cut feels instant.
  setDoc(
    produce((s) => {
      for (const id of ids) {
        const node = s.byId[id];
        if (!node) continue;
        pages.add(node.page);
        const arr =
          node.parent === null
            ? s.pages[s.pages.findIndex((p) => p.name === node.page)].roots
            : s.byId[node.parent].children;
        const ix = arr.indexOf(id);
        if (ix >= 0) arr.splice(ix, 1);
        const rm = (bid: string) => {
          for (const c of s.byId[bid].children) rm(c);
          delete s.byId[bid];
        };
        rm(id);
      }
    })
  );
  const ed = editingId();
  if (ed && !doc.byId[ed]) setEditingId(null);
  for (const p of pages) markDirty(p);
  clearSelection();
}

export function selectionMarkdown(): string {
  // Clipboard → always strip id:: (OG parity). collapsed:: and whole-subtree vs
  // selected-only are user-configurable (see copySettings): OG copies the full
  // sub-tree of a selected parent; Tine's default copies only the selected blocks.
  const stripCollapsed = copyStripCollapsed();
  const onlySel = copyIncludeSubtree() ? undefined : new Set(selectedIds());
  return topSelected()
    .map((id) => blockSubtreeMarkdown(id, 0, true, stripCollapsed, onlySel))
    .join("\n");
}

/** Move a block to be a child of `newParent` (or root of its page) at `index`.
 *  Used by drag-and-drop. */
/** Move without pushing an undo entry (for batched selection ops). */
function moveBlockInternal(id: string, newParent: string | null, index: number) {
  const node = doc.byId[id];
  if (!node) return;
  let p = newParent;
  while (p !== null) {
    if (p === id) return;
    p = doc.byId[p].parent;
  }
  const oldPage = node.page;
  const newPage = newParent ? doc.byId[newParent].page : oldPage;
  setDoc(
    produce((s) => {
      const oldArr =
        node.parent === null
          ? s.pages[s.pages.findIndex((x) => x.name === oldPage)].roots
          : s.byId[node.parent!].children;
      const from = oldArr.indexOf(id);
      oldArr.splice(from, 1);
      s.byId[id].parent = newParent;
      const newArr =
        newParent === null
          ? s.pages[s.pages.findIndex((x) => x.name === newPage)].roots
          : s.byId[newParent].children;
      let idx = index;
      if (oldArr === newArr && from < idx) idx -= 1;
      newArr.splice(Math.max(0, Math.min(idx, newArr.length)), 0, id);
      if (newPage !== oldPage) {
        const reassign = (bid: string) => {
          s.byId[bid].page = newPage;
          s.byId[bid].children.forEach(reassign);
        };
        reassign(id);
      }
    })
  );
  markDirty(oldPage);
  if (newPage !== oldPage) markDirty(newPage);
}

/** Move a block under `newParent` (or, when `newParent` is null, to the roots of
 *  `targetPage` — pass the drop target's page so a root-to-root drop across pages
 *  lands on the RIGHT page instead of defaulting back to the source). */
export async function moveBlock(
  id: string,
  newParent: string | null,
  index: number,
  targetPage?: string
) {
  const node = doc.byId[id];
  if (!node) return;
  // Don't drop a block into its own descendant.
  let p = newParent;
  while (p !== null) {
    if (p === id) return;
    p = doc.byId[p].parent;
  }
  const oldPage = node.page;
  // A root drop has no parent to read the page from — use the explicit target
  // page (the day/page the drop landed on); fall back to the source page only if
  // the caller didn't supply one (a same-page reorder).
  const newPage = newParent ? doc.byId[newParent].page : (targetPage ?? oldPage);
  // Cross-page drag: flush the source while it still holds the block, so a
  // pre-existing pending save can't write the removal before the destination
  // lands. Abort (no move) if the source can't be saved.
  if (newPage !== oldPage && !(await prepareCrossPageSources([oldPage]))) {
    pushToast(`Couldn't move — “${oldPage}” has unsaved changes that need resolving first.`, "error");
    return;
  }
  if (!doc.byId[id]) return; // block vanished during the async flush
  // Drag-move can cross pages → snapshot both source and destination.
  pushUndo("move", [...new Set([oldPage, newPage])]);
  setDoc(
    produce((s) => {
      const oldArr =
        node.parent === null
          ? s.pages[s.pages.findIndex((x) => x.name === oldPage)].roots
          : s.byId[node.parent!].children;
      const from = oldArr.indexOf(id);
      oldArr.splice(from, 1);
      s.byId[id].parent = newParent;
      const newArr =
        newParent === null
          ? s.pages[s.pages.findIndex((x) => x.name === newPage)].roots
          : s.byId[newParent].children;
      let idx = index;
      if (oldArr === newArr && from < idx) idx -= 1;
      newArr.splice(Math.max(0, Math.min(idx, newArr.length)), 0, id);
      // Reassign the moved subtree to the target page.
      if (newPage !== oldPage) {
        const reassign = (bid: string) => {
          s.byId[bid].page = newPage;
          s.byId[bid].children.forEach(reassign);
        };
        reassign(id);
      }
    })
  );
  if (newPage !== oldPage) {
    // Cross-page drag: persist the destination before the source removal.
    persistCrossPage(newPage, [oldPage]);
  } else {
    markDirty(oldPage);
  }
}

/** Move a block up/down among its siblings (mod+Up/Down). Keyed <For> keeps the
 *  DOM node — so if the block is being edited, the textarea + caret survive. */
// During a block-move reorder the textarea momentarily blurs; this flag tells
// the editor's onBlur to keep edit mode (the move handler refocuses + restores
// the caret right after).
let blockMoving = false;
export function isBlockMoving(): boolean {
  return blockMoving;
}
export function setBlockMoving(v: boolean): void {
  blockMoving = v;
}

export function moveItem(id: string, dir: 1 | -1) {
  const node = doc.byId[id];
  if (!node) return;
  const sibs = rootsOf(id);
  const i = sibs.indexOf(id);
  const ni = i + dir;
  if (ni < 0 || ni >= sibs.length) return;
  pushUndo("move-item", [node.page]);
  setDoc(
    produce((s) => {
      const arr =
        node.parent === null
          ? s.pages[s.pages.findIndex((p) => p.name === node.page)].roots
          : s.byId[node.parent!].children;
      arr.splice(i, 1);
      arr.splice(ni, 0, id);
    })
  );
  markDirty(node.page);
}

/** Can a block move one slot in `dir` within its sibling list? */
function canMoveItem(id: string, dir: 1 | -1): boolean {
  const sibs = rootsOf(id);
  const ni = sibs.indexOf(id) + dir;
  return ni >= 0 && ni < sibs.length;
}

// The journal feed treats its days as one continuous list: a root block at the
// top/bottom of a day moves into the adjacent *displayed* day (feed order, not
// calendar — non-displayed days like an uncreated 16th are skipped). Page.tsx
// registers a loader so a down-move past the last loaded day pulls in more.
let feedExtender: (() => Promise<boolean>) | null = null;
export function setFeedExtender(fn: () => Promise<boolean>): void {
  feedExtender = fn;
}

/** Reassign a block subtree's `page` (used when it crosses to another day). */
function reassignPage(s: DocState, id: string, page: string) {
  s.byId[id].page = page;
  for (const c of s.byId[id].children) reassignPage(s, c, page);
}

/** Move root blocks `ids` (document order) to the start (down) / end (up) of
 *  `toPage`, removing them from `fromPage`. Both pages must be loaded. */
function crossMoveBlocks(ids: string[], fromPage: string, toPage: string, dir: 1 | -1) {
  setDoc(
    produce((s) => {
      const from = s.pages.find((p) => p.name === fromPage);
      const to = s.pages.find((p) => p.name === toPage);
      if (!from || !to) return;
      const idset = new Set(ids);
      from.roots = from.roots.filter((x) => !idset.has(x));
      // up → bottom of the day above; down → top of the day below (keep order).
      if (dir === -1) to.roots.push(...ids);
      else to.roots.unshift(...ids);
      for (const id of ids) {
        s.byId[id].parent = null;
        reassignPage(s, id, toPage);
      }
    })
  );
  persistCrossPage(toPage, [fromPage]);
}

/** Persist a cross-page move so the ADDITION side (`dest`) lands on disk BEFORE
 *  any REMOVAL side (`sources`). If dest fails to save (e.g. an external
 *  conflict), the sources are NOT written, so disk is never left with the block
 *  removed from its source but never written to its destination (the data-losing
 *  state). dest is marked dirty immediately; each source only once dest succeeds. */
function persistCrossPage(dest: string, sources: string[]) {
  markDirty(dest);
  void (async () => {
    if (!(await flushPage(dest))) return; // dest conflict/failure → leave sources on disk
    for (const s of sources) {
      if (s !== dest) markDirty(s);
    }
    scheduleSave();
  })();
}

/** Before a cross-page move mutates memory, durably flush every SOURCE page while
 *  it still contains the blocks. Otherwise a save that was ALREADY pending/in-flight
 *  for a source (from an earlier, unrelated edit) can fire right after the in-memory
 *  removal and write the post-removal state to disk before the destination is saved
 *  — a removal-only, data-losing state that dest-first persistence alone can't
 *  prevent. Returns false if any source can't be flushed (an unresolved conflict);
 *  the caller MUST then abort the move. Clean sources flush as instant no-ops. */
export async function prepareCrossPageSources(sources: string[]): Promise<boolean> {
  for (const s of new Set(sources)) {
    if ((isDirty(s) || isSaving(s)) && !(await flushPage(s))) return false;
  }
  return true;
}

/** Resolve the adjacent feed day for a root block at the page boundary, loading
 *  older days if a down-move runs off the last loaded one. Returns the target
 *  page name, or null if there's nowhere to go. */
async function feedNeighbor(page: string, dir: 1 | -1): Promise<string | null> {
  let fi = doc.feed.indexOf(page);
  if (fi < 0) return null; // not a feed day (e.g. a named page)
  let ti = fi + dir;
  if (ti < 0) return null; // top of the feed (today) — can't go higher
  if (ti >= doc.feed.length) {
    if (dir !== 1 || !feedExtender || !(await feedExtender())) return null;
    fi = doc.feed.indexOf(page);
    ti = fi + dir;
    if (ti < 0 || ti >= doc.feed.length) return null;
  }
  return doc.feed[ti];
}

/** Move a single block one slot, crossing into the adjacent day at a page
 *  boundary. Returns how it moved so the caller can restore the caret. */
export async function moveBlockFeed(id: string, dir: 1 | -1): Promise<"within" | "crossed" | "none"> {
  const node = doc.byId[id];
  if (!node) return "none";
  if (canMoveItem(id, dir)) {
    moveItem(id, dir);
    return "within";
  }
  if (node.parent !== null) return "none"; // nested block at a child-list edge: stop
  const target = await feedNeighbor(node.page, dir);
  if (!target) return "none";
  if (!(await prepareCrossPageSources([node.page]))) return "none"; // source has unsaved edits → abort
  if (!doc.byId[id]) return "none"; // vanished during the flush
  pushUndo("move-cross", [node.page, target]);
  crossMoveBlocks([id], node.page, target, dir);
  return "crossed";
}

/** Move every top-level selected block up/down by one slot, preserving the
 *  selection; at a day boundary the whole group crosses into the adjacent day. */
export async function moveSelectionItems(dir: 1 | -1) {
  const ids = topSelected(); // document order: ids[0] topmost, last bottommost
  if (!ids.length) return;
  const lead = dir === 1 ? ids[ids.length - 1] : ids[0];
  if (canMoveItem(lead, dir)) {
    // Batch the whole selection into ONE undo entry + ONE produce. Doing it
    // per-block (a moveItem call each) snapshots the entire working set K times —
    // a 15-block nudge became 15 full clones, the visible jank. Going down, move
    // the bottom-most first so they don't collide; up, the top.
    const ordered = dir === 1 ? [...ids].reverse() : ids;
    const pages = [...new Set(ordered.map((id) => doc.byId[id]?.page).filter(Boolean) as string[])];
    pushUndo("move-sel", pages); // scope the undo to the touched pages, not the whole set
    setDoc(
      produce((s) => {
        for (const id of ordered) {
          const node = s.byId[id];
          if (!node) continue;
          const arr =
            node.parent === null
              ? s.pages[s.pages.findIndex((p) => p.name === node.page)].roots
              : s.byId[node.parent].children;
          const i = arr.indexOf(id);
          const ni = i + dir;
          if (i < 0 || ni < 0 || ni >= arr.length) continue;
          arr.splice(i, 1);
          arr.splice(ni, 0, id);
        }
      })
    );
    for (const p of pages) markDirty(p);
    return;
  }
  // Boundary: cross the whole group into the adjacent day (only if every
  // selected block is a root block on the same feed day).
  const page = doc.byId[ids[0]]?.page;
  if (!page) return;
  if (ids.some((id) => doc.byId[id].parent !== null || doc.byId[id].page !== page)) return;
  const target = await feedNeighbor(page, dir);
  if (!target) return;
  if (!(await prepareCrossPageSources([page]))) return; // source has unsaved edits → abort
  pushUndo("move-sel-cross", [page, target]);
  crossMoveBlocks(ids, page, target, dir);
}

// ---------------------------------------------------------------------------
// Carry unfinished tasks forward (B)
// ---------------------------------------------------------------------------

const OPEN_MARKERS = new Set(["TODO", "DOING", "NOW", "LATER", "WAITING"]);
function isOpenTask(id: string): boolean {
  const m = blockView(doc.byId[id]?.raw ?? "").marker;
  return !!m && OPEN_MARKERS.has(m);
}
function subtreeHasOpenTask(id: string): boolean {
  const n = doc.byId[id];
  if (!n) return false;
  return isOpenTask(id) || n.children.some(subtreeHasOpenTask);
}
/** Collect the top-most open-task blocks in a subtree (open tasks not nested
 *  under another open task) — the pull-out unit when keepContext is off. */
function collectTopOpenTasks(id: string, acc: string[]) {
  if (isOpenTask(id)) {
    acc.push(id);
    return; // its open-task descendants travel with it
  }
  for (const c of doc.byId[id]?.children ?? []) collectTopOpenTasks(c, acc);
}

/** Carry unfinished tasks from `fromPages` into today's journal. Pages are
 *  processed in the given order and each batch is appended, so passing days
 *  newest→oldest puts the newest on top. `keepContext` true moves each top-level
 *  block that contains an open task whole; false pulls out just the open-task
 *  subtrees. Returns the number of blocks moved. Today + every fromPage must be
 *  loaded into the working set first. */
export function carryUnfinished(
  fromPages: string[],
  keepContext: boolean,
  header: string | null
): number {
  const today = journalTitle(new Date());
  if (!pageByName(today)) return 0;
  type Item = { id: string; from: string; parent: string | null };
  const plan: Item[] = [];
  for (const fp of fromPages) {
    if (fp === today) continue;
    const page = pageByName(fp);
    if (!page) continue;
    if (keepContext) {
      for (const rid of page.roots) {
        if (subtreeHasOpenTask(rid)) plan.push({ id: rid, from: fp, parent: null });
      }
    } else {
      const ids: string[] = [];
      for (const rid of page.roots) collectTopOpenTasks(rid, ids);
      for (const id of ids) plan.push({ id, from: fp, parent: doc.byId[id].parent });
    }
  }
  if (!plan.length) return 0;
  pushUndo("carry", [today, ...new Set(plan.map((i) => i.from))]);
  setDoc(
    produce((s) => {
      const todayPage = s.pages.find((p) => p.name === today);
      if (!todayPage) return;
      const carried: string[] = [];
      for (const item of plan) {
        if (item.parent === null) {
          const pg = s.pages.find((p) => p.name === item.from);
          if (pg) pg.roots = pg.roots.filter((x) => x !== item.id);
        } else {
          const par = s.byId[item.parent];
          if (par) par.children = par.children.filter((x) => x !== item.id);
        }
        s.byId[item.id].parent = null;
        reassignPage(s, item.id, today);
        carried.push(item.id);
      }
      // NB: only the carried task blocks are removed from the source day. Anything
      // the user left behind — finished tasks, notes, and blank spacer bullets — is
      // never touched. (A blank bullet that only *held* a carried task is likewise
      // left in place; it never had a task marker itself.)
      // Drop today's lone empty placeholder bullet so carried tasks don't sit
      // under a blank line.
      if (todayPage.roots.length === 1) {
        const only = s.byId[todayPage.roots[0]];
        if (only && only.children.length === 0 && only.raw.trim() === "") {
          delete s.byId[todayPage.roots[0]];
          todayPage.roots = [];
        }
      }
      if (header) {
        const hid = freshId();
        s.byId[hid] = { id: hid, raw: header, collapsed: false, parent: null, page: today, children: [] };
        todayPage.roots.push(hid);
      }
      todayPage.roots.push(...carried);
    })
  );
  // Mark ONLY today (the destination) dirty here. The source days are marked +
  // flushed by carry.ts AFTER today saves, so the debounced batch can't write a
  // source removal while today is still unsaved/conflicted (removal-only loss).
  markDirty(today);
  return plan.length;
}

export function toggleCollapse(id: string) {
  const n = doc.byId[id];
  if (n.children.length === 0) return;
  pushUndo("collapse", [n.page]);
  writeCollapsed(id, !n.collapsed);
  markDirty(n.page);
}

/** Explicitly collapse or expand a block (no-op if it has no children or is
 *  already in the requested state). */
export function setCollapsed(id: string, collapsed: boolean) {
  const n = doc.byId[id];
  if (!n || n.children.length === 0 || n.collapsed === collapsed) return;
  pushUndo("collapse", [n.page]);
  writeCollapsed(id, collapsed);
  markDirty(n.page);
}

