import { type PageDto, type BlockDto, type PageKind } from "../types";
import { untombstone, setBaseRev, baseRevFor, activatePageInstance, forgetSaveState, rekeyPageSaveState, clearConflict, retirePageInstance, pageInstanceGeneration, isDirty, isSaving, isConflicted, conflictReason, flushPage, tombstone, dirtyPages, conflicts, resetSaveState, pageInstanceGenerations, deletePageOnDisk, group, groupedPages, savingPages, releaseGroup, reserveGroupMemberDeletion } from "./save/engine";
import { clearCollapseEpochs, doc, setDoc, FeedPage, pageByName } from "./model";
import { produce } from "solid-js/store";
import { purgePageNodes, toFeedPage, emptyPage } from "./convert";
import { invalidateAllMatrixDimensions, clearMatrixDimensionCache } from "../sheet/matrix";
import { invalidateUndoForPage, clearUndoHistory } from "./history";
import { captureBinding, stillBound, invalidateBinding } from "../binding";
import { graphOwner, readOwned } from "../owned";
import { backend } from "../backend";
import { removeDeletedPageFromNavigation, rightSidebar } from "../ui";
import { bumpDataRev, bumpPageInventoryRev } from "../graphSession";
import { type Route } from "../routeTypes";
import { type PageTarget } from "../routeTypes";
import { editingId, endEdit } from "../editorController";
import { clearSeededFacets } from "../render/facets";
import { notifyModeReset } from "../modeHooks";
import { deferExternalReload, replayDeferredExternalReloads } from "./deferredReload";
import { isBlockMoving } from "./edits/moves";
import { journalTitle, appNow } from "../journal";
import { graphRewriteFrozen } from "./graphRewriteState";
import { pushToast } from "../toasts";
import { resetReferenceSectionState } from "../referenceSectionState";

let publishIdentityNavigation: ((from: PageTarget, to: PageTarget) => void) | null = null;
/** The UI installs the exact-path route, tab, Recent and sidebar rewrite. */
export function installPageIdentityNavigation(handler: (from: PageTarget, to: PageTarget) => void): void {
  publishIdentityNavigation = handler;
}

/** Adopt an effective-name change for the same physical page. Navigation is
 * rekeyed first, then the working set and its save baseline change together.
 * External changes require a safe reload disposition; an acknowledged own
 * save may retain newer unsaved editor text under the new name. */
export function rekeyPageIdentityByPath(id: string, newName: string, rev: string | null, ownSaved = false): boolean {
  const old = doc.pages.find((page) => page.id === id && page.name !== newName);
  if (!old) return false;
  const oldName = old.name;
  if (doc.pages.some((page) => page.name === newName && page.id !== id)) return false;
  if (group(old.name) || isConflicted(old.name) || (!ownSaved && reloadDisposition(old.name) !== "reload")) return false;
  const from: PageTarget = { name: oldName, pageKind: old.kind, path: id };
  const to: PageTarget = { name: newName, pageKind: old.kind, path: id };
  if (!publishIdentityNavigation) return false;
  publishIdentityNavigation(from, to);
  rekeyPageSaveState(oldName, newName, rev);
  setDoc(produce((state) => {
    const page = state.pages.find((candidate) => candidate.id === id && candidate.name === oldName);
    if (page) { page.name = newName; page.title = newName; }
    state.feed = state.feed.map((name) => name === oldName ? newName : name);
    for (const node of Object.values(state.byId)) if (node.page === oldName) node.page = newName;
  }));
  invalidateUndoForPage(oldName);
  return true;
}

function upsertPage(dto: PageDto & { id?: string }) {
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
  activatePageInstance(dto.name);
  invalidateAllMatrixDimensions();
  if (replacing) invalidateUndoForPage(dto.name);
}

