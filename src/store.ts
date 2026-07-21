// The live editing tree. The frontend owns this during a session; all
// keystrokes and structural ops mutate it synchronously (zero IPC). Persistence
// is a debounced per-page save to Rust. See plan §"block editor model".
//
// Supports multiple pages at once (the journals feed): a single global `byId`
// map, each node tagged with its owning `page`, and an ordered `pages` list
// each with its own roots. A single-page route is just a feed of length one.

import { createStore, produce, unwrap } from "solid-js/store";
import { createSignal, createMemo, createRoot } from "solid-js";
import type { BlockDto, Format, PageDto, PageKind, RefGroup } from "./types";
import type { Route } from "./router";
import { parseOutline, type OutlineNode } from "./editor/outline";
import type { ExportNode } from "./editor/exportText";
import { backend } from "./backend";
import {
  isConflicted,
  clearConflict,
  rightSidebar,
  conflicts,
  pushToast,
  graphMeta,
  workflow,
  timetrackingEnabled,
  logbookWithSecondSupport,
  logicalOutdenting,
  removeDeletedPageFromNavigation,
  removeDeletedBlocksFromSidebar,
  bumpDataRev,
  bumpPageInventoryRev,
  captureHistorySidebarContext,
  restoreHistorySidebarContext,
  type HistorySidebarContext,
} from "./ui";
import { seedFacets, facetsFromDto, clearSeededFacets, facetsOf } from "./render/facets";
import { journalTitle } from "./journal";
import { upsertPropertyLine, readPropertyValue, splitProps, joinProps, isBuiltinHidden, isPropertiesOnly, isPageHeaderPropertiesOnly, parsePageHeaderPropertyLine, splitPagePreamble } from "./editor/properties";
import { copyIncludeSubtree, copyStripCollapsed } from "./copySettings";
import { trimBlockTrailingSpace } from "./editor/format";
import { OPEN_MARKERS, MARKER_RE } from "./markers";
import {
  editingId,
  endEdit,
  startEditing,
  captureHistoryEditorContext,
  restoreHistoryEditorContext,
  type HistoryEditorContext,
} from "./editorController";
import { notifyModeReset, notifyOutlineSelectionStarted } from "./modeHooks";
import { sheetConfigFromRaw } from "./sheet/config";
import { clearMatrixDimensionCache, invalidateAllMatrixDimensions } from "./sheet/matrix";
import { applyMarkerTransition } from "./logbook";
import { cycleMarkerSmart } from "./editor/repeat";
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
  holdSourcesForDest,
  trackAssetWrite,
} from "./persistence";
// The debounced persistence engine lives in persistence.ts; re-exported here so
// the rest of the app keeps importing the save API from the store.
export { markDirty, isDirty, isSaving, scheduleSave, flushPage, flushAll, forceSave, trackAssetWrite };

