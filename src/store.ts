// The live editing tree. The frontend owns this during a session; all
// keystrokes and structural ops mutate it synchronously (zero IPC). Persistence
// is a debounced per-page save to Rust. See plan §"block editor model".
//
// Supports multiple pages at once (the journals feed): a single global `byId`
// map, each node tagged with its owning `page`, and an ordered `pages` list
// each with its own roots. A single-page route is just a feed of length one.

import { createStore, produce, unwrap } from "solid-js/store";
import { createSignal, createMemo, createRoot } from "solid-js";
import type { BlockDto, PageDto, PageKind } from "./types";
import type { OutlineNode } from "./editor/outline";
import { backend } from "./backend";
import { markConflict, isConflicted, clearConflict, bumpDataRev, rightSidebar, conflicts, pushToast } from "./ui";
import { blockView } from "./render/block";
import { journalTitle } from "./journal";

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
    const childIds = flatten(d.children, d.id, pageName, byId);
    byId[d.id] = {
      id: d.id,
      raw: d.raw,
      collapsed: d.collapsed,
      parent,
      page: pageName,
      children: childIds,
    };
    return d.id;
  });
}

function toFeedPage(dto: PageDto, byId: Record<string, Node>): FeedPage {
  const roots = flatten(dto.blocks, null, dto.name, byId);
  return { name: dto.name, kind: dto.kind, title: dto.title, preBlock: dto.pre_block, roots };
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
  deletedPages.delete(dto.name);
  // Record the load baseline (the on-disk rev) so saves conflict against it.
  baseRev.set(dto.name, dto.rev ?? null);
  setDoc(
    produce((s) => {
      purgePageNodes(s, dto.name);
      const fp = toFeedPage(dto, s.byId);
      const i = s.pages.findIndex((p) => p.name === dto.name);
      if (i >= 0) s.pages[i] = fp;
      else s.pages.push(fp);
    })
  );
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
  dirty.delete(name);
  baseRev.delete(name);
  clearConflict(name);
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
  deletedPages.add(name);
  forgetPage(name);
  try {
    await backend().deletePage(name, kind);
    return true;
  } catch {
    return false;
  }
}

// Cap the working set so a long session browsing a big graph doesn't grow byId
// without bound. FIFO-evict pages that aren't pinned: the main feed, anything
// open in the right sidebar, the page being edited, and any page with unsaved
// edits are all kept (evicting a dirty page would lose those edits).
const WORKING_SET_CAP = 80;
function pinnedPages(): Set<string> {
  const pin = new Set<string>(doc.feed);
  for (const it of rightSidebar()) pin.add(it.kind === "page" ? it.name : it.page);
  for (const name of dirty) pin.add(name);
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
  if (saveTimer) {
    clearTimeout(saveTimer);
    saveTimer = null;
  }
  if (dataRevTimer) {
    clearTimeout(dataRevTimer);
    dataRevTimer = null;
  }
  // Invalidate any in-flight/queued save so it can't write the old graph's
  // content into the newly-loaded graph: bump the token (baseline updates bail)
  // and clear dirty (a stray queued save becomes a no-op).
  graphToken++;
  dirty.clear();
  baseRev.clear();
  deletedPages.clear();
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
  return { id: n.id, raw: n.raw, collapsed: n.collapsed, children: n.children.map(toDto) };
}