/** Whether a reload DTO carries the SAME content (page-property pre-block + every
 *  block's raw + tree shape, ignoring block ids) as the page already in memory —
 *  i.e. a self-write echo, not a real external change. Lets `upsertPage` skip a
 *  needless reload that would otherwise reset block identities and invalidate the
 *  undo history for content we already hold. */
function pageContentMatches(dto: PageDto & { id?: string }, page: FeedPage): boolean {
  if ((dto.id ?? "") !== (page.id ?? "")) return false;
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
export function ensurePageLoaded(dto: PageDto & { id?: string }) {
  const existing = doc.pages.find((p) => p.name === dto.name);
  if (existing && (existing.id ?? "") === (dto.id ?? "")) {
    // Same-content hydration still carries new disk authority (master
    // ba80a151e9a2): after a Concord resolution the winner can be open with an
    // older baseline, and the next ordinary edit would look like a new
    // conflict. Only a clean page whose content equals the DTO adopts its rev.
    if (dto.rev !== undefined && reloadDisposition(dto.name) === "reload" && pageContentMatches(dto, existing)) setBaseRev(dto.name, dto.rev);
    return;
  }
  if (existing && reloadDisposition(dto.name) !== "reload") return;
  // A path-pinned route may intentionally load a duplicate-day stray with the
  // same logical title as the canonical journal. Replace a safe name slot with
  // the exact requested file, but never discard unsaved edits or an active editor
  // while its backend read was in flight. Full simultaneous duplicate identity
  // is tracked by the file-identity ADR.
  upsertPage(dto);
  evictIfNeeded();
}

/** Admit a routed DTO when safe, retaining the live copy if replacement is unsafe.
 * Marks the document loaded so later dirty pages can save; it neither dirties this
 * page nor starts a save. Cost O(loaded pages + page blocks + cached sheet
 * dimensions), plus evicted page blocks if the working set exceeds its cap. */
export function loadRoutedPage(dto: PageDto & { id?: string }): void {
  ensurePageLoaded(dto);
  setDoc("loaded", true);
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
  releaseGroup(name);
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
  retirePageInstance(name);
  invalidateAllMatrixDimensions();
}

/** Delete a page: tombstone it (so any pending/in-flight save can't recreate the
 *  file), drop its dirty/baseline/conflict state, remove it from the working set
 *  and feed, then delete on disk. Routing deletion through the store — rather than
 *  calling the backend directly — is what prevents a queued baseRev=null save from
 *  resurrecting a just-typed, never-saved page. Returns backend success.
 *
 *  `retireRoutes`, when given, runs synchronously once the disk delete has
 *  succeeded and before the page leaves the working set, so no pane can render
 *  a route that names an already-purged page (GH #376: a black frame on
 *  Android). It must do UI-only work such as closing pane routes; if it throws,
 *  the delete still completes and the page is still retired. It never runs when
 *  the delete is refused or fails. */
export async function deletePage(
  name: string,
  kind: PageKind,
  expectedPath?: string,
  retireRoutes?: () => void,
): Promise<boolean> {
  if (graphRewriteFrozen()) return false;
  const binding = captureBinding();
  const generation = pageInstanceGeneration(name);
  const loaded = pageByName(name);
  if (expectedPath && loaded?.id !== expectedPath) return false;
  if (loaded?.readOnly || loaded?.guide) return false;
  if (conflictReason(name)?.kind === "released") {
    pushToast(`Resolve the conflict on “${name}” first.`, "error");
    return false;
  }
  // A group member must land with all its partners before the page can be
  // deleted. Otherwise deleting a source can remove the only live disk copy of
  // moved content. A failed or conflicted group leaves the delete untouched.
  const wasGrouped = !!group(name);
  const refuseGroupDelete = () => {
    const blocked = [...(group(name)?.members ?? [name])].find(isConflicted) ?? name;
    pushToast(`Resolve the conflict on “${blocked}” first.`, "error");
    return false;
  };
  if (wasGrouped) {
    if ([...(group(name)?.members ?? [])].some(isConflicted)) return refuseGroupDelete();
  }
  const needsFlush = wasGrouped || ((isDirty(name) || isSaving(name)) && !isConflicted(name));
  if (needsFlush && !(await flushPage(name))) {
    if (!wasGrouped) return false;
    if ([...(group(name)?.members ?? [])].some(isConflicted)) return refuseGroupDelete();
    pushToast(`Couldn't save “${name}”; the page was not deleted.`, "error");
    return false;
  }
  // Hold off any new successor request during the disk delete. If another
  // intent joined while the preceding request settled, leave it intact.
  const releaseReservation = wasGrouped ? await reserveGroupMemberDeletion(name) : undefined;
  if (group(name)) {
    releaseReservation?.();
    pushToast(`Resolve the conflict on “${name}” first.`, "error");
    return false;
  }
  if (!stillBound(binding) || pageInstanceGeneration(name) !== generation || graphRewriteFrozen()) {
    releaseReservation?.();
    return false;
  }
  if (conflictReason(name)?.kind === "released") {
    releaseReservation?.();
    pushToast(`Resolve the conflict on “${name}” first.`, "error");
    return false;
  }
  // Tombstone first so any queued/in-flight save no-ops during the delete, but
  // DON'T drop the in-memory page until the backend actually deletes it — if the
  // delete fails, the page (and its unsaved edits) must survive.
  tombstone(name);
  try {
    await deletePageOnDisk(name, kind, expectedPath);
  } catch {
    releaseReservation?.();
    if (!stillBound(binding)) return false;
    untombstone(name); // delete failed — lift the tombstone; page + edits stay intact
    return false;
  }
  releaseReservation?.();
  if (!stillBound(binding)) return false;
  try {
    retireRoutes?.();
  } catch {
    // UI-only: a durable delete still retires the loaded page below.
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
const draftPins = new Set<() => string | null | undefined>();
/** Keep the page a draft outside the block editor will write to (e.g. a sheet
 * prop cell, the page-title rename input) from being evicted OR replaced by a
 * navigation/feed/watcher reload (og 15a K17a; og 20b contract 2 — one registry
 * answers both, via `pinnedPages` and `reloadDisposition`). `page` is called at
 * each check and names a store page name (null/undefined pins nothing). Returns the unpin function; nothing unpins automatically — call it
 * from `onCleanup`. The pin is not cleared by a graph switch (it is re-read
 * against the new graph). O(1) to register; each eviction check calls every
 * registered accessor. */
export function pinPageWhileDrafting(page: () => string | null | undefined): () => void {
  draftPins.add(page);
  return () => {
    draftPins.delete(page);
    replayDeferredExternalReloads(); // a watcher change declined for this draft (GH #337)
  };
}
function draftPinned(name: string): boolean {
  for (const draft of draftPins) if (draft() === name) return true;
  return false;
}
function pinnedPages(): Set<string> {
  const pin = new Set<string>(doc.feed);
  for (const draft of draftPins) { const name = draft(); if (name) pin.add(name); }
  for (const r of paneRouteProvider()) {
    if (r.kind === "page") pin.add(r.name);
  }
  for (const it of rightSidebar()) pin.add(it.kind === "page" ? it.name : it.page);
  for (const name of dirtyPages()) pin.add(name);
  for (const name of groupedPages()) pin.add(name);
  for (const name of savingPages()) pin.add(name);
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
export function reloadPage(dto: PageDto & { id?: string }) {
  const existing = doc.pages.find((page) => page.name === dto.name);
  if (existing && group(dto.name) && !pageContentMatches(dto, existing)) releaseGroup(dto.name);
  upsertPage(dto);
}

/** A watcher result may arrive after the user starts editing. Keep the explicit
 * conflict-bar reload above as the only unconditional replacement. */
export function reloadPageIfStillSafe(name: string, dto: PageDto & { id?: string }): boolean {
  if (reloadDisposition(name) !== "reload") return false;
  upsertPage(dto);
  return true;
}

/** After a PDF highlight write changed an `hls__` page on disk, refresh its
 *  loaded copy (main view or sidebar) so its content AND save baseline (baseRev)
 *  track disk — otherwise a later editor save would conflict against the highlight
 *  write. Skips a page with unsaved edits / an open conflict: the caller flushes
 *  those FIRST so they're on disk and merged in, rather than clobbered here. */
export async function reloadHlsIfLoaded(name: string): Promise<void> {
  if (!pageByName(name)) return;
  if (isDirty(name) || isConflicted(name)) return;
  const generation = pageInstanceGeneration(name);
  const owner = graphOwner(() => pageInstanceGeneration(name) === generation);
  const result = await readOwned(owner, backend().getPage(name, "page"));
  if (result.kind === "current" && result.value)
    reloadPageIfStillSafe(name, result.value);
}
function evictIfNeeded() {
  if (doc.pages.length <= WORKING_SET_CAP) return;
  const pin = pinnedPages();
  const evicted: string[] = [];
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
        evicted.push(name);
      }
    })
  );
  for (const name of evicted) retirePageInstance(name);
  invalidateAllMatrixDimensions();
}