export interface Node {
  id: string;
  raw: string;
  collapsed: boolean;
  parent: string | null; // null = a root of its page
  page: string; // owning page name
  children: string[];
  /** Frontend-only editing provenance for an existing unbulleted Markdown page
   * header. Spread-based undo snapshots retain it; DTO serialization consumes
   * it and never sends it over the wire. */
  originatedFromPageHeader?: boolean;
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
  /** Bundled in-app Guide page: read-only and ephemeral. */
  guide: boolean;
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

// name → index into `doc.pages`, rebuilt only when the working set's membership
// changes (add / remove / rename / evict), NOT on a keystroke. Turns the O(pages)
// linear `find` in `pageByName`/`formatForPage`/`mainPages` — which run in the
// per-block render hot path and ~7×/page render — into an O(1) lookup. We map to
// the index (not the proxy) and read `doc.pages[idx]` live, so a property change
// (roots/preBlock/format) stays fine-grained-reactive and the index never goes
// stale: the memo re-derives whenever any page's `name` or the array length moves.
const pageIndexByName = createRoot(() =>
  createMemo(() => {
    const m = new Map<string, number>();
    doc.pages.forEach((p, i) => m.set(p.name, i));
    return m;
  })
);

/** The pages shown in the main content area, in feed order. Memoized: the O(feed)
 *  resolve runs once per structural change, not on each of its ~7 calls per render. */
export const mainPages = createRoot(() =>
  createMemo((): FeedPage[] => {
    const idx = pageIndexByName();
    return doc.feed
      .map((n) => {
        const i = idx.get(n);
        return i === undefined ? undefined : doc.pages[i];
      })
      .filter(Boolean) as FeedPage[];
  })
);

/** A loaded page record by name (anywhere in the working set), or undefined. */
export function pageByName(name: string): FeedPage | undefined {
  const i = pageIndexByName().get(name);
  return i === undefined ? undefined : doc.pages[i];
}

/** The format ("md"/"org") to parse a page's inline content with. Exact for a
 *  loaded page; for one that isn't loaded (e.g. the source of a backlink) fall back
 *  to the graph's preferred format — correct for single-format graphs, a safe guess
 *  otherwise (and far better than always assuming Markdown). Used by the inline
 *  renderers (InlineText callers) so org markup in property values / breadcrumbs /
 *  reference previews / block-refs renders as org, not literally. */
export function formatForPage(name: string | undefined): Format {
  if (name) {
    const p = pageByName(name);
    if (p?.format) return p.format;
  }
  return graphMeta()?.preferred_format ?? "md";
}

/** Like {@link formatForPage} but keyed by a block id (→ its page). */
export function formatForBlock(id: string | undefined): Format {
  return formatForPage(id ? doc.byId[id]?.page : undefined);
}

export function blockIsGridView(id: string | undefined): boolean {
  const n = id ? doc.byId[id] : undefined;
  return !!n && sheetConfigFromRaw(n.raw, formatForBlock(id)).view === "grid";
}

function blockIsOpaqueSheetView(id: string | undefined): boolean {
  const n = id ? doc.byId[id] : undefined;
  const view = n ? sheetConfigFromRaw(n.raw, formatForBlock(id)).view : null;
  return view === "grid" || view === "table" || view === "board";
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
  byId: Record<string, Node>,
  format: Format
): string[] {
  return dtos.map((d) => {
    // Seed the header-facet cache from the backend (one Rust lsdoc parse, shipped) so
    // the rendered chip reads off the DTO — zero frontend parse on load (M1 / P1).
    seedFacets(d.raw, format, facetsFromDto(d));
    // Cross-page id:: collision guard: if another LOADED page already owns this
    // id (two files share a persisted `id::` — copy-pasted raw, or a sync hiccup),
    // give this block a fresh store key instead of overwriting the other page's
    // node. Without this, the global byId entry is clobbered and saving one page
    // serializes the other's content. The block's raw (incl. its id:: line) is
    // untouched, so the file on disk is unchanged. Rust dedups ids WITHIN a page,
    // so this only fires across pages.
    const existing = byId[d.id];
    const key = existing && existing.page !== pageName ? `dup~${crypto.randomUUID()}` : d.id;
    const childIds = flatten(d.children, key, pageName, byId, format);
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
  const roots = flatten(dto.blocks, null, dto.name, byId, dto.format ?? "md");
  return {
    name: dto.name,
    kind: dto.kind,
    title: dto.title,
    preBlock: dto.pre_block,
    roots,
    format: dto.format ?? "md",
    readOnly: dto.read_only ?? false,
    guide: dto.guide ?? false,
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
  invalidateAllMatrixDimensions();
  if (replacing) invalidateUndoForPage(dto.name);
}

/** Whether a reload DTO carries the SAME content (page-property pre-block + every
 *  block's raw + tree shape, ignoring block ids) as the page already in memory —
 *  i.e. a self-write echo, not a real external change. Lets `upsertPage` skip a
 *  needless reload that would otherwise reset block identities and invalidate the
 *  undo history for content we already hold. */
function pageContentMatches(dto: PageDto, page: FeedPage): boolean {
  if ((dto.path ?? "") !== (page.path ?? "")) return false;
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
  const existing = doc.pages.find((p) => p.name === dto.name);
  if (existing && (existing.path ?? "") === (dto.path ?? "")) return;
  // A path-pinned route may intentionally load a duplicate-day stray with the
  // same logical title as the canonical journal. Replace the name slot with the
  // exact requested file instead of silently keeping (and then editing/saving)
  // the canonical file. Full simultaneous duplicate identity is tracked by the
  // file-identity ADR; this closes the wrong-target write immediately.
  upsertPage(dto);
  evictIfNeeded();
}

/** Load/reload bundled Guide pages into the working set without making them the
 *  main feed. Re-open uses this to re-derive the read-only virtual pages from
 *  the backend templates instead of trusting stale in-memory copies. */
export function loadGuidePages(dtos: PageDto[]) {
  for (const dto of dtos) {
    upsertPage({ ...dto, read_only: true, guide: true });
  }
  evictIfNeeded();
}

export function isGuidePage(name: string): boolean {
  return pageByName(name)?.guide ?? false;
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
  invalidateAllMatrixDimensions();
}

/** Delete a page: tombstone it (so any pending/in-flight save can't recreate the
 *  file), drop its dirty/baseline/conflict state, remove it from the working set
 *  and feed, then delete on disk. Routing deletion through the store — rather than
 *  calling the backend directly — is what prevents a queued baseRev=null save from
 *  resurrecting a just-typed, never-saved page. Returns backend success. */
export async function deletePage(name: string, kind: PageKind, expectedPath?: string): Promise<boolean> {
  const loaded = pageByName(name);
  if (expectedPath && loaded?.path !== expectedPath) return false;
  if (loaded?.readOnly || loaded?.guide) return false;
  // Capture the current (possibly unsaved) content first, so the recoverable trash
  // copy is the LATEST version — not the stale bytes on disk. A CONFLICTED page can
  // never flush (its save stays refused until the conflict is resolved); blocking
  // the delete on that flush made such a page *undeletable* — the user could neither
  // save nor discard it. Deleting is itself a resolution ("I don't want this page"),
  // and the on-disk version still lands in .tine-trash (recoverable), so a conflict
  // must not veto the delete. For a merely-dirty page we still flush first (to trash
  // the latest bytes) and abort only if that genuinely fails.
  if (isDirty(name) && !isConflicted(name) && !(await flushPage(name))) return false;
  // Tombstone first so any queued/in-flight save no-ops during the delete, but
  // DON'T drop the in-memory page until the backend actually deletes it — if the
  // delete fails, the page (and its unsaved edits) must survive.
  tombstone(name);
  try {
    if (expectedPath) await backend().deletePage(name, kind, expectedPath);
    else await backend().deletePage(name, kind);
  } catch {
    untombstone(name); // delete failed — lift the tombstone; page + edits stay intact
    return false;
  }
  forgetPage(name); // success — now drop it from the working set + feed
  removeDeletedPageFromNavigation({ name, pageKind: kind, ...(expectedPath ? { path: expectedPath } : {}) });
  // A page delete changes every live query / backlink result (the backend already
  // dropped its derived cache + bumped cache_gen in delete_page). Nudge dataRev so
  // open {{query}} panels re-run and drop the deleted page's rows — otherwise they
  // keep showing the stale cached result (only the block whose node was purged from
  // byId visibly disappears, leaving the rest of the deleted page's rows behind).
  bumpDataRev();
  bumpPageInventoryRev();
  return true;
}

// Cap the working set so a long session browsing a big graph doesn't grow byId
// without bound. FIFO-evict pages that aren't pinned: the main feed, anything
// open in the right sidebar, the page being edited, and any page with unsaved
// edits are all kept (evicting a dirty page would lose those edits).
const WORKING_SET_CAP = 80;
let paneRouteProvider: () => Route[] = () => [];
export function registerPaneRouteProvider(provider: () => Route[]) {
  paneRouteProvider = provider;
}
function pinnedPages(): Set<string> {
  const pin = new Set<string>(doc.feed);
  for (const r of paneRouteProvider()) {
    if (r.kind === "page") pin.add(r.name);
  }
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
  invalidateAllMatrixDimensions();
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
  // Drop the old graph's seeded facets (the never-evicted tier) so they don't linger
  // across the switch (audit P2).
  clearSeededFacets();
  clearMatrixDimensionCache();
  setDoc({ byId: {}, pages: [], feed: [], loaded: false });
  endEdit("graph-switch");
  notifyModeReset();
}

// A navigation/feed load must NOT replace a page that has unsaved edits (or an
// unresolved conflict) with a fresh disk DTO — e.g. you edited it in the sidebar,
// then opened it in the main view before the debounce saved. Keep the live dirty
// nodes; the disk version would otherwise be served and the next save could write
// it, silently dropping the edit. (reloadPage / "use disk version" still replace
// explicitly via upsertPage.)
function upsertUnlessDirty(dto: PageDto) {
  // `isSaving` too — an in-flight save's edit isn't durable yet (audit H1).
  if (pageByName(dto.name) && (isDirty(dto.name) || isConflicted(dto.name) || isSaving(dto.name)))
    return;
  upsertPage(dto);
}

export type ReloadDisposition = "reload" | "conflict" | "skip";
/** What to do when page `name` changed on disk (external editor / Syncthing),
 *  for the file-watcher reload sites. One rule so the (formerly 4 hand-coded)
 *  branches in Page.tsx can't diverge:
 *  - `"conflict"` — it has unsaved edits / an open conflict: surface a conflict,
 *    NEVER clobber the in-memory edit with the disk version.
 *  - `"skip"` — a block on it is being edited (don't yank the caret) or a block
 *    move is mid-flight (the textarea is transiently blurred): leave it alone.
 *  - `"reload"` — safe to replace the loaded copy with the disk version.
 *  (Navigation/flush-first paths — upsertUnlessDirty, reloadHlsIfLoaded — use a
 *  simpler dirty-only guard on purpose and do not go through this.) */
export function reloadDisposition(name: string): ReloadDisposition {
  // `isSaving` too: `doSave` clears `dirty` BEFORE the `await savePage`, so during the
  // save IPC the page is no longer dirty but its edit isn't durable. Reloading then
  // would clobber the in-memory edit + drop its undo, and the in-flight save would
  // conflict — silent loss (audit H1). The in-flight save's baseRev check surfaces the
  // real conflict.
  if (isDirty(name) || isConflicted(name) || isSaving(name)) return "conflict";
  const ed = editingId();
  if ((ed && doc.byId[ed]?.page === name) || isBlockMoving()) return "skip";
  return "reload";
}

/** Load a single page and make it the main view. */
export function loadSingle(dto: PageDto, opts: { endEdit?: boolean } = {}) {
  upsertUnlessDirty(dto);
  setDoc("feed", [dto.name]);
  setDoc("loaded", true);
  if (opts.endEdit !== false) endEdit("page-navigation");
  evictIfNeeded();
}

/** Load the journals feed as the main view. */
export function loadFeed(dtos: PageDto[], opts: { endEdit?: boolean } = {}) {
  for (const d of dtos) upsertUnlessDirty(d);
  setDoc("feed", dtos.map((d) => d.name));
  setDoc("loaded", true);
  if (opts.endEdit !== false) endEdit("page-navigation");
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

/** A fresh, empty (unsaved) page: one editable blank block. Used for a page that
 *  doesn't exist on disk yet — the file is written lazily on first save. Shared by
 *  the feed loader (today's placeholder), single-page open, and the post-delete
 *  today restore, so the empty-page shape has ONE definition. */
export function emptyPage(name: string, kind: "journal" | "page"): PageDto {
  return {
    name,
    kind,
    title: name,
    pre_block: null,
    blocks: [{ id: `new-${name}`, raw: "", collapsed: false, children: [] }],
  };
}

/** Re-assert "the journals feed always shows today" on the LIVE feed after today's
 *  journal is deleted from it. The feed loader's `withToday` only runs on (re)load,
 *  so deleting today in place while viewing the feed would otherwise leave the top
 *  blank until you navigate away and back (#17). No-op if today is still in the feed
 *  (e.g. it was an OLDER day that got deleted). The placeholder is empty and
 *  writable — `upsertPage` lifts the delete tombstone, so the first keystroke saves
 *  a fresh file, exactly like reopening the journal. */
export function restoreTodayJournalInFeed() {
  const title = journalTitle(new Date());
  if (doc.feed.includes(title)) return;
  upsertUnlessDirty(emptyPage(title, "journal"));
  setDoc("feed", [title, ...doc.feed]);
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

/** Mirror of Rust `first_root_is_promotable_page_header` (model.rs): a childless
 *  first root whose raw is exactly canonical page-header properties and carries
 *  no `id::` line (an id-bearing block is a real referenced outline block, not a
 *  header, and the Rust promote branch/firewall both leave it as a bullet). */
function isPromotablePageHeaderRoot(node: Node): boolean {
  const canonicalRaw = node.raw.replace(/\n+$/, "");
  return (
    node.children.length === 0 &&
    isPageHeaderPropertiesOnly(canonicalRaw) &&
    !canonicalRaw.split("\n").some((line) => parsePageHeaderPropertyLine(line)?.key.toLowerCase() === "id")
  );
}

export function pageToDto(pageName: string): PageDto | null {
  const p = doc.pages.find((x) => x.name === pageName);
  if (!p) return null;
  let rootIds = p.roots;
  let preBlock = p.preBlock;
  const first = doc.byId[rootIds[0]];
  if (first?.originatedFromPageHeader) {
    // Enter temporarily leaves one or more trailing newlines in the live
    // page-header editor. Tolerate only that authoring artifact at the disk
    // firewall; keep the strict shared display predicate and live raw intact.
    const canonicalRaw = first.raw.replace(/\n+$/, "");
    if (first.children.length > 0 || (first.raw !== "" && !isPageHeaderPropertiesOnly(canonicalRaw))) {
      pushToast("Page-header properties must contain only valid key:: value lines before they can be saved.", "error");
      return null;
    }
    // Exact raw is authoritative here: ordinary toDto trimming must never eat a
    // page-header value or its separator trivia. An empty draft deletes the
    // header and emits no stray outline bullet.
    preBlock = canonicalRaw ? canonicalRaw + (p.preBlock ?? "") : p.preBlock;
    rootIds = rootIds.slice(1);
  } else if (first && !p.preBlock && isPromotablePageHeaderRoot(first)) {
    // GH #198: a flagless "properties-only first bullet" (empty preBlock) IS the
    // page header — the same shape setPageProperty/beginPageHeaderEdit already
    // treat as the header. Fold it into pre_block so the DTO is honest, instead
    // of leaning on the Rust promote branch: once disk already carries the
    // promoted preamble, the GH #163 preservation firewall refuses the
    // pre_block=None + first-root-properties DTO and jams the save queue with a
    // "will retry" toast forever. Folding here emits pre_block=properties, so
    // the firewall precondition (empty pre_block) is false and the save writes
    // the identical canonical preamble. Mirrors Rust's promotability rule.
    preBlock = first.raw.replace(/\n+$/, "");
    rootIds = rootIds.slice(1);
  }
  let blocks = rootIds.map(toDto);
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
    pre_block: preBlock,
    blocks,
    format: p.format,
    // Pin the save to the exact file this page came from (#21). Absent for a
    // brand-new page → the backend resolves the file by name, as before.
    path: p.path,
    guide: p.guide,
    read_only: p.readOnly,
  };
}

// ---------------------------------------------------------------------------
// Virtual-guide resolution
//
// The in-app Guide is virtual — its pages live only in this store, never on
// disk — so the backend `((uuid))` / `{{embed [[page]]}}` resolvers (which scan
// the on-disk graph) can't see them. These fall back to the LOADED guide pages
// and are consulted ONLY on a backend miss, so a real-graph ref/embed always
// prefers the disk resolver and these never shadow it.
// ---------------------------------------------------------------------------

/** The block id (`id:: <uuid>` trailer) a guide node exposes to `((uuid))`
 *  references — matching the backend, which keys a block by its persisted id::. */
function guideBlockDurableId(raw: string): string | null {
  const m = /(?:^|\n)id:: *(\S+)/i.exec(raw);
  return m ? m[1] : null;
}

function findGuideNode(ids: string[], uuid: string): string | null {
  for (const id of ids) {
    const n = doc.byId[id];
    if (!n) continue;
    if (id === uuid || guideBlockDurableId(n.raw) === uuid) return id;
    const child = findGuideNode(n.children, uuid);
    if (child) return child;
  }
  return null;
}

/** Resolve a `((uuid))` block reference / block embed against the loaded guide
 *  pages. Returns null for any id not owned by a loaded guide page, so real
 *  refs fall through to the backend/disk resolver unchanged. */
export function resolveGuideBlockRef(uuid: string): RefGroup | null {
  for (const p of doc.pages) {
    if (!p.guide) continue;
    const hit = findGuideNode(p.roots, uuid);
    if (hit) return { page: p.name, kind: p.kind, blocks: [toDto(hit)] };
  }
  return null;
}

/** Serialize a loaded guide page (matched by its bare title, e.g.
 *  "Features/Tips & shortcuts") to a PageDto for in-app `{{embed [[page]]}}` —
 *  the embed macro carries no source context to remap the name, so we match on
 *  title. Null for non-guide/unloaded titles → the backend/disk path wins. */
export function resolveGuidePageDto(title: string): PageDto | null {
  const p = doc.pages.find((x) => x.guide && x.title === title);
  return p ? pageToDto(p.name) : null;
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
        if (n && !n.collapsed && n.children.length && !blockIsOpaqueSheetView(id)) walk(n.children);
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
export function pageVisibleOrder(pageName: string): string[] {
  const order: string[] = [];
  const page = doc.pages.find((p) => p.name === pageName);
  if (!page) return order;
  const walk = (ids: string[]) => {
    for (const id of ids) {
      order.push(id);
      const n = doc.byId[id];
      if (n && !n.collapsed && n.children.length && !blockIsOpaqueSheetView(id)) walk(n.children);
    }
  };
  walk(page.roots);
  return order;
}

/** Model-only description of the outline currently rendered around a block.
 * Zoom uses a single root whose durable collapse is overridden for this view. */
export interface OutlineScope {
  roots: string[];
  forceExpandedRoot?: string;
}

function scopedVisibleOrder(scope: OutlineScope): string[] {
  const order: string[] = [];
  const walk = (ids: readonly string[]) => {
    for (const id of ids) {
      const node = doc.byId[id];
      if (!node) continue;
      order.push(id);
      const expanded = !node.collapsed || id === scope.forceExpandedRoot;
      if (expanded && node.children.length && !blockIsOpaqueSheetView(id)) walk(node.children);
    }
  };
  walk(scope.roots);
  return order;
}

/** The only trailing-block reuse candidate for a rendered outline boundary.
 * The caller must supply the actual page or zoom scope so journal days cannot
 * cross-select each other. A collapsed parent and an opaque Sheet host remain
 * visible terminal rows, but their storage children mean neither is a leaf. */
export function trailingVisibleEmptyLeaf(scope: OutlineScope): string | null {
  const id = scopedVisibleOrder(scope).at(-1);
  if (!id) return null;
  const node = doc.byId[id];
  if (!node || node.children.length !== 0) return null;
  return splitProps(node.raw, isBuiltinHidden, formatForBlock(id)).visible.trim() === "" ? id : null;
}

let activeSelectionScope: OutlineScope | null = null;

/** Visible order to resolve a block SELECTION against. The journals feed lives in
 *  visibleData(); a routed single page is loaded via ensurePageLoaded and is NOT in
 *  doc.feed, so its blocks aren't in visibleOrder() — fall back to that block's own
 *  page order, mirroring prevVisible/nextVisible. Without this, block-select (Esc,
 *  Arrow, Shift+Arrow) is dead on any routed page / reference / embed. */
function selectionOrder(id: string | null, scope: OutlineScope | null = activeSelectionScope): string[] {
  if (!id) return [];
  if (scope) return scopedVisibleOrder(scope);
  if (visibleData().index.has(id)) return visibleOrder();
  const page = doc.byId[id]?.page;
  return page ? pageVisibleOrder(page) : [];
}

export function prevVisible(id: string, scope: OutlineScope | null = null): string | null {
  if (scope) {
    const order = scopedVisibleOrder(scope);
    const i = order.indexOf(id);
    return i > 0 ? order[i - 1] : null;
  }
  const { order, index } = visibleData();
  const i = index.get(id);
  if (i !== undefined) return i > 0 ? order[i - 1] : null;
  const node = doc.byId[id];
  if (!node) return null;
  const ord = pageVisibleOrder(node.page);
  const j = ord.indexOf(id);
  return j > 0 ? ord[j - 1] : null;
}

export function nextVisible(id: string, scope: OutlineScope | null = null): string | null {
  if (scope) {
    const order = scopedVisibleOrder(scope);
    const i = order.indexOf(id);
    return i >= 0 && i < order.length - 1 ? order[i + 1] : null;
  }
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
  context: HistoryContext;
}
interface RawEntry {
  kind: "raw";
  id: string;
  raw: string; // the block's text to restore
  page: string;
  /** A transient page-header node can legitimately disappear when its text is
   * deleted. Carry the structural shell on its normal O(1) typing undo entry so
   * Undo can restore it and Redo can remove it again without an extra step. */
  headerRoot?: { node: Node; rootIndex: number };
  removeHeaderOnApply?: boolean;
  context: HistoryContext;
}
type UndoEntry = SnapEntry | RawEntry;
const undoStack: UndoEntry[] = [];
let redoStack: UndoEntry[] = [];
let lastUndoTag: string | null = null;
let undoSuppressionDepth = 0;

// Session-scoped and global by default, matching OG's transient app-state flag
// at `src/main/frontend/state.cljs:304-306` (OG commit 6e7afa8eb).
let pageOnlyHistoryMode = false;

export function historyPageOnlyMode(): boolean {
  return pageOnlyHistoryMode;
}

export function toggleUndoRedoMode(): "Page only" | "Global" {
  pageOnlyHistoryMode = !pageOnlyHistoryMode;
  return pageOnlyHistoryMode ? "Page only" : "Global";
}

export interface HistoryRouteContext {
  paneId: string;
  route: Route;
}

let historyRouteContextAdapter: {
  capture: () => HistoryRouteContext | null;
  restore: (context: HistoryRouteContext) => boolean;
} = {
  capture: () => null,
  restore: () => false,
};

/** Router-owned adapter: keeps store.ts from adding a runtime import back to
 * router.ts (router already imports the store). */
export function installHistoryRouteContextAdapter(adapter: typeof historyRouteContextAdapter) {
  historyRouteContextAdapter = adapter;
}

interface HistoryContext {
  route: HistoryRouteContext | null;
  sidebar: HistorySidebarContext;
  editor: HistoryEditorContext | null;
}

/** Capture UI state at the same pre-mutation boundary as the data inverse. OG
 * stores app state on each history entity and cursor state by transaction at
 * `src/main/frontend/modules/editor/undo_redo.cljs:261-272` and
 * `src/main/frontend/modules/outliner/datascript.cljc:152-162`
 * (OG commit 6e7afa8eb). */
function captureHistoryContext(): HistoryContext {
  return {
    route: historyRouteContextAdapter.capture(),
    sidebar: captureHistorySidebarContext(),
    editor: captureHistoryEditorContext(),
  };
}

/** Discard all undo/redo history. Called on graph switch/reset so old-graph
 *  snapshots can't be replayed into a different graph. */
export function clearUndoHistory() {
  undoStack.length = 0;
  redoStack = [];
  lastUndoTag = null;
  undoSuppressionDepth = 0;
}

/** Does an undo entry reference page `name`? A raw entry by its `page`; a snap
 *  entry by its declared scope (a `null` scope = whole working set, so it touches
 *  every page including this one). */
function entryTouchesPage(e: UndoEntry, name: string): boolean {
  if (e.kind === "raw") return e.page === name;
  return e.pages === null || e.pages.includes(name);
}

/** The page owning the active editor wins over the focused pane's route. This is
 * OG's current/editing-page precedence at
 * `src/main/frontend/util/page.cljs:14-29` (OG commit 6e7afa8eb). */
function activeHistoryPage(): string | null {
  const id = editingId();
  const edited = id ? doc.byId[id] : undefined;
  if (edited) return edited.page;
  const route = historyRouteContextAdapter.capture()?.route;
  return route?.kind === "page" ? route.name : null;
}

/** Remove the newest matching entry in place while retaining every other entry
 * in its original order. This transcribes OG's filtered stack removal at
 * `src/main/frontend/modules/editor/undo_redo.cljs:81-106,132-156`
 * (OG commit 6e7afa8eb). */
function popNewestEntryForPage(stack: UndoEntry[], page: string): UndoEntry | undefined {
  for (let i = stack.length - 1; i >= 0; i--) {
    if (entryTouchesPage(stack[i], page)) return stack.splice(i, 1)[0];
  }
  return undefined;
}

function popHistoryEntry(stack: UndoEntry[]): UndoEntry | undefined {
  if (!stack.length) return undefined;
  if (!pageOnlyHistoryMode) return stack.pop();
  const page = activeHistoryPage();
  return page ? popNewestEntryForPage(stack, page) : stack.pop();
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
// Spread-based so a newly-added FeedPage/Node field can't be silently dropped from
// an undo snapshot (the trap that lost `path` — added for the #21 duplicate-day
// stray and read by pageToDto to pin the save to the exact file — so an undo/redo
// of a path-pinned page misrouted its next save to the canonical file). The only
// per-field work is deep-copying the one array each carries.
function cloneNode(n: Node): Node {
  return { ...n, children: n.children.slice() };
}
function clonePages(src: FeedPage[]): FeedPage[] {
  return src.map((p) => ({ ...p, roots: p.roots.slice() }));
}
function snapEntry(affected?: string[] | null): SnapEntry {
  const context = captureHistoryContext();
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
  return { kind: "snap", pages: affected ?? null, pageObjs, nodes, dirty: names, context };
}

/** Snapshot before a STRUCTURAL op. Pass the affected page name(s) so both the
 *  snapshot AND the undo re-save are scoped to just those pages; omit only when
 *  the op's page set isn't known (falls back to the whole working set — correct
 *  but O(loaded pages)). The affected set MUST include every page whose nodes the
 *  op changes, including a cross-page move's source AND destination, or undo
 *  would miss a page. `tag` resets the typing-coalesce marker. */
function pushUndo(tag: string, affected?: string[]) {
  if (undoSuppressionDepth > 0) return;
  undoStack.push(snapEntry(affected));
  if (undoStack.length > 200) undoStack.shift();
  redoStack = [];
  lastUndoTag = tag;
}

/** Record an O(1) inverse patch for a single-block text edit (typing). A typing
 *  burst in one block coalesces to a single entry holding the pre-burst text. */
function pushRawUndo(id: string, prevRaw: string) {
  if (undoSuppressionDepth > 0) return;
  const tag = `type:${id}`;
  if (tag === lastUndoTag) return; // mid-burst: keep the first (pre-burst) raw
  const node = doc.byId[id];
  const rootIndex = node.originatedFromPageHeader
    ? (pageByName(node.page)?.roots.indexOf(id) ?? -1)
    : -1;
  undoStack.push({
    kind: "raw",
    id,
    raw: prevRaw,
    page: node.page,
    context: captureHistoryContext(),
    ...(rootIndex >= 0 ? { headerRoot: { node: cloneNode(node), rootIndex } } : {}),
  });
  if (undoStack.length > 200) undoStack.shift();
  redoStack = [];
  lastUndoTag = tag;
}

/** Apply one entry and return its inverse (to push onto the opposite stack). */
function applyEntry(e: UndoEntry): UndoEntry {
  if (e.kind === "raw") {
    const node = doc.byId[e.id];
    const rootIndex = node?.originatedFromPageHeader
      ? (pageByName(node.page)?.roots.indexOf(e.id) ?? -1)
      : -1;
    const inverse: RawEntry = {
      kind: "raw",
      id: e.id,
      raw: node ? node.raw : "",
      page: e.page,
      context: captureHistoryContext(),
      ...(node && rootIndex >= 0 ? { headerRoot: { node: cloneNode(node), rootIndex } } : {}),
    };
    if (node) {
      if (e.removeHeaderOnApply && node.originatedFromPageHeader) {
        setDoc(produce((s) => {
          const page = s.pages.find((p) => p.name === node.page);
          if (page) page.roots = page.roots.filter((id) => id !== e.id);
          delete s.byId[e.id];
        }));
        inverse.headerRoot = { node: cloneNode(node), rootIndex: Math.max(0, rootIndex) };
      } else {
        setDoc("byId", e.id, "raw", e.raw);
      }
      addDirty(e.page);
    } else if (e.headerRoot) {
      const restored = { ...cloneNode(e.headerRoot.node), raw: e.raw };
      setDoc(produce((s) => {
        s.byId[e.id] = restored;
        const page = s.pages.find((p) => p.name === e.page);
        if (page) page.roots.splice(Math.min(e.headerRoot!.rootIndex, page.roots.length), 0, e.id);
      }));
      inverse.headerRoot = { node: cloneNode(restored), rootIndex: e.headerRoot.rootIndex };
      inverse.removeHeaderOnApply = true;
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
  invalidateAllMatrixDimensions();
  return inverse;
}

export function withUndoUnit<T>(tag: string, pages: string[], fn: () => T): T {
  if (pages.some((page) => pageByName(page) && !pageWritable(page))) return undefined as T;
  if (undoSuppressionDepth > 0) return fn();

  const undoBefore = undoStack.slice();
  const redoBefore = redoStack.slice();
  const tagBefore = lastUndoTag;
  pushUndo(tag, pages);
  undoSuppressionDepth++;
  try {
    return fn();
  } catch (err) {
    undoSuppressionDepth--;
    const entry = undoStack[undoStack.length - 1];
    if (entry) applyEntry(entry);
    undoStack.length = 0;
    undoStack.push(...undoBefore);
    redoStack = redoBefore;
    lastUndoTag = tagBefore;
    throw err;
  } finally {
    if (undoSuppressionDepth > 0) undoSuppressionDepth--;
  }
}

export function undo() {
  const entry = popHistoryEntry(undoStack);
  if (!entry) return;
  redoStack.push(applyEntry(entry));
  lastUndoTag = null;
  endEdit("undo");
  scheduleSave();
  restoreEntryContext(entry.context);
}

export function redo() {
  const entry = popHistoryEntry(redoStack);
  if (!entry) return;
  undoStack.push(applyEntry(entry));
  lastUndoTag = null;
  endEdit("redo");
  scheduleSave();
  restoreEntryContext(entry.context);
}

/** Data replay and opposite-stack insertion are complete before this function is
 * reached. Each UI step is isolated and best-effort, so a missing pane, route,
 * sidebar surface, or block cannot undo/reorder the already-applied inverse.
 * OG's restore order and global-mode app-state gate are at
 * `src/main/frontend/handler/history.cljs:10-60` (OG commit 6e7afa8eb). */
function restoreEntryContext(context: HistoryContext) {
  if (!pageOnlyHistoryMode) {
    if (context.route) {
      try {
        historyRouteContextAdapter.restore(context.route);
      } catch {
        // Route restoration is best-effort; content replay has already completed.
      }
    }
    try {
      restoreHistorySidebarContext(context.sidebar);
    } catch {
      // Sidebar restoration is best-effort; content replay has already completed.
    }
  }
  if (context.editor) {
    try {
      const node = doc.byId[context.editor.blockId];
      restoreHistoryEditorContext(context.editor, node ? node.raw.length : null);
    } catch {
      // Focus/caret restoration is best-effort; content replay has already completed.
    }
  }
}

// ---------------------------------------------------------------------------
// Mutations (each schedules a debounced save of the affected page)
// ---------------------------------------------------------------------------

export function setRaw(id: string, raw: string, opts?: { timetracking?: boolean }) {
  if (!blockWritable(id)) return;
  const prev = doc.byId[id].raw;
  const next =
    opts?.timetracking === false
      ? raw
      : applyMarkerTransition(
          prev,
          raw,
          formatForBlock(id),
          timetrackingEnabled(),
          logbookWithSecondSupport(),
        );
  pushRawUndo(id, prev);
  setDoc("byId", id, "raw", next);
  markDirty(doc.byId[id].page);
}

export function insertEmptyChildBlock(parentId: string, at: number): string | null {
  const parent = doc.byId[parentId];
  if (!parent || !blockWritable(parentId) || at < 0 || at > parent.children.length) return null;
  pushUndo(`insert-child:${parentId}`, [parent.page]);
  const id = freshId();
  const pageName = parent.page;
  setDoc(
    produce((s) => {
      s.byId[id] = { id, raw: "", collapsed: false, parent: parentId, page: pageName, children: [] };
      s.byId[parentId].children.splice(at, 0, id);
    })
  );
  markDirty(pageName);
  return id;
}

/** Replace child ordering for existing blocks under existing parents.
 *  Callers must pass permutations of existing child ids; this helper owns the
 *  produce-level tree write so higher-level sheet code stays out of store shape. */
export function replaceChildOrders(nextByParent: Record<string, readonly string[]>): boolean {
  const parentIds = Object.keys(nextByParent);
  if (!parentIds.length) return false;
  const pages = new Set<string>();
  for (const parentId of parentIds) {
    const parent = doc.byId[parentId];
    if (!parent || !blockWritable(parentId)) return false;
    pages.add(parent.page);
    for (const childId of nextByParent[parentId]) {
      const child = doc.byId[childId];
      if (!child || child.page !== parent.page) return false;
    }
  }
  pushUndo("replace-child-orders", [...pages]);
  setDoc(
    produce((s) => {
      for (const parentId of parentIds) {
        const next = [...nextByParent[parentId]];
        s.byId[parentId].children = next;
        for (const childId of next) s.byId[childId].parent = parentId;
      }
    })
  );
  for (const pageName of pages) markDirty(pageName);
  return true;
}

/** Append parsed outline blocks as children of `parentId`.
 *  Shared by normal editor paste (via parseOutline) and sheet indented paste. */
export function insertOutlineChildren(parentId: string, nodes: OutlineNode[]): string | null {
  if (!nodes.length) return null;
  const parent = doc.byId[parentId];
  if (!parent || !blockWritable(parentId)) return null;
  const pageName = parent.page;
  let lastId: string | null = null;
  pushUndo("paste-children", [pageName]);
  const format = formatForPage(pageName);
  setDoc(
    produce((s) => {
      const create = (n: OutlineNode, par: string): string => {
        const id = freshId();
        const childIds = n.children.map((c) => create(c, id));
        s.byId[id] = {
          id,
          raw: rawWithInheritedOrderListType(n.raw, format, parentId),
          collapsed: false,
          parent: par,
          page: pageName,
          children: childIds,
        };
        return id;
      };
      const created = nodes.map((n) => create(n, parentId));
      s.byId[parentId].children.push(...created);
      lastId = created[created.length - 1] ?? null;
    })
  );
  markDirty(pageName);
  return lastId;
}

/** Enter: split the block at `offset`. Built-in `id::`/`collapsed::` props are
 *  hidden from the editor (see editor/properties splitProps): the caret offset is
 *  in visible space, and hidden props stay with the ORIGINAL block across a split. */
export function splitBlock(
  id: string,
  offset: number,
  forceChild: boolean = false,
  keepStartInScope: boolean = false,
  editingSurface: string | null = null,
) {
  const node = doc.byId[id];
  if (!node || !blockWritable(id)) return;
  pushUndo("split", [node.page]);
  const fmt = formatForBlock(id);
  // The caret offset is in editor-visible space (hidden props aren't shown), so
  // split the visible text and keep the hidden props on the original block.
  const { visible, hidden } = splitProps(node.raw, isBuiltinHidden, fmt);
  const before = visible.slice(0, offset);
  const after = visible.slice(offset);
  const pageName = node.page;
  // Ordered-list items propagate: a block split off an ordered item is itself
  // ordered (OG inherits `:logseq.order-list-type`), toggleable per-block later.
  const ordered = isOrdered(id);
  const withOrdered = (raw: string) => rawWithOrderListType(raw, "number", fmt);
  const orderedAfter = ordered ? withOrdered(after) : after;
  const orderedEmpty = ordered ? withOrdered("") : "";

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
          id: emptyId,
          raw: orderedEmpty,
          collapsed: false,
          parent: keepStartInScope ? id : node.parent,
          page: pageName,
          children: [],
        };
        if (keepStartInScope) {
          s.byId[id].children.unshift(emptyId);
        } else {
          const sibs = node.parent === null
            ? s.pages[s.pages.findIndex((p) => p.name === pageName)].roots
            : s.byId[node.parent].children;
          sibs.splice(sibs.indexOf(id), 0, emptyId);
        }
      })
    );
    startEditing(emptyId, 0, null, editingSurface);
    markDirty(pageName);
    return;
  }

  const newId = freshId();

  setDoc(
    produce((s) => {
      s.byId[id].raw = joinProps(before, hidden, fmt);
      const hasVisibleChildren = node.children.length > 0 && !node.collapsed;
      if (hasVisibleChildren || forceChild) {
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
  startEditing(newId, 0, null, editingSurface);
  markDirty(pageName);
}

/** Tab: make the block the last child of its previous sibling. */
export function indentBlock(id: string, caretOffset: number) {
  if (!blockWritable(id)) return;
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
      np.raw = rawWithCollapsed(np.raw, false, formatForBlock(newParent));
      np.collapsed = false;
    })
  );
  startEditing(id, caretOffset);
  markDirty(pageName);
}

/** Shift+Tab: move the block out to be the next sibling of its parent. */
export function outdentBlock(id: string, caretOffset: number) {
  const node = doc.byId[id];
  if (!node || !blockWritable(id) || node.parent === null) return;
  pushUndo("outdent", [node.page]);
  const parentId = node.parent;
  const grandParent = doc.byId[parentId].parent;
  const pageName = node.page;

  setDoc(
    produce((s) => {
      const parent = s.byId[parentId];
      const idx = parent.children.indexOf(id);
      // OG only reparents the following siblings for traditional outdenting;
      // logical outdenting stops after moving this block (`src/main/frontend/modules/outliner/core.cljs:835-852`
      // at `6e7afa8eb`). Keep this decision inside the shared store operation so
      // keyboard, mobile, and any future caller all use the same mode.
      if (logicalOutdenting()) {
        parent.children.splice(idx, 1);
      } else {
        const following = parent.children.splice(idx);
        following.shift(); // drop id
        for (const f of following) s.byId[f].parent = id;
        s.byId[id].children.push(...following);
      }
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
export function mergeWithPrev(
  id: string,
  scope: OutlineScope | null = null,
  editingSurface: string | null = null,
): boolean {
  if (!blockWritable(id)) return false;
  const prev = prevVisible(id, scope);
  if (prev === null) return false;
  const node = doc.byId[id];
  if (doc.byId[prev].page !== node.page) return false; // don't merge across pages
  pushUndo("merge", [node.page]);
  const fmt = formatForBlock(id); // prev is same page (checked above) → same format
  // Merge visible content only; keep the previous block's hidden props (it keeps
  // its identity) and drop the absorbed block's — otherwise the id::/collapsed::
  // lines would be concatenated mid-line and a block could end up with two ids.
  const prevSplit = splitProps(doc.byId[prev].raw, isBuiltinHidden, fmt);
  const curSplit = splitProps(node.raw, isBuiltinHidden, fmt);
  const curVisible = curSplit.visible;
  const joinOffset = prevSplit.visible.length;
  const pageName = node.page;

  // Preserve the absorbed block's id if the survivor has none — otherwise inbound
  // ((id)) references to the absorbed block would orphan on merge. Match the id
  // line in the block's on-disk syntax (md `id:: x` vs org drawer `:id: x`).
  let hidden = prevSplit.hidden;
  const idPresent = fmt === "org" ? /(?:^|\n):id:\s/i : /(?:^|\n)id:: /i;
  const idLine = fmt === "org" ? /(?:^|\n)(:id:\s*\S+)/i : /(?:^|\n)(id:: \S+)/i;
  const survivorHasId = idPresent.test(prevSplit.hidden);
  const absorbedId = idLine.exec(curSplit.hidden)?.[1];
  if (!survivorHasId && absorbedId) {
    hidden = hidden ? `${hidden}\n${absorbedId}` : absorbedId;
  }

  setDoc(
    produce((s) => {
      s.byId[prev].raw = joinProps(prevSplit.visible + curVisible, hidden, fmt);
      for (const c of node.children) s.byId[c].parent = prev;
      s.byId[prev].children.push(...node.children);
      const arr = node.parent === null
        ? s.pages[s.pages.findIndex((p) => p.name === pageName)].roots
        : s.byId[node.parent].children;
      arr.splice(arr.indexOf(id), 1);
      delete s.byId[id];
    })
  );
  startEditing(prev, joinOffset, null, editingSurface);
  markDirty(pageName);
  return true;
}

/** Insert a parsed outline (from a paste) as siblings right after `afterId`.
 *  Returns the last top-level inserted block id (to focus). */
export function insertOutlineAfter(afterId: string, nodes: OutlineNode[]): string {
  if (!nodes.length) return afterId;
  // Read-only gate at the choke point — file drops (and any future caller)
  // must not mutate a page the round-trip self-check marked read-only
  // (Phase-6 review finding, validated).
  if (!blockWritable(afterId)) return afterId;
  pushUndo("paste", [doc.byId[afterId].page]);
  const parent = doc.byId[afterId].parent;
  const pageName = doc.byId[afterId].page;
  const format = formatForPage(pageName);
  let lastId = afterId;
  setDoc(
    produce((s) => {
      const create = (n: OutlineNode, par: string | null): string => {
        const id = freshId();
        const childIds = n.children.map((c) => create(c, id));
        s.byId[id] = {
          id,
          raw: rawWithInheritedOrderListType(n.raw, format, afterId),
          collapsed: false,
          parent: par,
          page: pageName,
          children: childIds,
        };
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

/** Replace one empty leaf with a parsed outline in one store transaction and one
 * undo entry. Structured/multiline paste uses this instead of insert-then-delete,
 * which could leave a partial import after one Undo. */
export function replaceEmptyBlockWithOutline(id: string, nodes: OutlineNode[]): string {
  const current = doc.byId[id];
  if (!nodes.length || !current || current.children.length || !blockWritable(id)) return id;
  const format = formatForBlock(id);
  const split = splitProps(current.raw, isBuiltinHidden, format);
  if (split.visible.trim()) return id;
  pushUndo("paste-replace-empty", [current.page]);
  let lastId = id;
  setDoc(produce((state) => {
    const create = (outline: OutlineNode, parent: string | null, reuseId?: string): string => {
      const created = reuseId ?? freshId();
      const children = outline.children.map((child) => create(child, created));
      const sourceRaw = reuseId ? joinProps(outline.raw, split.hidden, format) : outline.raw;
      const raw = rawWithInheritedOrderListType(sourceRaw, format, id);
      state.byId[created] = { id: created, raw, collapsed: false, parent, page: current.page, children };
      return created;
    };
    // Reuse the host for the first imported root. Besides avoiding a ghost blank,
    // this preserves its hidden id/properties and therefore inbound references.
    const created = nodes.map((node, index) => create(node, current.parent, index === 0 ? id : undefined));
    const siblings = current.parent === null
      ? state.pages[state.pages.findIndex((page) => page.name === current.page)].roots
      : state.byId[current.parent].children;
    siblings.splice(siblings.indexOf(id), 1, ...created);
    lastId = created[created.length - 1];
  }));
  markDirty(current.page);
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
  if (!page || !pageWritable(name)) return false;
  if (page.roots.length) {
    // Append after the last top-level block (end of the page).
    insertOutlineAfter(page.roots[page.roots.length - 1], nodes);
  } else {
    // Empty (or brand-new) page: seed an empty anchor root, append after it, then
    // drop the anchor — reuses insertOutlineAfter's subtree creation rather than a
    // bespoke root builder. One undo unit: the anchor/insert/delete sequence used
    // to push three undo entries, so one undo left the anchor + row behind
    // (Phase-6 review finding, validated).
    withUndoUnit("capture", [name], () => {
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
    });
  }
  return await flushPage(name);
}

const PROP_LINE = /^([A-Za-z0-9_./-]+):: ?(.*)$/;

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
  return n ? (pageByName(n.page)?.readOnly ?? false) : false;
}

/** Store mutation boundary. UI affordances also hide on read-only pages, but
 * every write API must enforce this itself because menus/shortcuts/sheets can
 * call the store without entering the textarea. Guide pages are virtual and
 * equally non-writable. */
export function pageWritable(name: string): boolean {
  const page = pageByName(name);
  return !!page && !page.readOnly && !page.guide;
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
  pushUndo(`prop:${id}:${key}`, [node.page]);
  if (formatForBlock(id) === "org") {
    // ORG blocks carry properties in a `:PROPERTIES:` drawer — writing a
    // markdown `key:: value` line into org renders as visible body text and is
    // NOT read back as a property (same class as GH #25 for id::). Mirrors
    // rawWithBlockId's canonical placement: title, planning, drawer, body.
    setDoc("byId", id, "raw", orgRawWithProperty(node.raw, key, value));
    markDirty(node.page);
    return;
  }
  const lines = node.raw.split("\n");
  const first = lines[0] ?? "";
  // Canonical block shape: first line, planning lines, property lines, body.
  // Scan ONLY the canonical head region (stop at the first body line) plus the
  // legacy trailing property block (the old writer appended at the end), so a
  // `key::`-looking line inside body text or a code fence is never touched or
  // reordered — this regex is fence-unaware and must not reach into the body.
  const PLANNING_LINE = /^\s*(SCHEDULED|DEADLINE):\s*</;
  let i = 1;
  while (i < lines.length && PLANNING_LINE.test(lines[i])) i++;
  const planningEnd = i;
  while (i < lines.length && PROP_LINE.test(lines[i])) i++;
  const propsEnd = i;
  let j = lines.length;
  while (j > propsEnd && PROP_LINE.test(lines[j - 1] ?? "")) j--;
  const notKey = (l: string) => PROP_LINE.exec(l)?.[1] !== key;
  // Update an existing key IN PLACE so its line position (and therefore the
  // field-table column order it drives) is preserved; only a genuinely new key
  // appends after the others. Re-appending on every edit moved the touched
  // column to the end (GH #216) and churned the property order on disk.
  const props = lines.slice(planningEnd, propsEnd);
  const at = props.findIndex((l) => PROP_LINE.exec(l)?.[1] === key);
  if (value !== null) {
    const line = `${key}:: ${value}`;
    if (at >= 0) props[at] = line;
    else props.push(line);
  } else if (at >= 0) {
    props.splice(at, 1);
  }
  const out = [
    first,
    ...lines.slice(1, planningEnd), // planning stays before properties (OG order)
    ...props,
    ...lines.slice(propsEnd, j), // body untouched
    ...lines.slice(j).filter(notKey), // legacy trailing props: only the key is removed
  ];
  setDoc("byId", id, "raw", out.join("\n"));
  markDirty(node.page);
}

/** Read a page-level property from the page's pre-block (the leading
 *  `key:: value` lines), or null. */
export function readPageProperty(pageName: string, key: string): string | null {
  const p = doc.pages.find((x) => x.name === pageName);
  if (!p) return null;
  const fromPreBlock = readPropertyValue(p.preBlock, key);
  if (fromPreBlock !== null) return fromPreBlock;
  const first = p.format === "md" ? doc.byId[p.roots[0]] : null;
  return first && isPropertiesOnly(first.raw) ? readPropertyValue(first.raw, key) : null;
}

/** Set or clear a page-level property in the page's canonical property source:
 *  pre-block normally, or OG's properties-only first bullet. Persists through
 *  the normal dirty/save path and is undo-safe. */
export function setPageProperty(pageName: string, key: string, value: string | null) {
  const idx = doc.pages.findIndex((x) => x.name === pageName);
  if (idx < 0 || !pageWritable(pageName)) return;
  pushUndo(`pageprop:${pageName}:${key}`, [pageName]);
  const page = doc.pages[idx];
  const first = page.format === "md" ? doc.byId[page.roots[0]] : null;
  // A properties-only first root is the same editable source as the rendered
  // header. Do not silently duplicate its property into preBlock; pageToDto or
  // the native new-header boundary canonicalizes its persisted form.
  if (first && (first.originatedFromPageHeader || (!page.preBlock && isPropertiesOnly(first.raw)))) {
    const next = upsertPropertyLine(first.raw, key, value) ?? "";
    if (first.originatedFromPageHeader && next === "") {
      setDoc(produce((s) => {
        const target = s.pages.find((p) => p.name === pageName);
        if (target?.roots[0] === first.id) target.roots.shift();
        delete s.byId[first.id];
      }));
    } else {
      setDoc("byId", first.id, "raw", next);
    }
    markDirty(pageName);
    return;
  }
  setDoc("pages", idx, "preBlock", upsertPropertyLine(doc.pages[idx].preBlock, key, value));
  markDirty(pageName);
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
  const node = doc.byId[id];
  if (!node?.originatedFromPageHeader || node.raw !== "" || node.children.length > 0) return;
  setDoc(
    produce((s) => {
      const page = s.pages.find((p) => p.name === node.page);
      if (page?.roots[0] === id) page.roots.shift();
      delete s.byId[id];
    })
  );
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
  markDirty(pageName);
  return id;
}

/** Toggle a property: set it to `value`, or remove it if already that value. */
export function toggleBlockProperty(id: string, key: string, value: string) {
  setBlockProperty(id, key, blockProperty(id, key) === value ? null : value);
}

const ORDER_KEY = "logseq.order-list-type";
function isOrdered(id: string | null | undefined): boolean {
  return !!id && blockProperty(id, ORDER_KEY) === "number";
}

function orderListTypeFromRaw(raw: string, format: Format): string | null {
  for (const [key, value] of facetsOf(raw, format).properties) {
    if (key.toLowerCase() === ORDER_KEY) return value.trim();
  }
  return null;
}

/** The one format-aware raw transform for the block-level list property.
 * `splitProps`/`joinProps` are the audited metadata path: they preserve visible
 * body bytes, ignore property lookalikes inside fences, and emit Org drawers.
 * OG writes both the in-memory property and serialized content at
 * `src/main/frontend/modules/outliner/core.cljs:420-433` (6e7afa8eb). */
function rawWithOrderListType(raw: string, value: string | null, format: Format): string {
  const { visible } = splitProps(raw, (key) => key.toLowerCase() === ORDER_KEY, format);
  if (value === null) return visible;
  const property = format === "org" ? `:${ORDER_KEY}: ${value}` : `${ORDER_KEY}:: ${value}`;
  return joinProps(visible, property, format);
}

/** Preserve a source's explicit list type; otherwise inherit the target's.
 * This is OG's common move/insert rule (`outliner/core.cljs:420-433,536-555`
 * at 6e7afa8eb), shared by drag and every structural outline insertion below. */
function rawWithInheritedOrderListType(raw: string, format: Format, targetId: string | null | undefined): string {
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
  markDirty(node.page);
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
  toggleListItemAtIndex(id, node.raw.split("\n").indexOf(rawLine));
}

/** Flip the `[ ]`/`[x]` checkbox on a SPECIFIC raw line index. Targeting by index
 *  (not line text) is what makes the AST list checkbox toggle safe when two items
 *  share the same label — see toggleAstCheckbox in render/body.tsx. */
export function toggleListItemAtIndex(id: string, lineIndex: number) {
  const node = doc.byId[id];
  if (!node || !blockWritable(id)) return;
  const lines = node.raw.split("\n");
  const ln = lines[lineIndex];
  if (ln === undefined || !/\[[ xX]\]/.test(ln)) return;
  const next = /\[ \]/.test(ln) ? ln.replace(/\[ \]/, "[x]") : ln.replace(/\[[xX]\]/, "[ ]");
  if (next === ln) return;
  pushUndo(`listcheck:${id}`, [node.page]);
  lines[lineIndex] = next;
  setDoc("byId", id, "raw", lines.join("\n"));
  markDirty(node.page);
}

/** Set the block's heading level via the markdown `#` prefix (null clears it). */
export function setHeading(id: string, level: number | null) {
  const node = doc.byId[id];
  if (!node || !blockWritable(id)) return;
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
  const tag = which === "scheduled" ? "SCHEDULED" : "DEADLINE";
  // Capture the optional time (`HH:mm`) and org repeater cookie (`+1w`, `.+1w`,
  // `++1w`) — both after the weekday, in OG's fixed order `<date wday time repeater>`
  // — so re-opening the picker pre-fills the existing time AND recurrence. The
  // weekday is `[A-Za-z]+` (mldoc consumes any letters; OG writes English 3-letter).
  const m = new RegExp(
    `^${tag}:\\s*<(\\d{4})-(\\d{2})-(\\d{2})(?:\\s+[A-Za-z]+)?(?:\\s+(\\d{1,2}:\\d{2}))?(?:\\s+((?:\\.\\+|\\+\\+|\\+)\\d+[dwmy]))?`,
    "m"
  ).exec(node.raw);
  return m
    ? { y: +m[1], m: +m[2] - 1, d: +m[3], time: m[4] ? normalizeHHmm(m[4]) : null, repeater: m[5] ?? null }
    : null;
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
  pushUndo(`sched:${id}:${which}`, [node.page]);
  const tag = which === "scheduled" ? "SCHEDULED" : "DEADLINE";
  // Remove the old planning line ONLY from the canonical head region (the run of
  // planning/property lines right after the first line) — a `SCHEDULED:` inside a
  // code fence or body text is content and must never be touched (review finding:
  // the old any-line filter deleted fenced planning-lookalikes).
  const all = node.raw.split("\n");
  const isHeadLine = (l: string) => /^\s*(SCHEDULED|DEADLINE):/.test(l) || PROP_LINE.test(l);
  let headEnd = 1;
  while (headEnd < all.length && isHeadLine(all[headEnd])) headEnd++;
  const lines = [
    ...all.slice(0, headEnd).filter((l, i) => i === 0 || !new RegExp(`^${tag}:`).test(l.trim())),
    ...all.slice(headEnd),
  ];
  if (date) {
    const wd = WEEKDAYS[new Date(date.y, date.m, date.d).getDay()];
    const hhmm = time ? normalizeHHmm(time) : null;
    const timePart = hhmm ? ` ${hhmm}` : "";
    const rep = repeater ? ` ${repeater}` : "";
    const stamp = `${tag}: <${date.y}-${pad2(date.m + 1)}-${pad2(date.d)} ${wd}${timePart}${rep}>`;
    lines.splice(Math.min(1, lines.length), 0, stamp);
  }
  setDoc("byId", id, "raw", lines.join("\n"));
  markDirty(node.page);
}

/** A block's raw with `collapsed:: true` added or removed so the persisted
 *  property matches the collapsed state. OG stores collapse in the file as a
 *  block property, so mirroring it here makes a collapse survive a relaunch and
 *  show up collapsed in OG / the mobile app. Fence-aware via splitProps. */
function rawWithCollapsed(raw: string, collapsed: boolean, format: Format): string {
  if (format === "org") return orgRawWithProperty(raw, "collapsed", collapsed ? "true" : null);
  const { visible, hidden } = splitProps(raw, isBuiltinHidden, format);
  const nextHidden = upsertPropertyLine(hidden, "collapsed", collapsed ? "true" : null) ?? "";
  return joinProps(visible, nextHidden, format);
}

/** Set a block's collapsed state AND mirror it into its raw `collapsed::` so it
 *  persists — the on-disk markdown is the source of truth on the next load. */
function writeCollapsed(id: string, collapsed: boolean) {
  const n = doc.byId[id];
  if (!n || !blockWritable(id)) return;
  const nextRaw = rawWithCollapsed(n.raw, collapsed, formatForBlock(id));
  setDoc("byId", id, "collapsed", collapsed);
  if (nextRaw !== n.raw) setDoc("byId", id, "raw", nextRaw);
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
  markDirty(doc.byId[id].page);
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
  markDirty(root.page);
}

/** The block's existing durable `id` — a markdown `id:: <uuid>` trailer or an
 *  org `:PROPERTIES:` drawer `:id: <uuid>` line — case-insensitively, or null.
 *  Format-aware because in ORG `id:: x` is plain body text, NOT a property (lsdoc
 *  reads the drawer, not a `key::` line); so an org block's real id lives in its
 *  `:PROPERTIES:` drawer and must be matched there (GH #25). */
export function existingBlockId(raw: string, format: Format): string | null {
  const re = format === "org" ? /(?:^|\n):id:\s*(\S+)/i : /(?:^|\n)id:: *(\S+)/i;
  const m = re.exec(raw);
  return m ? m[1] : null;
}

/** `raw` with a durable `id` property added in the page's on-disk format.
 *  Markdown appends an `id:: <uuid>` trailer. ORG inserts/extends a
 *  `:PROPERTIES:`/`:id:`/`:END:` drawer at OG's canonical position — right after
 *  the title line and any SCHEDULED/DEADLINE planning lines (mirroring OG's
 *  `insert-property`, util/property.cljs). Writing markdown `id::` into an org
 *  file would BOTH render as visible body text and not be read back as the
 *  block's id (GH #25) — org MUST use the drawer. The caller guarantees the
 *  block has no id yet (see {@link existingBlockId}). */
export function rawWithBlockId(raw: string, uuid: string, format: Format): string {
  if (format !== "org") return `${raw}\nid:: ${uuid}`;
  const lines = raw.split("\n");
  const start = lines.findIndex((l) => l.trim().toUpperCase() === ":PROPERTIES:");
  const end =
    start >= 0 ? lines.findIndex((l, i) => i > start && l.trim().toUpperCase() === ":END:") : -1;
  if (start >= 0 && end > start) {
    // Extend the existing drawer: insert the id line just before :END:.
    lines.splice(end, 0, `:id: ${uuid}`);
    return lines.join("\n");
  }
  // No drawer: title, SCHEDULED*, DEADLINE*, :PROPERTIES: drawer, rest-of-body —
  // OG groups planning lines above the drawer (util/property.cljs insert-property).
  const [title, ...rest] = lines;
  const isSched = (l: string) => l.startsWith("SCHEDULED");
  const isDead = (l: string) => l.startsWith("DEADLINE");
  const scheduled = rest.filter(isSched);
  const deadline = rest.filter(isDead);
  const body = rest.filter((l) => !isSched(l) && !isDead(l));
  return [title, ...scheduled, ...deadline, ":PROPERTIES:", `:id: ${uuid}`, ":END:", ...body].join(
    "\n"
  );
}

/** `raw` with an org drawer property set/updated/removed. Operates ONLY on the
 *  first `:PROPERTIES:` drawer in the canonical head region (title, planning,
 *  drawer, body — the same placement rawWithBlockId uses); body text and code
 *  blocks are never scanned. Removing the last property removes the drawer. */
function orgRawWithProperty(raw: string, key: string, value: string | null): string {
  const lines = raw.split("\n");
  const start = lines.findIndex((l) => l.trim().toUpperCase() === ":PROPERTIES:");
  const end =
    start >= 0 ? lines.findIndex((l, i) => i > start && l.trim().toUpperCase() === ":END:") : -1;
  const keyRe = new RegExp(`^:${key.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}:\\s*`, "i");
  if (start >= 0 && end > start) {
    // Update in place so an existing drawer key keeps its position (GH #216);
    // only a new key appends.
    const inner = lines.slice(start + 1, end);
    const at = inner.findIndex((l) => keyRe.test(l.trim()));
    if (value !== null) {
      const line = `:${key}: ${value}`;
      if (at >= 0) inner[at] = line;
      else inner.push(line);
    } else if (at >= 0) {
      inner.splice(at, 1);
    }
    if (inner.length === 0) {
      // Drawer emptied: drop it entirely.
      return [...lines.slice(0, start), ...lines.slice(end + 1)].join("\n");
    }
    return [...lines.slice(0, start + 1), ...inner, ...lines.slice(end)].join("\n");
  }
  if (value === null) return raw; // nothing to remove
  // No drawer yet: title, SCHEDULED*, DEADLINE*, drawer, rest (rawWithBlockId's rule).
  const [title, ...rest] = lines;
  const isPlan = (l: string) => l.startsWith("SCHEDULED") || l.startsWith("DEADLINE");
  let planEnd = 0;
  while (planEnd < rest.length && isPlan(rest[planEnd])) planEnd++;
  return [
    title,
    ...rest.slice(0, planEnd),
    ":PROPERTIES:",
    `:${key}: ${value}`,
    ":END:",
    ...rest.slice(planEnd),
  ].join("\n");
}

/** Ensure a block has a persistent id (assigned lazily, like OG) AND that it's
 *  durably on disk, returning the uuid — or null if it couldn't be saved
 *  (conflict/error). Used to make `((uuid))` references: the caller must not put
 *  a ref on the clipboard until the id is actually written, or quitting /
 *  resolving a conflict with "use disk version" would leave the ref dangling. */
export async function ensureBlockId(id: string): Promise<string | null> {
  const node = doc.byId[id];
  if (!node || !blockWritable(id)) return null;
  const fmt = formatForBlock(id);
  // Any existing id is the block's durable id — match its value (not just a UUID
  // shape), case-INSENSITIVELY (Rust's property("id") is case-insensitive, so an
  // `ID::` / `:ID:` from another editor counts), so we never write a SECOND id
  // that Rust then ignores → dangling copied ref.
  const existing = existingBlockId(node.raw, fmt);
  const uuid = existing ?? crypto.randomUUID();
  if (!existing) {
    setDoc("byId", id, "raw", rawWithBlockId(node.raw, uuid, fmt));
    markDirty(node.page);
  }
  // Even a pre-existing id may not be on disk yet (added in-memory, not flushed);
  // flush and only hand back the uuid if the write actually landed.
  const ok = await flushPage(node.page);
  return ok ? uuid : null;
}

/** A live reference to a loaded block — its stable uuid + the page it lives on
 *  (so a satellite surface can load that page and render the same editable
 *  node). The uuid IS the store key, so no snapshot is needed. */
export function blockRef(id: string): { uuid: string; page: string; pageKind: PageKind; path?: string } {
  const n = doc.byId[id];
  const owner = pageByName(n.page);
  return {
    uuid: n.id,
    page: n.page,
    pageKind: owner?.kind ?? "page",
    ...(owner?.path ? { path: owner.path } : {}),
  };
}

const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

/** Persist a block's `id::` equal to its current uuid, so its in-memory key and
 *  on-disk identity match — making a reference to it survive an app restart.
 *  No-op if it already has an id::. Only writes when the uuid is a real UUID
 *  (always true for blocks loaded from the graph); a freshly-created,
 *  not-yet-reloaded block is skipped rather than writing a non-UUID id::. */
export function ensureStableBlockId(id: string): void {
  const node = doc.byId[id];
  if (!node || !blockWritable(id)) return;
  const fmt = formatForBlock(id);
  if (existingBlockId(node.raw, fmt)) return;
  if (!UUID_RE.test(id)) return;
  setDoc("byId", id, "raw", rawWithBlockId(node.raw, id, fmt));
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
export function persistentBlockRef(id: string): { uuid: string; page: string; pageKind: PageKind; path?: string } {
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
  kind: PageKind,
  path?: string,
): Promise<void> {
  const loadedOwner = pageByName(page);
  const exactOwnerLoaded = !!loadedOwner && (!path || loadedOwner.path === path);
  const loadedNode = doc.byId[uuid];
  if (!loadedNode || !exactOwnerLoaded || loadedNode.page !== page) {
    const dto = path
      ? await backend().getPageByPath(path)
      : await backend().getPage(page, kind);
    if (dto) ensurePageLoaded(dto);
  }
  // Re-check: a concurrent navigation may have loaded the page meanwhile, or the
  // cache may have been rebuilt (external change) and reassigned the block a new
  // uuid — in which case there's nothing safe to stamp.
  const owner = pageByName(page);
  const node = doc.byId[uuid];
  if (node?.page === page && owner && (!path || owner.path === path)) ensureStableBlockId(uuid);
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
  const format = formatForBlock(id);
  const strip = stripId || stripCollapsed;
  const raw = strip
    ? splitProps(
        n.raw,
        (k) => (stripId && k === "id") || (stripCollapsed && k === "collapsed"),
        format,
      ).visible
    : n.raw;
  const lines = raw.split("\n");
  const out: string[] = [];
  // OG's clipboard path intentionally exports blocks as Markdown even when the
  // source page is Org (`export-blocks-as-markdown`), but removes IDs using the
  // SOURCE format. Keep that portable outline shape while stripping Org drawers.
  const tabs = "\t".repeat(level);
  out.push(`${tabs}- ${lines[0] ?? ""}`.replace(/\s+$/, ""));
  for (const line of lines.slice(1)) out.push(line === "" ? "" : `${tabs}  ${line}`);
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
    return {
      raw: n.raw,
      format: pageByName(n.page)?.format ?? "md",
      children: n.children.map(toNode).filter((x): x is ExportNode => x != null),
    };
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
  const format = pageByName(pageName)?.format ?? "md";
  const removedSidebarIds = new Set<string>();
  const collectRemovedIds = (bid: string) => {
    const current = doc.byId[bid];
    if (!current) return;
    removedSidebarIds.add(current.id);
    const durable = existingBlockId(current.raw, format);
    if (durable) removedSidebarIds.add(durable);
    current.children.forEach(collectRemovedIds);
  };
  collectRemovedIds(id);
  setDoc(
    produce((s) => {
      const arr =
        node.parent === null
          ? s.pages[s.pages.findIndex((p) => p.name === pageName)].roots
          : s.byId[node.parent!].children;
      const ix = arr.indexOf(id);
      if (ix >= 0) arr.splice(ix, 1);
      const rm = (bid: string) => {
        for (const c of s.byId[bid].children) rm(c);
        delete s.byId[bid];
      };
      rm(id);
    })
  );
  removeDeletedBlocksFromSidebar(removedSidebarIds);
  if (editingId() === id) endEdit("delete-block");
  markDirty(pageName);
}

export function deleteBlock(id: string) {
  if (!blockWritable(id)) return;
  pushUndo("delete", [doc.byId[id].page]);
  deleteBlockInternal(id);
}

/** Re-seed the phantom empty bullet on a page emptied of its last block. Explicit
 *  "Delete block" / selection-delete bypass the Backspace last-block guard, so a page
 *  CAN reach zero roots — and then has nothing to type into. Mirrors {@link emptyPage}
 *  exactly: an editable blank root that is deliberately NOT marked dirty, so — like a
 *  brand-new day — it shows a bullet to write in but only persists to disk once the
 *  user actually types (the edit path marks it dirty then). Returns the new id, or
 *  null if the page is missing, read-only, or already non-empty. */
export function ensureEmptyBlock(pageName: string, opts: { afterProperties?: boolean } = {}): string | null {
  const page = pageByName(pageName);
  if (!page || page.readOnly) return null;
  const onlyPropertyRoot =
    opts.afterProperties === true &&
    page.format === "md" &&
    page.roots.length === 1 &&
    isPropertiesOnly(doc.byId[page.roots[0]]?.raw ?? "");
  if (page.roots.length && !onlyPropertyRoot) return null;
  const id = freshId();
  setDoc(
    produce((s) => {
      s.byId[id] = { id, raw: "", collapsed: false, parent: null, page: pageName, children: [] };
      s.pages[s.pages.findIndex((p) => p.name === pageName)].roots.push(id);
    })
  );
  return id;
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
  const order = selectionOrder(a);
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
export function selectBlock(id: string, scope: OutlineScope | null = null) {
  endEdit("select-block");
  notifyOutlineSelectionStarted(id);
  activeSelectionScope = scope;
  setSelAnchor(id);
  setSelFocus(id);
}
export function clearSelection() {
  setSelAnchor(null);
  setSelFocus(null);
  activeSelectionScope = null;
}
/** Extend the current block selection's focus to `id` (mouse-drag / shift-click).
 *  Starts a fresh selection anchored at `id` if none is active. */
export function extendSelectionTo(id: string, scope: OutlineScope | null = activeSelectionScope) {
  notifyOutlineSelectionStarted(id);
  if (selAnchor() === null) {
    activeSelectionScope = scope;
    setSelAnchor(id);
  }
  if (activeSelectionScope && !scopedVisibleOrder(activeSelectionScope).includes(id)) return;
  setSelFocus(id);
}
export function hasSelection(): boolean {
  return selAnchor() !== null;
}
export function moveSelection(dir: 1 | -1, extend: boolean) {
  const f = selFocus();
  if (!f) return;
  const order = selectionOrder(f);
  const i = order.indexOf(f);
  const ni = i + dir;
  if (ni < 0 || ni >= order.length) return;
  const next = order[ni];
  setSelFocus(next);
  if (!extend) setSelAnchor(next);
  scrollBlockRowIntoView(next);
}

/** Cycle every non-empty block in the active selection as one document
 * transaction. Each block advances from its own current marker, so a mixed
 * selection stays mixed (plain -> open, open -> active, active -> done). The
 * operation is all-or-nothing across read-only pages and preserves the visual
 * selection for repeated cycling. */
export function cycleSelectionTasks(): boolean {
  const ids = selectedIds().filter((id) => !!doc.byId[id]?.raw.trim());
  if (!ids.length || ids.some((id) => !blockWritable(id))) return false;

  const pages = [...new Set(ids.map((id) => doc.byId[id].page))];
  pushUndo("cycle-task-sel", pages);
  setDoc(
    produce((state) => {
      for (const id of ids) {
        const node = state.byId[id];
        if (!node) continue;
        // Match the existing editor command exactly: marker cycling handles
        // repeaters, while checkbox/marker-chip transitions own time tracking.
        node.raw = cycleMarkerSmart(node.raw, workflow()).raw;
      }
    })
  );
  for (const page of pages) markDirty(page);
  return true;
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

function selectionRemovalSurvivor(): string | null {
  const selected = selectedIds();
  const first = selected[0];
  const last = selected.at(-1);
  if (!first || !last) return null;
  return nextVisible(last) ?? prevVisible(first) ?? doc.byId[first]?.parent ?? null;
}

function reselectSurvivingBlock(id: string | null) {
  if (id && doc.byId[id]) selectBlock(id);
  else clearSelection();
}

export function indentSelection() {
  const ids = topSelected();
  if (!ids.length || ids.some((id) => !blockWritable(id))) return;
  const first = ids[0];
  const sibs = rootsOf(first);
  const fi = sibs.indexOf(first);
  if (fi <= 0) return;
  const newParent = sibs[fi - 1];
  if (activeSelectionScope && !scopedVisibleOrder(activeSelectionScope).includes(newParent)) return;
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
  if (!ids.length || ids.some((id) => !blockWritable(id))) return;
  const parentId = doc.byId[ids[0]].parent;
  if (parentId === null) return;
  if (activeSelectionScope?.forceExpandedRoot === parentId) return;
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
  const survivor = selectionRemovalSurvivor();
  const ids = topSelected();
  if (!ids.length || ids.some((id) => !blockWritable(id))) return;
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
  if (ed && !doc.byId[ed]) endEdit("delete-selection");
  for (const p of pages) markDirty(p);
  reselectSurvivingBlock(survivor);
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
  if (!node || !blockWritable(id) || (newParent !== null && !blockWritable(newParent))) return;
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
  targetPage?: string,
  dropTargetId?: string,
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
  if (!pageWritable(oldPage) || !pageWritable(newPage)) return;
  // Cross-page drag: flush the source while it still holds the block, so a
  // pre-existing pending save can't write the removal before the destination
  // lands. Abort (no move) if the source can't be saved.
  if (newPage !== oldPage && !(await prepareCrossPageSources([oldPage]))) {
    pushToast(`Couldn't move — “${oldPage}” has unsaved changes that need resolving first.`, "error");
    return;
  }
  if (!doc.byId[id]) return; // block vanished during the async flush
  const sourceFormat = formatForBlock(id);
  const destinationFormat = formatForPage(newPage);
  const inheritanceTarget = dropTargetId ?? newParent;
  // A cross-format move already preserves the source raw verbatim; only a newly
  // inherited property is emitted in the destination page's syntax.
  const movedRaw = orderListTypeFromRaw(doc.byId[id].raw, sourceFormat) !== null
    ? doc.byId[id].raw
    : rawWithInheritedOrderListType(doc.byId[id].raw, destinationFormat, inheritanceTarget);
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
      s.byId[id].raw = movedRaw;
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
// A reorder only keeps the editor transiently blurred for one animation frame.
// Keep its page ownership: watcher/feed refreshes for another page must not be
// held hostage by a sidebar or split-pane reorder.
let blockMovingPage: string | null = null;
// Feed refresh ownership observes the end of a page-scoped drag.  Keep the
// inexpensive page check above, but make its lifecycle observable so a deferred
// restart is released by the move itself rather than a coincidental later event.
const [blockMoveRev, setBlockMoveRev] = createSignal(0);
export function isBlockMoving(page?: string): boolean {
  blockMoveRev();
  return blockMovingPage !== null && (page === undefined || blockMovingPage === page);
}
export function setBlockMoving(v: boolean, page?: string): void {
  blockMovingPage = v ? (page ?? blockMovingPage ?? "") : null;
  setBlockMoveRev((n) => n + 1);
}

export function moveItem(id: string, dir: 1 | -1) {
  const node = doc.byId[id];
  if (!node || !blockWritable(id)) return;
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
export function setFeedExtender(fn: (() => Promise<boolean>) | null): void {
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
  // Hold the sources' saves until `dest` is durable (audit C#1), so a concurrent edit to
  // a source during the dest-write window can't write its post-removal state before the
  // block exists in the dest. On dest success, doSave → releaseSourcesFor frees +
  // reschedules the sources; on dest conflict/failure they stay held (the block is kept
  // on disk in the source) until the dest conflict is resolved and it saves durably.
  holdSourcesForDest(dest, sources);
  markDirty(dest);
  void flushPage(dest);
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

/** Like `nextVisible`, but when we're at the last LOADED block of the journal feed
 *  it pulls in the next day first (via the feed extender) and returns that day's
 *  first block. This lets Down-arrow keep going past the loaded window — previously
 *  only mouse-wheel scrolling (the LoadMore sentinel) grew the feed, so keyboard nav
 *  dead-ended at the last loaded bullet. Resolves to null when there's genuinely
 *  nothing below (a non-feed page, or the feed is exhausted). */
export async function nextVisibleOrExtend(id: string): Promise<string | null> {
  const direct = nextVisible(id);
  if (direct) return direct;
  const node = doc.byId[id];
  if (!node || doc.feed.indexOf(node.page) < 0) return null; // not a feed day → nothing to load
  if (!feedExtender || !(await feedExtender())) return null; // feed exhausted / no extender
  return nextVisible(id); // the newly-appended day's first block is now loaded
}

/** Pull in the next journal-feed day if there is one; resolves to whether the feed
 *  actually grew. Used by scroll-restore to reach a saved offset that lives in
 *  not-yet-loaded days (the feed otherwise only grows on a mouse-wheel sentinel
 *  hit). No-op (false) on a non-feed page or when the feed is exhausted. */
export async function extendFeedForScroll(): Promise<boolean> {
  return feedExtender ? feedExtender() : false;
}

/** Move a single block one slot, crossing into the adjacent day at a page
 *  boundary. Returns how it moved so the caller can restore the caret. */
export async function moveBlockFeed(id: string, dir: 1 | -1): Promise<"within" | "crossed" | "none"> {
  const node = doc.byId[id];
  if (!node || !blockWritable(id)) return "none";
  if (canMoveItem(id, dir)) {
    moveItem(id, dir);
    return "within";
  }
  if (node.parent !== null) return "none"; // nested block at a child-list edge: stop
  const target = await feedNeighbor(node.page, dir);
  if (!target || !pageWritable(target)) return "none";
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
  if (!ids.length || ids.some((id) => !blockWritable(id))) return;
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
  if (!target || !pageWritable(target)) return;
  if (!(await prepareCrossPageSources([page]))) return; // source has unsaved edits → abort
  pushUndo("move-sel-cross", [page, target]);
  crossMoveBlocks(ids, page, target, dir);
}

// ---------------------------------------------------------------------------
// Carry unfinished tasks forward (B)
// ---------------------------------------------------------------------------

function isOpenTask(id: string): boolean {
  // Leading task marker via the one markers.ts recognizer (vocabulary == lsdoc's, so
  // no disagreement) — parser-free, so carry works without the wasm renderer up.
  const m = MARKER_RE.exec((doc.byId[id]?.raw ?? "").trimStart())?.[1];
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
  if (!pageWritable(today) || fromPages.some((page) => pageByName(page) && !pageWritable(page))) return 0;
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
  if (!n || !blockWritable(id) || n.children.length === 0) return;
  pushUndo("collapse", [n.page]);
  writeCollapsed(id, !n.collapsed);
  markDirty(n.page);
}

/** Explicitly collapse or expand a block (no-op if it has no children or is
 *  already in the requested state). */
export function setCollapsed(id: string, collapsed: boolean) {
  const n = doc.byId[id];
  if (!n || !blockWritable(id) || n.children.length === 0 || n.collapsed === collapsed) return;
  pushUndo("collapse", [n.page]);
  writeCollapsed(id, collapsed);
  markDirty(n.page);
}