export function pageToDto(pageName: string): PageDto | null {
  const p = doc.pages.find((x) => x.name === pageName);
  if (!p) return null;
  return {
    name: p.name,
    kind: p.kind,
    title: p.title,
    pre_block: p.preBlock,
    blocks: p.roots.map(toDto),
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

export function prevVisible(id: string): string | null {
  const { order, index } = visibleData();
  const i = index.get(id);
  return i !== undefined && i > 0 ? order[i - 1] : null;
}

export function nextVisible(id: string): string | null {
  const { order, index } = visibleData();
  const i = index.get(id);
  return i !== undefined && i < order.length - 1 ? order[i + 1] : null;
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
  }));
}
function snapEntry(affected?: string[] | null): SnapEntry {
  // null/omitted → snapshot the whole working set (safe fallback). Otherwise just
  // the named pages: their FeedPage objects + every node living on them.
  const names = affected ?? doc.pages.map((p) => p.name);
  const nameSet = new Set(names);
  const byId = unwrap(doc.byId);
  const nodes: Record<string, Node> = {};
  for (const id in byId) {
    if (nameSet.has(byId[id].page)) nodes[id] = cloneNode(byId[id]);
  }
  const pageObjs = clonePages(unwrap(doc.pages).filter((p) => nameSet.has(p.name)));
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
      dirty.add(e.page);
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
    const scope = new Set(e.pages);
    setDoc(
      produce((s) => {
        for (const id in s.byId) {
          if (scope.has(s.byId[id].page)) delete s.byId[id]; // drop affected pages' nodes (incl. ones the op added)
        }
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
  for (const p of e.dirty) dirty.add(p);
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

const dirty = new Set<string>();
// Per-page save baseline: the on-disk file rev the editor last loaded or saved.
// Sent on save so the backend conflicts against the version the editor actually
// has, not its own mutable cache (which the watcher can advance under us).
const baseRev = new Map<string, string | null>();
// Pages the user just deleted. A never-saved page can have a queued save with
// baseRev=null; without this, that save fires after the delete and the backend
// (missing file + null baseline = "new page") happily recreates it. While a name
// is tombstoned, saves for it are skipped; re-loading/creating the page clears it.
const deletedPages = new Set<string>();
/** Does a page have unsaved (debounced) edits pending? */
export function isDirty(pageName: string): boolean {
  return dirty.has(pageName);
}
function markDirty(pageName: string) {
  dirty.add(pageName);
  scheduleSave();
}

export function setRaw(id: string, raw: string) {
  pushRawUndo(id, doc.byId[id].raw);
  setDoc("byId", id, "raw", raw);
  markDirty(doc.byId[id].page);
}

export function startEditing(id: string, offset: number, owner: string | null = null) {
  setSelAnchor(null);
  setSelFocus(null);
  setCaretTarget({ id, offset });
  setEditingId(id);
  setEditingOwner(owner);
}

/** Enter: split the block at `offset`. */
// Built-in properties hidden from the editor (like OG): `id::`/`collapsed::` are
// kept in the file for persistence but never shown in the edit textarea. They're
// stripped from the edit view (reattached on commit) and stay with the ORIGINAL
// block across a split.
const HIDDEN_PROP_KEYS = new Set(["id", "collapsed"]);
function propLineKey(line: string): string | null {
  const m = /^\s*([A-Za-z0-9_.\/-]+)::/.exec(line);
  return m ? m[1].toLowerCase() : null;
}
/** Split a block's raw into the editor-visible text (content + user properties)
 *  and the hidden built-in property lines. */
export function splitHiddenProps(raw: string): { visible: string; hidden: string } {
  const vis: string[] = [];
  const hid: string[] = [];
  for (const l of raw.split("\n")) {
    const k = propLineKey(l);
    (k && HIDDEN_PROP_KEYS.has(k) ? hid : vis).push(l);
  }
  return { visible: vis.join("\n"), hidden: hid.join("\n") };
}
export function withHiddenProps(visible: string, hidden: string): string {
  return hidden ? `${visible}\n${hidden}` : visible;
}

export function splitBlock(id: string, offset: number) {
  pushUndo("split", [doc.byId[id].page]);
  const node = doc.byId[id];
  // The caret offset is in editor-visible space (hidden props aren't shown), so
  // split the visible text and keep the hidden props on the original block.
  const { visible, hidden } = splitHiddenProps(node.raw);
  const before = visible.slice(0, offset);
  const after = visible.slice(offset);
  const pageName = node.page;

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
          id: emptyId, raw: "", collapsed: false, parent: node.parent, page: pageName, children: [],
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
      s.byId[id].raw = withHiddenProps(before, hidden);
      const hasVisibleChildren = node.children.length > 0 && !node.collapsed;
      if (hasVisibleChildren) {
        s.byId[newId] = {
          id: newId, raw: after, collapsed: false, parent: id, page: pageName, children: [],
        };
        s.byId[id].children.unshift(newId);
      } else {
        s.byId[newId] = {
          id: newId, raw: after, collapsed: false, parent: node.parent, page: pageName, children: [],
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
      s.byId[newParent].collapsed = false;
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
  const prevSplit = splitHiddenProps(doc.byId[prev].raw);
  const curSplit = splitHiddenProps(node.raw);
  const curVisible = curSplit.visible;
  const joinOffset = prevSplit.visible.length;
  const pageName = node.page;

  // Preserve the absorbed block's `id::` if the survivor has none — otherwise
  // inbound ((id)) references to the absorbed block would orphan on merge.
  let hidden = prevSplit.hidden;
  const survivorHasId = /(?:^|\n)id:: /.test(prevSplit.hidden);
  const absorbedId = /(?:^|\n)(id:: \S+)/.exec(curSplit.hidden)?.[1];
  if (!survivorHasId && absorbedId) {
    hidden = hidden ? `${hidden}\n${absorbedId}` : absorbedId;
  }

  setDoc(
    produce((s) => {
      s.byId[prev].raw = withHiddenProps(prevSplit.visible + curVisible, hidden);
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

/** Toggle a property: set it to `value`, or remove it if already that value. */
export function toggleBlockProperty(id: string, key: string, value: string) {
  setBlockProperty(id, key, blockProperty(id, key) === value ? null : value);
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
export function readSchedule(id: string, which: "scheduled" | "deadline"): { y: number; m: number; d: number } | null {
  const node = doc.byId[id];
  if (!node) return null;
  const tag = which === "scheduled" ? "SCHEDULED" : "DEADLINE";
  const m = new RegExp(`^${tag}:\\s*<(\\d{4})-(\\d{2})-(\\d{2})`, "m").exec(node.raw);
  return m ? { y: +m[1], m: +m[2] - 1, d: +m[3] } : null;
}

/** Set or clear a block's SCHEDULED/DEADLINE org-timestamp (line 2, like OG). */
export function setSchedule(
  id: string,
  which: "scheduled" | "deadline",
  date: { y: number; m: number; d: number } | null
) {
  const node = doc.byId[id];
  if (!node) return;
  pushUndo(`sched:${id}:${which}`, [node.page]);
  const tag = which === "scheduled" ? "SCHEDULED" : "DEADLINE";
  const lines = node.raw.split("\n").filter((l) => !new RegExp(`^${tag}:`).test(l.trim()));
  if (date) {
    const wd = WEEKDAYS[new Date(date.y, date.m, date.d).getDay()];
    const stamp = `${tag}: <${date.y}-${pad2(date.m + 1)}-${pad2(date.d)} ${wd}>`;
    lines.splice(Math.min(1, lines.length), 0, stamp);
  }
  setDoc("byId", id, "raw", lines.join("\n"));
  markDirty(node.page);
}

/** Collapse or expand a block and its entire descendant subtree. */
export function setCollapsedDeep(id: string, collapsed: boolean) {
  pushUndo("collapse-all", [doc.byId[id].page]);
  const walk = (bid: string) => {
    const n = doc.byId[bid];
    if (!n) return;
    if (n.children.length) setDoc("byId", bid, "collapsed", collapsed);
    n.children.forEach(walk);
  };
  walk(id);
  markDirty(doc.byId[id].page);
}

/** Ensure a block has a persistent `id::` uuid (assigned lazily, like OG);
 *  returns the uuid. Used to make `((uuid))` block references. */
export function ensureBlockId(id: string): string {
  const node = doc.byId[id];
  const m = /(?:^|\n)id:: *([0-9a-fA-F-]{8,})/.exec(node.raw);
  if (m) return m[1];
  const uuid = crypto.randomUUID();
  setDoc("byId", id, "raw", `${node.raw}\nid:: ${uuid}`);
  markDirty(node.page);
  // Persist immediately: a ref to this id may be pasted and the app quit before
  // the 400ms debounce fires, which would leave the reference dangling.
  void flushPage(node.page);
  return uuid;
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
 *  resolves after a restart. Used when parking a block in the right sidebar. */
export function persistentBlockRef(id: string): { uuid: string; page: string; pageKind: PageKind } {
  ensureStableBlockId(id);
  return blockRef(id);
}

/** Serialize a block and its subtree to Logseq markdown. */
export function blockSubtreeMarkdown(id: string, level = 0): string {
  const n = doc.byId[id];
  if (!n) return "";
  const tabs = "\t".repeat(level);
  const lines = n.raw.split("\n");
  const out: string[] = [];
  out.push(`${tabs}- ${lines[0] ?? ""}`.replace(/\s+$/, ""));
  for (const line of lines.slice(1)) {
    out.push(line === "" ? "" : `${tabs}  ${line}`);
  }
  for (const c of n.children) out.push(blockSubtreeMarkdown(c, level + 1));
  return out.join("\n");
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
  setSelFocus(order[ni]);
  if (!extend) setSelAnchor(order[ni]);
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
  // The selection can span feed days (pages); the move's target is on first's
  // page, already among the selected pages — so the union of selected pages is
  // the complete affected set.
  pushUndo("indent-sel", [...new Set(ids.map((x) => doc.byId[x]?.page).filter(Boolean) as string[])]);
  for (const id of ids) moveBlockInternal(id, newParent, doc.byId[newParent].children.length);
  setDoc("byId", newParent, "collapsed", false);
}

export function outdentSelection() {
  const ids = topSelected();
  if (!ids.length) return;
  const parentId = doc.byId[ids[0]].parent;
  if (parentId === null) return;
  const grand = doc.byId[parentId].parent;
  // Target `grand` is on ids[0]'s page, already among the selected pages.
  pushUndo("outdent-sel", [...new Set(ids.map((x) => doc.byId[x]?.page).filter(Boolean) as string[])]);
  let after = parentId;
  for (const id of ids) {
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
  return topSelected()
    .map((id) => blockSubtreeMarkdown(id))
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

export function moveBlock(id: string, newParent: string | null, index: number) {
  const node = doc.byId[id];
  if (!node) return;
  // Don't drop a block into its own descendant.
  let p = newParent;
  while (p !== null) {
    if (p === id) return;
    p = doc.byId[p].parent;
  }
  const oldPage = node.page;
  const newPage = newParent ? doc.byId[newParent].page : oldPage;
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
  markDirty(oldPage);
  if (newPage !== oldPage) markDirty(newPage);
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
  markDirty(fromPage);
  markDirty(toPage);
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
    pushUndo("move-sel");
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
    const pages = new Set(ordered.map((id) => doc.byId[id]?.page).filter(Boolean) as string[]);
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
  for (const fp of new Set(plan.map((p) => p.from))) markDirty(fp);
  markDirty(today);
  return plan.length;
}

export function toggleCollapse(id: string) {
  const n = doc.byId[id];
  if (n.children.length === 0) return;
  pushUndo("collapse", [n.page]);
  setDoc("byId", id, "collapsed", !n.collapsed);
  markDirty(n.page);
}

/** Explicitly collapse or expand a block (no-op if it has no children or is
 *  already in the requested state). */
export function setCollapsed(id: string, collapsed: boolean) {
  const n = doc.byId[id];
  if (!n || n.children.length === 0 || n.collapsed === collapsed) return;
  pushUndo("collapse", [n.page]);
  setDoc("byId", id, "collapsed", collapsed);
  markDirty(n.page);
}

// ---------------------------------------------------------------------------
// Debounced persistence
// ---------------------------------------------------------------------------

// Debounced query-recompute trigger: bump dataRev only after edits go quiet, so
// sustained typing doesn't re-run every visible query every save batch.
let dataRevTimer: ReturnType<typeof setTimeout> | null = null;
function scheduleDataRev() {
  if (dataRevTimer) clearTimeout(dataRevTimer);
  dataRevTimer = setTimeout(() => {
    dataRevTimer = null;
    bumpDataRev();
  }, 700);
}

// Bumped whenever the working set is reset (graph switch). A save abandons its
// baseline update if the graph changed under it; resetStore also clears `dirty`
// so a stray queued save becomes a no-op — old-graph content can't reach a new
// graph.
let graphToken = 0;

// Per-page save queue: writes for one page run strictly one-after-another (never
// concurrently) and each runs against the LATEST store state. This means a
// flushPage racing the debounce, or an edit landing during an in-flight write,
// can neither write stale content "last" nor be dropped — the trailing save
// always reflects the final edit and is awaited by flushAll.
const saveChain = new Map<string, Promise<boolean>>();

function enqueueSave(name: string, force = false): Promise<boolean> {
  const prev = saveChain.get(name) ?? Promise.resolve(true);
  const next = prev.then(() => doSave(name, force), () => doSave(name, force));
  saveChain.set(name, next);
  void next.finally(() => {
    if (saveChain.get(name) === next) saveChain.delete(name);
  });
  return next;
}

/** Write the page's CURRENT state once. No-op success if it isn't dirty and not
 *  forced. Sends `baseRev` (the version the editor loaded) so the backend
 *  conflicts against external changes; updates the baseline on success. On a
 *  conflict marks it (no clobber); on a transient error keeps it dirty + toasts. */
async function doSave(name: string, force: boolean): Promise<boolean> {
  if (deletedPages.has(name)) return true; // tombstoned — never recreate a deleted page
  if (!force && !dirty.has(name)) return true; // already saved by a prior link
  if (isConflicted(name) && !force) return false;
  const token = graphToken;
  const dto = pageToDto(name);
  if (!dto) return false;
  dirty.delete(name);
  try {
    const rev = await backend().savePage(dto, baseRev.get(name) ?? null, force);
    if (token === graphToken) baseRev.set(name, rev);
    return true;
  } catch (e) {
    if (String(e).includes("conflict")) {
      markConflict(name);
    } else if (token === graphToken) {
      dirty.add(name); // keep pending — retried on next edit / flush
      pushToast(`Couldn't save “${name}” — will retry. (${String(e)})`, "error");
    }
    return false;
  }
}

let saveTimer: ReturnType<typeof setTimeout> | null = null;
export function scheduleSave() {
  if (!doc.loaded) return;
  if (saveTimer) clearTimeout(saveTimer);
  saveTimer = setTimeout(() => {
    saveTimer = null;
    const names = [...dirty];
    void (async () => {
      const results = await Promise.all(names.map((n) => enqueueSave(n)));
      // The backend cache now reflects these edits → let queries recompute, but
      // coalesce: re-running every on-screen query is a whole-graph scan, so wait
      // for a lull instead of firing on every 400ms save batch.
      if (results.some(Boolean)) scheduleDataRev();
    })();
  }, 400);
}

/** Save one page immediately, bypassing the debounce — for actions that must
 *  durably persist before the user might quit (e.g. parking a block in the
 *  sidebar writes an id:: that has to survive a restart). Returns success. */
export async function flushPage(name: string): Promise<boolean> {
  if (!doc.loaded) return false;
  const ok = await enqueueSave(name);
  if (ok) scheduleDataRev();
  return ok;
}

/** Persist every dirty page now and wait for them (incl. anything mid-write) —
 *  for graph switch / restore / app close. Returns true only if everything
 *  landed (no conflicts or errors), so the caller can abort a destructive
 *  transition rather than discard the un-saved edit. */
export async function flushAll(): Promise<boolean> {
  if (saveTimer) {
    clearTimeout(saveTimer);
    saveTimer = null;
  }
  let landed = false;
  // Drain repeatedly: an edit made WHILE a save is in flight re-dirties the page,
  // and a queued save may still be running, so one pass can miss work. Keep
  // flushing until nothing is pending (bounded against a persistently-failing
  // save).
  for (let i = 0; i < 4; i++) {
    const names = new Set<string>([...dirty, ...saveChain.keys()]);
    if (names.size === 0) break;
    const results = await Promise.all([...names].map((n) => enqueueSave(n)));
    if (results.some(Boolean)) landed = true;
  }
  if (landed) bumpDataRev();
  // Success only if nothing is still pending AND there are no unresolved
  // conflicts (a conflicted page's edit is NOT on disk) — so a destructive
  // transition (graph switch / restore / close) can abort instead of discarding it.
  return dirty.size === 0 && conflicts().length === 0;
}

/** Resolve a save conflict by overwriting the on-disk file with the in-memory
 *  version ("keep mine"). Returns whether the overwrite succeeded — the caller
 *  must not clear the conflict unless it did. */
export async function forceSave(name: string): Promise<boolean> {
  dirty.add(name); // ensure doSave writes even though it's parked as conflicted
  const ok = await enqueueSave(name, true);
  if (!ok) pushToast(`Couldn't overwrite “${name}”.`, "error");
  return ok;
}