/** Clear the entire working set. Used for test isolation and when switching
 *  graphs; normal navigation is additive (keeps satellite pages alive). Also
 *  cancels queued saves and clears dirty flags after the caller flushes the old
 *  graph. An IPC save already issued cannot be cancelled here. */
export function resetStore() {
  invalidateBinding();
  // Cancel queued saves and clear save guard state (timers, graph token,
  // dirty/baseline/tombstone); callers flush issued saves before switching.
  resetSaveState();
  // Drop undo/redo history: it holds page snapshots from the OLD graph; an undo
  // after a graph switch would otherwise restore (and save) those into the new
  // graph, even creating a foreign page there.
  clearUndoHistory();
  // Drop the old graph's seeded facets (the never-evicted tier) so they don't linger
  // across the switch (audit P2).
  clearSeededFacets();
  clearMatrixDimensionCache();
  resetReferenceSectionState();
  for (const name of pageInstanceGenerations.keys()) retirePageInstance(name);
  setDoc({ byId: {}, pages: [], feed: [], loaded: false });
  clearCollapseEpochs();
  endEdit("graph-switch");
  notifyModeReset();
}

// A navigation/feed load must NOT replace a page that holds unsaved input with a
// fresh disk DTO — e.g. you edited it in the sidebar, then opened it in the main
// view before the debounce saved, or it is open in a block editor / title rename /
// sheet cell. Keep the live nodes; the disk version would otherwise be served and
// the next save could write it, silently dropping the edit (GH #304 family, og 20b
// contract 2). Same gate as the watcher: `reloadDisposition`. (reloadPage / "use
// disk version" still replace explicitly via upsertPage — that is the user's choice.)
function upsertUnlessDirty(dto: PageDto & { id?: string }) {
  const disp = pageByName(dto.name) ? reloadDisposition(dto.name) : "reload";
  // A held page ("skip") keeps its loaded copy, and the declined read replays
  // through the watcher's deferred reload once the hold releases, so a feed or
  // navigation read landing mid-hold is never silently dropped (master
  // ba80a151e). A "conflict" page's own save settles it.
  if (disp === "skip" && (dto.rev == null || dto.rev !== baseRevFor(dto.name))) deferExternalReload(dto.name, { name: dto.name, kind: dto.kind, path: dto.id, created: false, removed: false });
  if (disp !== "reload") return;
  upsertPage(dto);
}

export type ReloadDisposition = "reload" | "conflict" | "skip";
/** What to do when page `name` changed on disk (external editor / Syncthing),
 *  for the file-watcher reload sites. One rule so the (formerly 4 hand-coded)
 *  branches in Page.tsx can't diverge:
 *  - `"conflict"` — it has unsaved edits / an open conflict: surface a conflict,
 *    NEVER clobber the in-memory edit with the disk version.
 *  - `"skip"` — a block on it is being edited (don't yank the caret), a block
 *    move is mid-flight (the textarea is transiently blurred), or a component
 *    draft pinned it (`pinPageWhileDrafting`): leave it alone.
 *  - `"reload"` — safe to replace the loaded copy with the disk version.
 *  Every replacement of a loaded instance except the user's explicit "use disk"
 *  asks this: the watcher, navigation/feed loads (`upsertUnlessDirty`),
 *  `ensurePageLoaded`, and `reloadHlsIfLoaded`. */
export function reloadDisposition(name: string): ReloadDisposition {
  // `isSaving` too: `doSave` clears `dirty` BEFORE the `await savePages`, so during the
  // save IPC the page is no longer dirty but its edit isn't durable. Reloading then
  // would clobber the in-memory edit + drop its undo, and the in-flight save would
  // conflict — silent loss (audit H1). The in-flight save's baseRev check surfaces the
  // real conflict.
  if (isDirty(name) || isConflicted(name) || isSaving(name) || group(name)) return "conflict";
  const ed = editingId();
  if ((ed && doc.byId[ed]?.page === name) || isBlockMoving() || draftPinned(name)) return "skip";
  return "reload";
}

/** Load a single page and make it the main view. */
export function loadSingle(dto: PageDto & { id?: string }, opts: { endEdit?: boolean } = {}) {
  upsertUnlessDirty(dto);
  setDoc("feed", [dto.name]);
  setDoc("loaded", true);
  if (opts.endEdit !== false) endEdit("page-navigation");
  evictIfNeeded();
}

/** Load the journals feed as the main view. Normal refresh replaces safe pages;
 * a calendar rollover can add the returned days while retaining every existing
 * feed page and its mounted editor. An optional owner check refuses stale
 * publication. Returns false only for a stale request. Cost is O(returned page
 * blocks + loaded pages + feed days + any pages evicted at the working-set cap);
 * no disk write or failure is exposed here. */
export function loadFeed(dtos: (PageDto & { id?: string })[], opts: { endEdit?: boolean; preserveExisting?: boolean; isRequestLive?: () => boolean } = {}): boolean {
  if (opts.isRequestLive?.() === false) return false;
  for (const d of dtos) {
    // A calendar rollover adds the new day while keeping every mounted older
    // feed page, including its editor and any unsaved text, as the same object.
    if (opts.preserveExisting && doc.feed.includes(d.name)) continue;
    if (opts.preserveExisting) ensurePageLoaded(d);
    else upsertUnlessDirty(d);
  }
  if (opts.isRequestLive?.() === false) return false;
  const incoming = dtos.map((d) => d.name);
  setDoc("feed", opts.preserveExisting
    ? [...incoming, ...doc.feed.filter((name) => !incoming.includes(name))]
    : incoming);
  setDoc("loaded", true);
  if (opts.endEdit !== false) endEdit("page-navigation");
  evictIfNeeded();
  return true;
}

/** Append more pages to the journals feed (infinite scroll). */
export function appendFeed(dtos: (PageDto & { id?: string })[]) {
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

/** Re-assert "the journals feed always shows today" on the LIVE feed after today's
 *  journal is deleted from it. The feed loader's `withToday` only runs on (re)load,
 *  so deleting today in place while viewing the feed would otherwise leave the top
 *  blank until you navigate away and back (#17). No-op if today is still in the feed
 *  (e.g. it was an OLDER day that got deleted). The placeholder is empty and
 *  writable — `upsertPage` lifts the delete tombstone, so the first keystroke saves
 *  a fresh file, exactly like reopening the journal. */
export function restoreTodayJournalInFeed() {
  const title = journalTitle(appNow());
  if (doc.feed.includes(title)) return;
  upsertUnlessDirty(emptyPage(title, "journal"));
  setDoc("feed", [title, ...doc.feed]);
}
