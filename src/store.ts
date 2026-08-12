// The live editing tree. The frontend owns this during a session; all
// keystrokes and structural ops mutate it synchronously (zero IPC). Persistence
// is a debounced per-page save to Rust. See plan §"block editor model".
//
// Supports multiple pages at once (the journals feed): a single global `byId`
// map, each node tagged with its owning `page`, and an ordered `pages` list
// each with its own roots. A single-page route is just a feed of length one.

import { createStore, produce, unwrap } from "solid-js/store";
import { createSignal, createMemo, createRoot } from "solid-js";
import type { ActivationIntent, BlockDto, EditorActivationHandle, Format, PageDto, PageKind, RefGroup } from "./types";
import type { ClipboardBlock, ClipboardPayloadData, ClipboardPayloadSlot, ClipboardSourcePage } from "./clipboard";
import {
  CLIPBOARD_PAYLOAD_MAX_BLOCKS,
  CLIPBOARD_PAYLOAD_MAX_RAW_BYTES,
  consumeCutGrant,
} from "./clipboard";
import type { Route } from "./router";
import { parseOutline, type OutlineNode } from "./editor/outline";
import type { ExportNode } from "./editor/exportText";
import { backend } from "./backend";
import { managedStorageRuntime } from "./managedStorageRuntime";
import { resetReferenceSectionState } from "./referenceSectionState";
import {
  isConflicted,
  clearConflict,
  rightSidebar,
  conflicts,
  pushToast,
  graphMeta,
  graphEpoch,
  graphTransitioning,
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
import { upsertPropertyLine, readPropertyValue, splitProps, joinProps, isBuiltinHidden, hideAll, isPropertiesOnly, isPageHeaderPropertiesOnly, parsePageHeaderPropertyLine, splitPagePreamble } from "./editor/properties";
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
import { notifyModeReset, notifyOutlineSelectionStarted, onGraphRebound } from "./modeHooks";
import { sheetConfigFromRaw } from "./sheet/config";
import { clearMatrixDimensionCache, invalidateAllMatrixDimensions } from "./sheet/matrix";
import { applyMarkerTransition } from "./logbook";
import { cycleMarkerSmart } from "./editor/repeat";
import {
  recordClipboardDirtyPageForTest,
  recordClipboardPhaseForTest,
  recordClipboardUndoSnapshotForTest,
  recordClipboardWorkForTest,
} from "./clipboardWorkProbe";
import {
  markDirty as markDirtyInner,
  isDirty,
  scheduleSave,
  flushPage,
  flushPageToQuiescence,
  flushAll,
  forceSave,
  canForceSave,
  addDirty,
  dirtyPages,
  savingPages,
  setBaseRev,
  tombstoneIfQuiescent,
  untombstone,
  isTombstonedFile,
  tombstoneCovers,
  graphBinding,
  saveBaselineFor,
  forgetSaveState,
  resetSaveState,
  isSaving,
  holdSourcesForDest,
  trackAssetWrite,
  flushCutSourcePages,
  cutSourcePagesRetired,
} from "./persistence";
// The debounced persistence engine lives in persistence.ts; re-exported here so
// the rest of the app keeps importing the save API from the store.
type StoreMutationObservation = { kind: "publication" | "dirty" | "undo-snapshot"; page?: string };
let storeMutationObserverForTest: ((observation: StoreMutationObservation) => void) | null = null;

/** Test-only observation seam for proving work shape at the actual store and
 * persistence entry points. Production leaves this unset. */
export function __setStoreMutationObserverForTest(
  observer: ((observation: StoreMutationObservation) => void) | null,
) {
  storeMutationObserverForTest = observer;
}

export function __setPageMutationEffectFailureForTest(enabled: boolean): void {
  if (import.meta.env.MODE !== "test") return;
  pageMutationEffectFailureForTest = enabled;
}

/** Production keeps the exact persistence function identity and call path. Vitest
 * selects the observing façade once at module initialization, never per edit. */
export const markDirty: typeof markDirtyInner = import.meta.env.MODE === "test"
  ? ((...args: Parameters<typeof markDirtyInner>) => {
      storeMutationObserverForTest?.({ kind: "dirty", page: args[0] });
      recordClipboardDirtyPageForTest(args[0]);
      return markDirtyInner(...args);
    }) as typeof markDirtyInner
  : markDirtyInner;

export {
  isDirty,
  isSaving,
  scheduleSave,
  flushPage,
  flushAll,
  forceSave,
  canForceSave,
  trackAssetWrite,
};

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

const [doc, setDocInner] = createStore<DocState>({ byId: {}, pages: [], feed: [], loaded: false });
export { doc };

/** Production keeps Solid's original setter. Vitest chooses the observing
 * façade once at module initialization, so there is no production wrapper or
 * observer check on the editing hot path. */
export const setDoc: typeof setDocInner = import.meta.env.MODE === "test"
  ? ((...args: unknown[]) => {
      storeMutationObserverForTest?.({ kind: "publication" });
      return (setDocInner as (...innerArgs: unknown[]) => unknown)(...args);
    }) as typeof setDocInner
  : setDocInner;

export interface LoadedIdentityWorkForTest {
  loaded_identity_passes: number;
  loaded_identity_nodes_scanned: number;
  loaded_identity_raw_bytes_scanned: number;
  incoming_identity_ids_checked: number;
}

const loadedIdentityWorkForTest: LoadedIdentityWorkForTest = {
  loaded_identity_passes: 0,
  loaded_identity_nodes_scanned: 0,
  loaded_identity_raw_bytes_scanned: 0,
  incoming_identity_ids_checked: 0,
};

/** Reset the F3 identity-work receipt. Test-only branches in the collision
 * helper are dead-code-eliminated from production builds. */
export function __resetLoadedIdentityWorkForTest(): void {
  if (import.meta.env.MODE !== "test") return;
  loadedIdentityWorkForTest.loaded_identity_passes = 0;
  loadedIdentityWorkForTest.loaded_identity_nodes_scanned = 0;
  loadedIdentityWorkForTest.loaded_identity_raw_bytes_scanned = 0;
  loadedIdentityWorkForTest.incoming_identity_ids_checked = 0;
}

/** Snapshot the resettable F3 identity-work receipt for focused tests. */
export function __loadedIdentityWorkForTest(): LoadedIdentityWorkForTest {
  return { ...loadedIdentityWorkForTest };
}

// Keep the existing fail-closed coverage for a Markdown `id::` line and an Org
// `:id:` property line. In particular, do not require an Org drawer here: the
// previous safety check refused any matching loaded raw line, including one
// introduced by setRaw before the byId key can be reconciled.
const RAW_BLOCK_ID_PROPERTY_RE = /(?:^|\r?\n)[ \t]*(?:id[ \t]*::|:id:)[ \t]*([^\r\n]*?)[ \t]*(?=\r?\n|$)/gi;

/**
 * Does one candidate list collide with a live identity in the loaded document?
 *
 * The old per-id predicate re-scanned all loaded keys and raws for every
 * candidate. Preserved cut paste and redo now build the loaded identity set
 * once, then check the moving IDs in O(moved IDs). Raw properties remain part
 * of the set so a synchronous setRaw cannot open a duplicate-id window.
 */
function hasLoadedIdentityCollision(incomingIds: readonly string[]): boolean {
  if (import.meta.env.MODE === "test") {
    loadedIdentityWorkForTest.incoming_identity_ids_checked += incomingIds.length;
  }

  // Reject an internally duplicated moved payload before consulting the loaded
  // document. paste normally catches this earlier, but redo history can carry
  // preserved IDs from an older snapshot and must be equally fail-closed.
  const incoming = new Set<string>();
  for (const id of incomingIds) {
    const normalized = id.toLowerCase();
    if (incoming.has(normalized)) return true;
    incoming.add(normalized);
  }

  if (import.meta.env.MODE === "test") loadedIdentityWorkForTest.loaded_identity_passes++;
  const loaded = new Set<string>();
  for (const [key, node] of Object.entries(doc.byId)) {
    loaded.add(key.toLowerCase());
    const raw = node.raw;
    if (import.meta.env.MODE === "test") {
      loadedIdentityWorkForTest.loaded_identity_nodes_scanned++;
      loadedIdentityWorkForTest.loaded_identity_raw_bytes_scanned += new TextEncoder().encode(raw).byteLength;
    }
    RAW_BLOCK_ID_PROPERTY_RE.lastIndex = 0;
    let match: RegExpExecArray | null;
    while ((match = RAW_BLOCK_ID_PROPERTY_RE.exec(raw)) !== null) {
      const identity = match[1].trim();
      if (identity) loaded.add(identity.toLowerCase());
    }
  }
  return incomingIds.some((id) => loaded.has(id.toLowerCase()));
}

// A generation identifies one exact loaded page instance. It is deliberately
// frontend-only and monotonic across resets: a later page with the same name and
// path must never satisfy a cut payload captured from an evicted/deleted/rebound
// instance. Stage B uses this at its durable-retirement boundary.
let pageInstanceClock = 0;
const pageInstanceGenerations = new Map<string, number>();

// Advances on every CONTENT edit, which `pageInstanceGeneration` deliberately
// does not: that counter tracks page instances (install/retire), so `setRaw`
// leaves it unchanged. An authority captured at a click therefore cannot use it
// to tell "the user typed while my read was in flight" from "nothing happened" —
// which is the difference between honouring a discard and destroying text the
// user entered after asking for it. (GH #254 increment 3.)
let editClock = 0;
const editGenerations = new Map<string, number>();

// Advances whenever a component-local editor transaction starts. Unlike the
// content-edit generation above, this also covers input that intentionally has
// not reached the store yet: a title-rename draft and an active IME composition.
// A discard click captures both generations so it can authorise the state the
// user saw without authorising a new local transaction begun during an await.
let editorTransactionClock = 0;
const editorTransactionGenerations = new Map<string, number>();

/** Current content-edit generation for a page. */
export function editGeneration(name: string): number {
  return editGenerations.get(name) ?? 0;
}

export function bumpEditGeneration(name: string): void {
  editGenerations.set(name, ++editClock);
}

/** Current component-local editor-transaction generation for a page. */
export function editorTransactionGeneration(name: string): number {
  return editorTransactionGenerations.get(name) ?? 0;
}

/** The page-instance generation WITHOUT creating one for a page that has none.
 *
 *  `pageInstanceGeneration` lazily activates, so reading it as a check would mint
 *  a generation and mutate the identity cut retirement compares. */
export function peekPageInstanceGeneration(name: string): number | undefined {
  return pageInstanceGenerations.get(name);
}

function activatePageInstance(name: string): number {
  const generation = ++pageInstanceClock;
  pageInstanceGenerations.set(name, generation);
  return generation;
}

function retirePageInstance(name: string): void {
  ++pageInstanceClock;
  pageInstanceGenerations.delete(name);
  editorTransactionGenerations.delete(name);
}

/** Current exact loaded-page generation, or null when that page is absent. */
export function pageInstanceGeneration(name: string): number | null {
  if (!pageByName(name)) return null;
  // Direct setDoc page seeding is supported by model tests and small embedded
  // surfaces; lazily bind it to the same invariant as loader-created pages.
  return pageInstanceGenerations.get(name) ?? activatePageInstance(name);
}

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

/**
 * Live editor activations, keyed by page name.
 *
 * Deliberately NOT a field of `FeedPage`. `clonePages` spread-copies every field
 * of a page into history snapshots, so a token living on the page object would be
 * carried into every snapshot and reinstalled by `applyEntry` — handing a restored
 * editor a RETIRED activation, whose conflicts could then never be answered. An
 * activation identifies a live editor instance, and a copy of a page is not one.
 * (GH #254 increment 3; the failure was reproduced against a `FeedPage` field.)
 */
const editorActivations = new Map<string, number>();

/** The activation for `pageName`, if this page currently has a live editor. */
export function editorActivationFor(pageName: string): number | undefined {
  return editorActivations.get(pageName);
}

/** Record a freshly minted activation for `pageName`. */
export function setEditorActivation(pageName: string, activation: number): void {
  editorActivations.set(pageName, activation);
}

/**
 * Forget `pageName`'s activation locally, but only if it is still the one named.
 *
 * The local half of compare-and-retire: a retirement racing a newer activation
 * must not drop the newer one. The core is told separately, and its own
 * compare-and-retire is the authority.
 */
export function clearEditorActivation(pageName: string, activation?: number): boolean {
  const live = editorActivations.get(pageName);
  if (live === undefined) return false;
  if (activation !== undefined && live !== activation) return false;
  editorActivations.delete(pageName);
  prospectiveTargets.delete(pageName);
  return true;
}

/**
 * Prospective targets for editors activated with no file yet.
 *
 * Kept beside the activation registry rather than written onto the page: writing
 * it into the store mid-save was tried and reverted, because mutating the page
 * while a save is building its snapshot disturbs cut retirement, which is
 * authority-bound to the exact loaded instance. This is read at the DTO boundary
 * instead, which is where the core actually needs it — its drift/re-resolve
 * branch only runs for a pinned path. (GH #254 increment 3.)
 */
const prospectiveTargets = new Map<string, string>();

export function setProspectiveTarget(pageName: string, target: string): void {
  prospectiveTargets.set(pageName, target);
}

function recordEditorActivation(pageName: string, handle: EditorActivationHandle): void {
  setEditorActivation(pageName, handle.activation);
  // Keep the exact target beside a pathless editor after first creation too.
  // The save response is the first authoritative place the frontend learns the
  // resolved path, and `pageToDto` must keep pinning later saves to it.
  prospectiveTargets.set(pageName, handle.target);
}

async function retireExactEditorActivation(
  pageName: string,
  target: string | undefined,
  activation: number,
): Promise<void> {
  clearEditorActivation(pageName, activation);
  if (!target) return;
  await backend().retireEditorActivation(target, activation).catch(() => {});
}

async function retireMintedActivation(handle: EditorActivationHandle | null): Promise<void> {
  if (!handle) return;
  await backend().retireEditorActivation(handle.target, handle.activation).catch(() => {});
}

/** Drop every activation — graph reset and teardown. */
export function clearAllEditorActivations(): void {
  editorActivations.clear();
  prospectiveTargets.clear();
}

// A backend reopen installs a FRESH `Graph` whose activation registry is empty,
// so every token this side still holds names an editor the core has never heard
// of. Keeping them produced conflicts nobody could resolve: the ordinary save
// minted a banner carrying a retained token, and the matching force was refused
// `conflict_authority.superseded`, so BOTH banner buttons only re-observed into
// the same dead conflict. The registry has to be dropped with the graph that
// issued it. (GH #254 increment 3, round 15.)
onGraphRebound(clearAllEditorActivations);

/**
 * Retire `pageName`'s editor identity locally AND in the core.
 *
 * An activation that outlives its editor is not inert: with same-path activation
 * idempotence, a stale live token would be handed to the NEXT editor of that path,
 * which is exactly the cross-instance authority this increment exists to prevent.
 * So retirement is driven by the same events that retire the frontend instance —
 * eviction, `forgetPage`, reset — and is compare-and-retire on both sides, never a
 * bare "retire this path": a retirement racing a newer activation must not revoke
 * the newer editor. (GH #254 increment 3.)
 */
export function retireEditorFor(pageName: string, path?: string): void {
  const activation = editorActivations.get(pageName);
  if (activation === undefined) return;
  const target = path
    ?? doc.pages.find((p) => p.name === pageName)?.path
    ?? prospectiveTargets.get(pageName);
  void retireExactEditorActivation(pageName, target, activation);
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
function upsertPage(dto: PageDto): boolean {
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
    return false;
  }
  // Replacing an already-loaded copy means the page's content changed under us
  // (a conflict-resolution / watcher reload). Any undo entry predating this reload
  // is stale — replaying it would clobber the just-loaded (external) version, so
  // drop those entries. (A first load has no prior entries → no-op.)
  const replacing = !!existing;
  // Activation retirement is deliberately NOT done here. Editable DTOs enter
  // through the two-phase installer below, which records B before compare-
  // retiring exact A. Retiring by name from this mutation primitive can race a
  // concurrent installer and destroy the activation it just installed.
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
  return true;
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

type CapturedEditorInstance = {
  generation: number;
  path?: string;
  kind: PageKind;
  activation?: number;
  activationTarget?: string;
};

function captureEditorInstance(name: string): CapturedEditorInstance | null {
  const page = pageByName(name);
  const generation = pageInstanceGeneration(name);
  if (!page || generation === null) return null;
  return {
    generation,
    path: page.path,
    kind: page.kind,
    activation: editorActivationFor(name),
    activationTarget: page.path || prospectiveTargets.get(name),
  };
}

function isExactCapturedInstance(name: string, captured: CapturedEditorInstance | null): boolean {
  const current = pageByName(name);
  if (!captured) return !current && peekPageInstanceGeneration(name) === undefined;
  return !!current
    && current.kind === captured.kind
    && current.path === captured.path
    && peekPageInstanceGeneration(name) === captured.generation
    && editorActivationFor(name) === captured.activation
    && (current.path || prospectiveTargets.get(name)) === captured.activationTarget;
}

export type EditorInstallOptions = {
  /** Binding captured before the read that produced the DTO. */
  expectedGraphBinding?: number;
  /** Explicit user-authorised discard; identity and binding still re-check. */
  bypassReplacementGate?: boolean;
  /** Surface/component ownership spanning the activation await. */
  isRequestLive?: () => boolean;
  /** Awaited only after disk read and replacement activation have succeeded.
   * Returning false compare-retires the minted activation without installing. */
  beforeInstall?: () => Promise<boolean>;
};

/** Load a page into the editable working set through the activation boundary.
 *
 * Replacement is a two-phase protocol: capture exact A under the synchronous
 * full gate; await B's activation; re-check both the gate and exact A; install B
 * and record it; then compare-retire A. A failed/stale activation never installs
 * an editable DTO. Same-instance same-content hydration stays idempotent. */
export async function ensurePageLoaded(
  dto: PageDto,
  options: EditorInstallOptions = {},
): Promise<InstanceRefusal | null> {
  const binding = options.expectedGraphBinding ?? graphBinding();
  if (binding !== graphBinding() || options.isRequestLive?.() === false) {
    return { reason: "stale-instance", page: dto.name };
  }

  const captured = captureEditorInstance(dto.name);
  const incumbent = pageByName(dto.name);
  const samePath = !!incumbent && (incumbent.path ?? "") === (dto.path ?? "");
  const sameInstanceHydration = !!incumbent && samePath && pageContentMatches(dto, incumbent);
  const replacing = !!captured && !sameInstanceHydration;

  // Phase 1: the complete synchronous gate, before activation. A same-instance
  // hydration does not replace anything, so a dirty editor may safely acquire
  // the identity it already owns.
  if (replacing && !options.bypassReplacementGate && !mayReplaceInstance(dto.name)) {
    return { reason: "unsaved-changes", page: dto.name };
  }

  // Already active exact-instance hydration is the idempotent fast path.
  if (sameInstanceHydration && editorActivationFor(dto.name) !== undefined) return null;

  let handle: EditorActivationHandle | null = null;
  const editable = !dto.read_only && !dto.guide;
  if (editable) {
    try {
      if (dto.path) {
        // Reuse is only valid when this exact frontend instance already owns the
        // activation, and that case returned through the fast path above.  With
        // no local activation, mint a replacement even for a first installation:
        // a best-effort retirement from an older destroyed editor may have failed,
        // and inheriting that stale core record would cross editor episodes.
        const intent: ActivationIntent = "replace";
        handle = await backend().activateEditor(dto.path, intent, dto.rev ?? null);
      } else {
        handle = await backend().activateAbsentEditor(dto.name, dto.kind);
      }
    } catch {
      return { reason: "activation-failed", page: dto.name };
    }
  }

  // Presentation spends one-shot conflict authority, so identity must be
  // re-checked after activation and BEFORE that fallible/consuming operation.
  // The same check still runs again below after presentation, closing changes
  // that race the presentation await itself.
  if (
    binding !== graphBinding()
    || options.isRequestLive?.() === false
    || !isExactCapturedInstance(dto.name, captured)
  ) {
    await retireMintedActivation(handle);
    return { reason: "stale-instance", page: pageByName(dto.name)?.name ?? dto.name };
  }

  if (options.beforeInstall) {
    let proceed = false;
    try {
      proceed = await options.beforeInstall();
    } catch {
      proceed = false;
    }
    if (!proceed) {
      await retireMintedActivation(handle);
      return { reason: "stale-instance", page: pageByName(dto.name)?.name ?? dto.name };
    }
  }

  // Phase 3: both graph ownership and exact incumbent identity must survive the
  // await. Then re-evaluate the full gate in the same synchronous turn as the
  // install. A raced B is compare-retired; A/current remains untouched.
  if (
    binding !== graphBinding()
    || options.isRequestLive?.() === false
    || !isExactCapturedInstance(dto.name, captured)
  ) {
    await retireMintedActivation(handle);
    return { reason: "stale-instance", page: pageByName(dto.name)?.name ?? dto.name };
  }
  if (replacing && !options.bypassReplacementGate && !mayReplaceInstance(dto.name)) {
    await retireMintedActivation(handle);
    return { reason: "unsaved-changes", page: dto.name };
  }

  if (sameInstanceHydration) {
    if (handle) recordEditorActivation(dto.name, handle);
    return null;
  }

  // Phase 4: publish B's instance first, then its identity, then retire exact A.
  // `clearEditorActivation` is compare-based, so retiring A cannot clear B.
  upsertPage(dto);
  if (handle) recordEditorActivation(dto.name, handle);
  if (captured?.activation !== undefined) {
    await retireExactEditorActivation(
      dto.name,
      captured.activationTarget,
      captured.activation,
    );
  }
  evictIfNeeded();
  return null;
}

/** Install the isolated quick-capture scratch DTO without touching core.
 * It is the sole C9 exception: local-only, never graph-persisted, and therefore
 * deliberately has no editor activation. */
export function installCaptureScratchPage(dto: PageDto): void {
  upsertPage(dto);
}

/** Load a page selected by the main graph router. `ensurePageLoaded` also serves
 * the isolated quick-capture window, where `doc.loaded` must stay false so its
 * scratch editor can never write the graph. A successfully resolved main route,
 * however, is sufficient to arm ordinary persistence even when an invalidated
 * Journals reload has not landed yet. */
export async function loadRoutedPage(
  dto: PageDto,
  expectedGraphBinding = graphBinding(),
): Promise<InstanceRefusal | null> {
  const refusal = await ensurePageLoaded(dto, { expectedGraphBinding });
  if (refusal) {
    // A refused route must not leave the surface silently blank — that is a trap,
    // not a safeguard. The route currently marks itself loaded after this call and
    // its loader effect watches route/graph identity rather than the incumbent's
    // save lifecycle, so nothing would retry on its own. Say what is holding the
    // file and what resolves it, so the user can act and ask again.
    // (GH #254 increment 3.)
    const message = refusal.reason === "unsaved-changes"
      ? `“${refusal.page}” has unsaved changes, so the other file with that name can't be shown yet. ` +
        `Save or resolve it, then open the file again.`
      : refusal.reason === "activation-failed"
        ? `“${refusal.page}” could not be activated for editing. Open it again to retry.`
        : `The request for “${refusal.page}” became stale. Open it again to retry.`;
    pushToast(message, "error");
    return refusal;
  }
  setDoc("loaded", true);
  return null;
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
  retireEditorFor(name);
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
  // AFTER the dirty/conflict state is cleared and the page is gone — announcing
  // at the top ran while `mayReplaceInstance` was still false, so the
  // announcement was correctly dropped and then nothing swept again, stranding a
  // waiting request forever. This is the externally-deleted "Use disk version"
  // route and the successful `deletePage` route, which share this ordering.
  // Swept, not named: the page no longer exists, and other watchers may have been
  // freed by the same teardown. (GH #254 increment 3.)
  sweepReplaceable();
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
  // Capture the exact loaded instance before awaiting.  A replacement, graph
  // reload, or path rebind must not let this delete tombstone a later editor that
  // happens to reuse its logical name.
  const captured = loaded && {
    name: loaded.name,
    kind: loaded.kind,
    path: loaded.path,
    generation: pageInstanceGeneration(name),
  };
  // A by-name delete may target an unloaded page. With no live instance or draft
  // to protect, it can publish its name-wide tombstone synchronously below.
  if (captured && (captured.kind !== kind || captured.generation === null)) return false;
  const capturedConflicted = !!captured && isConflicted(name);
  const stillCaptured = () => {
    if (!captured) return false;
    const current = pageByName(name);
    return !!current
      && current.name === captured.name
      && current.kind === captured.kind
      && current.path === captured.path
      && pageInstanceGeneration(name) === captured.generation;
  };

  // A conflicted draft is deliberately not flushed: its current actor winner,
  // not unrecoverable draft bytes, is what the warning says reaches trash.  For
  // every other page, drain through quiescence rather than one save so a keystroke
  // injected during that first save either becomes a second accepted snapshot or
  // causes this delete to refuse with the draft still live.
  if (
    captured
    && !capturedConflicted
    && !(await flushPageToQuiescence(name))
  ) return false;
  // The identity proof and persistence retirement run back-to-back without a
  // yield. tombstoneIfQuiescent re-checks dirty/saving/conflict state in the same
  // synchronous turn that publishes the marker, closing the resolved-Promise
  // handoff after flushPageToQuiescence.
  if (
    (captured && !stillCaptured())
    || !tombstoneIfQuiescent(name, capturedConflicted, expectedPath)
  ) return false;
  try {
    if (expectedPath) await backend().deletePage(name, kind, expectedPath);
    else await backend().deletePage(name, kind);
  } catch {
    untombstone(name); // delete failed — lift the tombstone; page + edits stay intact
    // Anything that parked itself while this page looked deleted may proceed now.
    notifyPageBecameReplaceable(name);
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
  // A page whose save is in flight is ALSO not in `dirty` — `doSave` removes it
  // before awaiting the backend. Evicting it there loses the edit outright, and
  // if that save then fails transiently `doSave` re-adds a name with no page
  // behind it, which `pageToDto` cannot serialize: the name is stuck in `dirty`
  // forever and `flushAll()` can never succeed again. (Direct Files data-safety
  // audit, 2026-08-09, finding 6.)
  for (const name of savingPages()) pin.add(name);
  const ed = editingId();
  if (ed && doc.byId[ed]) pin.add(doc.byId[ed].page);
  return pin;
}

/** Replace a page in the working set from a fresh DTO (e.g. resolving a conflict
 *  with the disk version, or a watcher reload). Updates the main view and any
 *  satellite that shows it, since they share `byId`. */
export async function reloadPage(
  dto: PageDto,
  options: Pick<EditorInstallOptions, "isRequestLive" | "beforeInstall"> = {},
): Promise<InstanceRefusal | null> {
  return ensurePageLoaded(dto, { ...options, bypassReplacementGate: true });
}

/** Install the managed actor's current DTO after an explicit discard choice.
 *
 * Managed conflicts have revision authority of their own and deliberately do
 * not mint Direct Files editor activations or observation epochs. Keep this
 * narrow installer separate from the Direct read → activate → present protocol.
 */
export function installManagedConflictVersion(dto: PageDto): void {
  upsertPage(dto);
  evictIfNeeded();
}

/** Apply a watcher-driven disk reload only if it is STILL safe at this instant.
 *
 *  `reloadDisposition` is correct, but the watcher sites read its verdict and
 *  then `await backend().getPage(...)` before acting — tens to hundreds of ms on
 *  a large graph, and a Syncthing burst fires many of these concurrently. If the
 *  user clicks into a block and types inside that window, `commit()` writes into
 *  the store synchronously; the resolved IPC then replaces the page, dropping the
 *  typed text AND its undo history, with no conflict raised and nothing written
 *  to disk. (Direct Files data-safety audit, 2026-08-09, finding 5.)
 *
 *  `upsertUnlessDirty` already re-checks at the moment of the upsert; only these
 *  watcher sites skipped it. Re-checking here rather than at each call site means
 *  a fifth site cannot reintroduce the hole. `reloadPage` itself stays a
 *  deliberate clobber — "use disk version" is an explicit user decision.
 *
 *  Returns false when the reload was declined. */
export async function reloadPageIfStillSafe(
  name: string,
  dto: PageDto,
  expectedGraphBinding = graphBinding(),
): Promise<boolean> {
  // The full gate, not `reloadDisposition` alone: component-local uncommitted
  // input (the title-rename draft, IME composition) is invisible to every store
  // predicate, and this path deliberately replaces the working instance.
  if (!mayReplaceInstance(name)) return false;
  if (reloadDisposition(name) !== "reload") return false;
  return (await ensurePageLoaded(dto, { expectedGraphBinding })) === null;
}

const pendingHlsRefreshes = new Map<string, () => void>();

function retryHlsRefreshWhenReplaceable(name: string, binding: number): void {
  if (pendingHlsRefreshes.has(name)) return;
  const stop = onPageBecameReplaceable(name, () => {
    stop();
    pendingHlsRefreshes.delete(name);
    if (binding !== graphBinding()) return;
    void reloadHlsIfLoaded(name);
  });
  pendingHlsRefreshes.set(name, stop);
}

function clearPendingHlsRefreshes(): void {
  for (const stop of pendingHlsRefreshes.values()) stop();
  pendingHlsRefreshes.clear();
}

/** After a PDF highlight write changed an `hls__` page on disk, refresh its
 *  loaded copy (main view or sidebar) so its content AND save baseline (baseRev)
 *  track disk — otherwise a later editor save would conflict against the highlight
 *  write. Skips a page with unsaved edits / an open conflict: the caller flushes
 *  those FIRST so they're on disk and merged in, rather than clobbered here. */
export async function reloadHlsIfLoaded(name: string): Promise<boolean> {
  if (!pageByName(name)) return false;
  // The FULL gate, and re-evaluated after the await. The old dirty-or-conflicted
  // check missed uncommitted input the store cannot see — an IME composition on
  // the notes page was reproduced being destroyed here while the store was clean
  // — and checking only before the await let the page become dirty during it,
  // since `reloadPage` is a deliberate clobber. Declining is safe: the next
  // highlight write re-drives this. (GH #254 increment 3.)
  const binding = graphBinding();
  if (!mayReplaceInstance(name)) {
    retryHlsRefreshWhenReplaceable(name, binding);
    return false;
  }
  const dto = await backend().getPage(name, "page");
  if (!dto || binding !== graphBinding()) return false;
  const refusal = await ensurePageLoaded(dto, { expectedGraphBinding: binding });
  if (refusal) {
    if (refusal.reason === "unsaved-changes") retryHlsRefreshWhenReplaceable(name, binding);
    return false;
  }
  return true;
}
function evictIfNeeded() {
  if (doc.pages.length <= WORKING_SET_CAP) return;
  const pin = pinnedPages();
  const evicted: { name: string; path?: string }[] = [];
  setDoc(
    produce((s) => {
      // Oldest first (insertion order); stop once at the cap or only pinned left.
      for (let i = 0; i < s.pages.length && s.pages.length > WORKING_SET_CAP; ) {
        const name = s.pages[i].name;
        if (pin.has(name)) {
          i++;
          continue;
        }
        // Capture the path BEFORE the page leaves the working set. A retirement
        // that has to look the page up afterwards finds nothing and silently
        // retires nothing, leaking the native activation — which the next editor
        // of that path would then inherit under same-path Reuse.
        evicted.push({ name, path: s.pages[i].path });
        purgePageNodes(s, name);
        s.pages.splice(i, 1);
      }
    })
  );
  for (const { name, path } of evicted) {
    retireEditorFor(name, path);
    retirePageInstance(name);
  }
  invalidateAllMatrixDimensions();
}

/** Clear the entire working set. Used for test isolation and when switching
 *  graphs; normal navigation is additive (keeps satellite pages alive). Also
 *  cancels pending saves and clears dirty flags so nothing from the old graph
 *  can be written after a switch. */
export function resetStore() {
  // Pure node tests historically exercise the synchronous Direct store without
  // opening a graph. Seed that authority once; managed-boundary tests explicitly
  // rebind to the authority they exercise after reset.
  if (import.meta.env.MODE === "test" && managedStorageRuntime.snapshot().bindingGeneration === null) {
    managedStorageRuntime.bind(1, { binding_generation: 1, authority: "direct" });
  }
  // Every identity belongs to the graph being left. The core drops its own
  // registry with the Graph, so clearing locally is sufficient and avoids a
  // storm of per-page retirements against a graph that is going away.
  clearAllEditorActivations();
  clearAllEditorLeases();
  clearPendingHlsRefreshes();
  clearPendingBlockRefStamps();
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
  // Linked/Unlinked References expand state is keyed by page identity, and every
  // page identity is retired with the old graph (GH #272).
  resetReferenceSectionState();
  for (const name of pageInstanceGenerations.keys()) retirePageInstance(name);
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
/** Install `dto` unless the loaded page holds unsaved work. Reports whether it
 *  actually installed, so publication can follow installation rather than assume
 *  it. (GH #254 increment 3.) */
async function upsertUnlessDirty(dto: PageDto, expectedGraphBinding: number): Promise<boolean> {
  return (await ensurePageLoaded(dto, { expectedGraphBinding })) === null;
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
 *  (Both `upsertUnlessDirty` and `reloadHlsIfLoaded` now compose this with the
 *  editor-lease set via `mayReplaceInstance`; the old deliberately-weaker
 *  dirty-only guard was what GH #304 cost.) */
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

/**
 * Component-local editors that currently hold uncommitted input.
 *
 * `reloadDisposition` only sees state that lives in the store, and not all
 * uncommitted user input does. The page-title rename keeps its draft in local
 * signals and an `<input>`, so replacing the page unmounts the input and the typed
 * title is gone with nothing ever having been dirty. IME composition has the same
 * shape. Enumerating those cases kept losing — each round of review found another
 * one — so a component that holds uncommitted input DECLARES itself instead.
 *
 * The registry is keyed by page, then by a unique per-component handle, because
 * one page can be mounted on several surfaces: cancelling transaction A must not
 * clear transaction B's. (GH #254 increment 3.)
 */
const editorLeases = new Map<string, Set<symbol>>();

/**
 * Take a lease for uncommitted input on `pageName`. Returns its release, which is
 * idempotent and MUST be wired to the component lifecycle (`onCleanup`), not only
 * to commit and cancel: disposing a mounted page removes the title section without
 * running either, and a literal registration would then outlive its component and
 * its draft and refuse every later replacement forever.
 */
export function takeEditorLease(pageName: string): () => void {
  const handle = Symbol("editor-lease");
  editorTransactionGenerations.set(pageName, ++editorTransactionClock);
  let leases = editorLeases.get(pageName);
  if (!leases) {
    leases = new Set();
    editorLeases.set(pageName, leases);
  }
  leases.add(handle);
  let released = false;
  return () => {
    if (released) return;
    released = true;
    const live = editorLeases.get(pageName);
    if (!live) return;
    live.delete(handle);
    if (live.size === 0) {
      editorLeases.delete(pageName);
      notifyPageBecameReplaceable(pageName);
    }
  };
}

/**
 * Watchers waiting for a specific page to become replaceable.
 *
 * Keyed BY PAGE, so liveness does not depend on my enumeration of emission sites
 * being complete — which is what kept failing. Explicit announcements make the
 * common transitions prompt; `sweepReplaceable()` is the net that re-checks every
 * watched page, so a route nobody thought to instrument delays a resume rather
 * than stranding it forever. (GH #254 increment 3.)
 */
const replaceableWatchers = new Map<string, Set<(pageName: string) => void>>();

export function onPageBecameReplaceable(
  pageName: string,
  listener: (pageName: string) => void,
): () => void {
  let set = replaceableWatchers.get(pageName);
  if (!set) {
    set = new Set();
    replaceableWatchers.set(pageName, set);
  }
  set.add(listener);
  let stopped = false;
  return () => {
    if (stopped) return;
    stopped = true;
    const live = replaceableWatchers.get(pageName);
    if (!live) return;
    live.delete(listener);
    if (live.size === 0) replaceableWatchers.delete(pageName);
  };
}

/** Announce `pageName` if it is genuinely replaceable now. */
export function notifyPageBecameReplaceable(pageName: string): void {
  const set = replaceableWatchers.get(pageName);
  if (!set || set.size === 0) return;
  if (!mayReplaceInstance(pageName)) return;
  for (const listener of [...set]) listener(pageName);
}

/** Re-check every watched page. The safety net behind the explicit sites. */
export function sweepReplaceable(): void {
  if (replaceableWatchers.size === 0) return;
  for (const name of [...replaceableWatchers.keys()]) notifyPageBecameReplaceable(name);
}

export function clearReplaceableWatchers(): void {
  replaceableWatchers.clear();
}

/** Does any component hold uncommitted input for this page? */
export function hasEditorLease(pageName: string): boolean {
  return (editorLeases.get(pageName)?.size ?? 0) > 0;
}

/** Drop every lease — graph reset and teardown. */
export function clearAllEditorLeases(): void {
  editorLeases.clear();
  editorTransactionGenerations.clear();
}

/**
 * May this page's loaded instance be REPLACED right now?
 *
 * The composed gate: the store's own disposition plus the component-local leases
 * it cannot see. Both halves are required, and both must be re-evaluated
 * synchronously at the final replacement boundary — every caller awaits a backend
 * read first, and the incumbent can become dirty, start saving, or begin an
 * uncommitted rename during that await. (GH #254 increment 3.)
 */
export function mayReplaceInstance(name: string): boolean {
  return reloadDisposition(name) === "reload" && !hasEditorLease(name);
}

/** Why a replacement was refused, for the surface that asked for it. */
export type InstanceRefusal = {
  reason: "unsaved-changes" | "stale-instance" | "activation-failed";
  /** The page holding the unsaved work — what the surface tells the user. */
  page: string;
};

/** Load a single page and make it the main view. */
export function loadSingle(dto: PageDto, opts: { endEdit?: boolean } = {}) {
  // Legacy synchronous store seeding used by isolated/test surfaces. Production
  // routed editors use `loadRoutedPage`, and feed editors use `loadFeed`; both go
  // through activation before publication. A page seeded here still acquires its
  // activation at the save boundary before any write.
  if (pageByName(dto.name) && !mayReplaceInstance(dto.name)) return false;
  upsertPage(dto);
  setDoc("feed", [dto.name]);
  setDoc("loaded", true);
  if (opts.endEdit !== false) endEdit("page-navigation");
  evictIfNeeded();
  return true;
}

/** Load the journals feed as the main view. */
export async function loadFeed(
  dtos: PageDto[],
  opts: { endEdit?: boolean; expectedGraphBinding?: number } = {},
) {
  // Publication FOLLOWS installation. When the DTO is declined the name used to
  // be published into the feed anyway, so the feed rendered a dirty path-pinned
  // stray as though it were the requested canonical journal — no refusal, no
  // path warning, and an edit saved to the wrong file. A page already present
  // under that name stays published; one that never installed does not.
  // (GH #254 increment 3.)
  // Publication follows INSTALLATION. An earlier draft fell back to
  // `|| pageByName(d.name)`, which reintroduced the exact defect: a dirty
  // path-pinned stray already occupying the name made the declined canonical DTO
  // publish anyway, so the feed rendered the stray as though it were the
  // requested journal.
  const binding = opts.expectedGraphBinding ?? graphBinding();
  const installed: string[] = [];
  for (const dto of dtos) {
    if (await upsertUnlessDirty(dto, binding)) installed.push(dto.name);
  }
  if (binding !== graphBinding()) return;
  setDoc("feed", installed);
  setDoc("loaded", true);
  if (opts.endEdit !== false) endEdit("page-navigation");
  evictIfNeeded();
}

/** Append more pages to the journals feed (infinite scroll). */
export async function appendFeed(
  dtos: PageDto[],
  expectedGraphBinding = graphBinding(),
) {
  const binding = expectedGraphBinding;
  for (const d of dtos) {
    if (doc.feed.includes(d.name)) continue;
    // Publication follows installation — see `loadFeed`.
    if (!(await upsertUnlessDirty(d, binding))) continue;
    if (binding !== graphBinding()) return;
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
export async function restoreTodayJournalInFeed(): Promise<boolean> {
  const title = journalTitle(new Date());
  if (doc.feed.includes(title)) return true;
  const binding = graphBinding();
  if (!(await upsertUnlessDirty(emptyPage(title, "journal"), binding))) return false;
  if (binding !== graphBinding()) return false;
  setDoc("feed", [title, ...doc.feed]);
  return true;
}

function toDtoFrom(nodes: Readonly<Record<string, Node>>, id: string): BlockDto {
  const n = nodes[id];
  // Trim a block's trailing space only here, at the disk-write boundary — OG
  // keeps the space while you edit and trims on save. (The live editor buffer
  // keeps it so backspacing to a trailing space doesn't eat the space out from
  // under the caret.) `trimBlockTrailingSpace` is idempotent and only touches
  // whitespace at the very end of the block, so a block with nothing to trim
  // serializes byte-identically — no churn, no property reordering.
  return {
    id: n.id,
    raw: trimBlockTrailingSpace(n.raw),
    collapsed: n.collapsed,
    children: n.children.map((child) => toDtoFrom(nodes, child)),
  };
}

function toDto(id: string): BlockDto {
  return toDtoFrom(doc.byId, id);
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

function projectPageDto(
  p: FeedPage | undefined,
  nodes: Readonly<Record<string, Node>>,
  reportInvalidHeader: boolean,
): PageDto | null {
  if (!p) return null;
  let rootIds = p.roots;
  let preBlock = p.preBlock;
  const first = nodes[rootIds[0]];
  if (first?.originatedFromPageHeader) {
    // Enter temporarily leaves one or more trailing newlines in the live
    // page-header editor. Tolerate only that authoring artifact at the disk
    // firewall; keep the strict shared display predicate and live raw intact.
    const canonicalRaw = first.raw.replace(/\n+$/, "");
    if (first.children.length > 0 || (first.raw !== "" && !isPageHeaderPropertiesOnly(canonicalRaw))) {
      if (reportInvalidHeader) {
        pushToast("Page-header properties must contain only valid key:: value lines before they can be saved.", "error");
      }
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
  let blocks = rootIds.map((id) => toDtoFrom(nodes, id));
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
    // Which live editor is issuing this save. Read from the registry rather than
    // carried on the page, so no clone or history snapshot can claim it.
    // (GH #254 increment 3.)
    activation: editorActivations.get(p.name),
    // Pin the save to the exact file this page came from (#21). For an editor
    // activated with no file yet, this is the prospective target it is live for —
    // without it the DTO goes out unpinned and the core cannot recognise its own
    // absent editor when the target drifts underneath it.
    path: p.path || prospectiveTargets.get(p.name) || "",
    guide: p.guide,
    read_only: p.readOnly,
  };
}

export function pageToDto(pageName: string): PageDto | null {
  return projectPageDto(doc.pages.find((x) => x.name === pageName), doc.byId, true);
}

// ---------------------------------------------------------------------------
// Detached one-page mutation plans
// ---------------------------------------------------------------------------

export type PageMutationEffect =
  | { kind: "create"; node: Readonly<Omit<Node, "children">> & { readonly children: readonly string[] }; parent: string; at: number }
  | { kind: "delete"; id: string }
  | { kind: "raw"; id: string; raw: string }
  | { kind: "property"; id: string; key: string; value: string | null; raw: string }
  | { kind: "parent"; id: string; parent: string }
  | { kind: "order"; parent: string; children: readonly string[] };

export type PageMutationDraftNode = Readonly<Omit<Node, "children">> & {
  readonly children: readonly string[];
};

export type PageMutationDraftPage = Readonly<Omit<FeedPage, "roots">> & {
  readonly roots: readonly string[];
};

export interface PageMutationDraft {
  readonly page: PageMutationDraftPage;
  node(id: string): PageMutationDraftNode | undefined;
  createChild(parentId: string, at: number, raw?: string): string | null;
  insertOutlineChildren(parentId: string, outlines: readonly OutlineNode[]): string | null;
  deleteSubtree(id: string): boolean;
  setRaw(id: string, raw: string): boolean;
  setProperty(id: string, key: string, value: string | null): boolean;
  replaceChildren(parentId: string, children: readonly string[]): boolean;
}

const pageMutationPlanSeal = Symbol("page-mutation-plan");

export interface PageMutationPlan<T> {
  readonly [pageMutationPlanSeal]: true;
  readonly pageName: string;
  readonly tag: string;
  readonly value: T;
  readonly candidate: PageDto;
  readonly effects: readonly PageMutationEffect[];
}

/** Optional UI ownership attached to a plan. The token itself is immutable and
 * `isCurrent` must cover the exact selection and mounted Sheet surface that
 * issued the command. Store code checks it before native admission, before
 * publication, and once more before post-commit UI effects. */
export interface PageMutationAuthority<T> {
  readonly token: Readonly<Record<string, unknown>>;
  isCurrent(value: T): boolean;
}

interface InternalPageMutationPlan<T> extends PageMutationPlan<T> {
  graphRoot: string;
  graphEpoch: number;
  graphBinding: number;
  pageGeneration: number;
  editGeneration: number;
  editorTransactionGeneration: number;
  saveBaseline: string | null;
  bindingGeneration: number;
  authority: "direct" | "managed_writable" | "managed_unavailable" | "missing";
  pendingKey: string;
  captured: Readonly<Record<string, PageMutationDraftNode>>;
  capturedPage: PageMutationDraftPage;
  uiAuthority?: PageMutationAuthority<T>;
}

export type PageMutationDispatch<T> =
  | { kind: "applied"; value: T }
  | { kind: "pending"; value: T; settled: Promise<boolean> }
  | { kind: "refused"; claimed: boolean };

const pendingPageMutations = new Map<string, object>();
const startedPageMutationPlans = new WeakSet<object>();
const appliedPageMutationPlans = new WeakSet<object>();
let pageMutationEffectFailureForTest = false;

function immutableClone<T>(value: T): T {
  if (value === null || typeof value !== "object") return value;
  if (Array.isArray(value)) {
    return Object.freeze(value.map((item) => immutableClone(item))) as T;
  }
  const clone: Record<string, unknown> = {};
  for (const [key, nested] of Object.entries(value as Record<string, unknown>)) {
    clone[key] = immutableClone(nested);
  }
  return Object.freeze(clone) as T;
}

function immutableNode(node: Node): PageMutationDraftNode {
  return immutableClone(cloneNode(node)) as PageMutationDraftNode;
}

function immutablePage(page: FeedPage): PageMutationDraftPage {
  return immutableClone(clonePages([page])[0]) as PageMutationDraftPage;
}

function clonePageTree(page: FeedPage): Record<string, Node> | null {
  const nodes: Record<string, Node> = {};
  const visit = (id: string): boolean => {
    if (nodes[id]) return true;
    const current = doc.byId[id];
    if (!current || current.page !== page.name) return false;
    nodes[id] = cloneNode(unwrap(current));
    return current.children.every(visit);
  };
  return page.roots.every(visit) ? nodes : null;
}

/** Build a pure detached draft. The callback sees only the captured page tree;
 * it cannot publish, dirty, save, select, or enter an editor. */
export function createPageMutationPlan<T>(
  pageName: string,
  tag: string,
  build: (draft: PageMutationDraft) => T | null,
  uiAuthority?: PageMutationAuthority<T>,
): PageMutationPlan<T> | null {
  const livePage = pageByName(pageName);
  if (!livePage || !pageWritable(pageName) || graphTransitioning()) return null;
  const generation = pageInstanceGeneration(pageName);
  if (generation === null) return null;
  const draftPage = clonePages([unwrap(livePage)])[0];
  const draftNodes = clonePageTree(livePage);
  if (!draftNodes) return null;
  const capturedMutable: Record<string, Node> = {};
  for (const [id, node] of Object.entries(draftNodes)) capturedMutable[id] = cloneNode(node);
  const effects: PageMutationEffect[] = [];
  let active = true;

  const remove = (id: string): boolean => {
    const node = draftNodes[id];
    if (!node) return false;
    const siblings = node.parent === null
      ? draftPage.roots
      : draftNodes[node.parent]?.children;
    if (!siblings) return false;
    const at = siblings.indexOf(id);
    if (at < 0) return false;
    siblings.splice(at, 1);
    const descend = (childId: string) => {
      const child = draftNodes[childId];
      if (!child) return;
      for (const grandchild of [...child.children]) descend(grandchild);
      delete draftNodes[childId];
    };
    descend(id);
    effects.push({ kind: "delete", id });
    return true;
  };

  const draft: PageMutationDraft = {
    page: immutablePage(draftPage),
    node: (id) => active && draftNodes[id] ? immutableNode(draftNodes[id]) : undefined,
    createChild(parentId, at, raw = "") {
      if (!active) return null;
      const parent = draftNodes[parentId];
      if (!parent || at < 0 || at > parent.children.length) return null;
      const id = freshId();
      const node: Node = {
        id,
        raw,
        collapsed: false,
        parent: parentId,
        page: pageName,
        children: [],
      };
      draftNodes[id] = node;
      parent.children.splice(at, 0, id);
      effects.push({ kind: "create", node: immutableNode(node), parent: parentId, at });
      return id;
    },
    insertOutlineChildren(parentId, outlines) {
      if (!active) return null;
      const parent = draftNodes[parentId];
      if (!parent || !outlines.length) return null;
      const format = draftPage.format;
      let last: string | null = null;
      const create = (outline: OutlineNode, parent: string): string => {
        const id = freshId();
        const raw = rawWithInheritedOrderListType(outline.raw, format, parentId);
        draftNodes[id] = { id, raw, collapsed: false, parent, page: pageName, children: [] };
        const at = draftNodes[parent]?.children.length ?? 0;
        draftNodes[parent]?.children.push(id);
        effects.push({ kind: "create", node: immutableNode(draftNodes[id]), parent, at });
        const children = outline.children.map((child) => create(child, id));
        draftNodes[id].children = children;
        return id;
      };
      const created = outlines.map((outline) => create(outline, parentId));
      last = created[created.length - 1] ?? null;
      return last;
    },
    deleteSubtree(id) {
      return active && remove(id);
    },
    setRaw(id, raw) {
      if (!active) return false;
      const node = draftNodes[id];
      if (!node) return false;
      node.raw = raw;
      effects.push({ kind: "raw", id, raw });
      return true;
    },
    setProperty(id, key, value) {
      if (!active) return false;
      const node = draftNodes[id];
      if (!node) return false;
      node.raw = draftPage.format === "org"
        ? orgRawWithProperty(node.raw, key, value)
        : markdownRawWithProperty(node.raw, key, value);
      effects.push({ kind: "property", id, key, value, raw: node.raw });
      return true;
    },
    replaceChildren(parentId, children) {
      if (!active) return false;
      const parent = draftNodes[parentId];
      if (!parent || children.some((id) => !draftNodes[id] || draftNodes[id].page !== pageName)) return false;
      parent.children = [...children];
      for (const id of children) {
        if (draftNodes[id].parent !== parentId) {
          draftNodes[id].parent = parentId;
          effects.push({ kind: "parent", id, parent: parentId });
        }
      }
      effects.push({ kind: "order", parent: parentId, children: [...children] });
      return true;
    },
  };

  let builtValue: T | null;
  try {
    builtValue = build(draft);
  } finally {
    active = false;
  }
  if (builtValue === null) return null;
  const frozenEffects = immutableClone(effects) as readonly PageMutationEffect[];
  const capturedPage = immutablePage(draftPage);
  const captured = immutableClone(Object.fromEntries(
    Object.entries(capturedMutable).map(([id, node]) => [id, immutableNode(node)]),
  )) as Readonly<Record<string, PageMutationDraftNode>>;
  const replay = replayPageMutationEffects(capturedPage, captured, frozenEffects);
  if (!replay) return null;
  const candidate = projectPageDto(replay.page, replay.nodes, false);
  if (!candidate) return null;
  const value = immutableClone(builtValue);
  const admission = managedStorageRuntime.snapshot().applicationPageAdmission;
  const graphRoot = graphMeta()?.root ?? "";
  const epoch = graphEpoch();
  const binding = graphBinding();
  const plan: InternalPageMutationPlan<T> = {
    [pageMutationPlanSeal]: true,
    pageName,
    tag,
    value,
    candidate: immutableClone(candidate),
    effects: frozenEffects,
    graphRoot,
    graphEpoch: epoch,
    graphBinding: binding,
    pageGeneration: generation,
    editGeneration: editGeneration(pageName),
    editorTransactionGeneration: editorTransactionGeneration(pageName),
    saveBaseline: saveBaselineFor(pageName),
    bindingGeneration: admission?.binding_generation ?? -1,
    authority: admission?.authority ?? "missing",
    pendingKey: `${graphRoot}\0${epoch}\0${binding}\0${generation}\0${pageName}`,
    captured,
    capturedPage,
    uiAuthority: uiAuthority
      ? Object.freeze({ token: immutableClone(uiAuthority.token), isCurrent: uiAuthority.isCurrent })
      : undefined,
  };
  return Object.freeze(plan);
}

interface ReplayedPageMutation {
  page: FeedPage;
  nodes: Record<string, Node>;
}

function mutablePage(page: PageMutationDraftPage | FeedPage): FeedPage {
  return clonePages([page as FeedPage])[0];
}

function mutableNodes(nodes: Readonly<Record<string, PageMutationDraftNode>>): Record<string, Node> {
  return Object.fromEntries(Object.entries(nodes).map(([id, node]) => [id, cloneNode(node as Node)]));
}

function applyEffectsToMutablePage(
  page: FeedPage,
  nodes: Record<string, Node>,
  effects: readonly PageMutationEffect[],
): boolean {
  if (pageMutationEffectFailureForTest) return false;
  for (const effect of effects) {
    if (effect.kind === "create") {
      const parent = nodes[effect.parent];
      if (nodes[effect.node.id] || !parent || effect.at < 0 || effect.at > parent.children.length) return false;
      const node = cloneNode(effect.node as Node);
      if (node.page !== page.name || node.parent !== effect.parent || node.children.length) return false;
      nodes[node.id] = node;
      parent.children.splice(effect.at, 0, node.id);
      continue;
    }
    if (effect.kind === "delete") {
      const node = nodes[effect.id];
      if (!node) return false;
      const siblings = node.parent === null ? page.roots : nodes[node.parent]?.children;
      if (!siblings) return false;
      const at = siblings.indexOf(effect.id);
      if (at < 0) return false;
      siblings.splice(at, 1);
      const remove = (id: string): boolean => {
        const current = nodes[id];
        if (!current) return false;
        for (const child of [...current.children]) if (!remove(child)) return false;
        delete nodes[id];
        return true;
      };
      if (!remove(effect.id)) return false;
      continue;
    }
    if (effect.kind === "raw") {
      const node = nodes[effect.id];
      if (!node) return false;
      node.raw = effect.raw;
      continue;
    }
    if (effect.kind === "property") {
      const node = nodes[effect.id];
      if (!node) return false;
      const raw = page.format === "org"
        ? orgRawWithProperty(node.raw, effect.key, effect.value)
        : markdownRawWithProperty(node.raw, effect.key, effect.value);
      if (raw !== effect.raw) return false;
      node.raw = raw;
      continue;
    }
    if (effect.kind === "parent") {
      const node = nodes[effect.id];
      if (!node || !nodes[effect.parent]) return false;
      node.parent = effect.parent;
      continue;
    }
    const parent = nodes[effect.parent];
    if (!parent || new Set(effect.children).size !== effect.children.length) return false;
    if (effect.children.some((id) => !nodes[id] || nodes[id].page !== page.name)) return false;
    parent.children = [...effect.children];
  }

  const seen = new Set<string>();
  const visit = (id: string, parent: string | null): boolean => {
    const node = nodes[id];
    if (!node || seen.has(id) || node.page !== page.name || node.parent !== parent) return false;
    seen.add(id);
    return node.children.every((child) => visit(child, id));
  };
  if (new Set(page.roots).size !== page.roots.length || !page.roots.every((id) => visit(id, null))) return false;
  return Object.values(nodes).every((node) => node.page !== page.name || seen.has(node.id));
}

function replayPageMutationEffects(
  page: PageMutationDraftPage,
  captured: Readonly<Record<string, PageMutationDraftNode>>,
  effects: readonly PageMutationEffect[],
): ReplayedPageMutation | null {
  const replay = { page: mutablePage(page), nodes: mutableNodes(captured) };
  return applyEffectsToMutablePage(replay.page, replay.nodes, effects) ? replay : null;
}

function replayMatchesCandidate(plan: InternalPageMutationPlan<unknown>): boolean {
  const replay = replayPageMutationEffects(plan.capturedPage, plan.captured, plan.effects);
  const projected = replay && projectPageDto(replay.page, replay.nodes, false);
  return !!projected && JSON.stringify(projected) === JSON.stringify(plan.candidate);
}

/** Test-only proof that the finalized authority is a deeply immutable replay
 * program. Returns false in production builds. */
export function __pageMutationPlanDeeplyFrozenForTest(plan: PageMutationPlan<unknown>): boolean {
  if (import.meta.env.MODE !== "test") return false;
  const check = (value: unknown): boolean => {
    if (value === null || typeof value !== "object") return true;
    if (!Object.isFrozen(value)) return false;
    return Object.values(value).every(check);
  };
  return check(plan.effects) && check(plan.candidate) && check(plan.value);
}

function pageMutationPlanCurrent(
  plan: InternalPageMutationPlan<unknown>,
  checkUiAuthority = true,
): boolean {
  const admission = managedStorageRuntime.snapshot().applicationPageAdmission;
  if (
    graphTransitioning()
    || (graphMeta()?.root ?? "") !== plan.graphRoot
    || graphEpoch() !== plan.graphEpoch
    || graphBinding() !== plan.graphBinding
    || pageInstanceGeneration(plan.pageName) !== plan.pageGeneration
    || editGeneration(plan.pageName) !== plan.editGeneration
    || editorTransactionGeneration(plan.pageName) !== plan.editorTransactionGeneration
    || saveBaselineFor(plan.pageName) !== plan.saveBaseline
    || admission?.binding_generation !== plan.bindingGeneration
    || admission?.authority !== plan.authority
  ) return false;
  const page = pageByName(plan.pageName);
  if (!page || JSON.stringify(mutablePage(page)) !== JSON.stringify(mutablePage(plan.capturedPage))) return false;
  const liveIds = Object.values(doc.byId).filter((node) => node.page === plan.pageName).map((node) => node.id).sort();
  const capturedIds = Object.keys(plan.captured).sort();
  if (liveIds.length !== capturedIds.length || liveIds.some((id, index) => id !== capturedIds[index])) return false;
  for (const [id, expected] of Object.entries(plan.captured)) {
    const current = doc.byId[id];
    if (!current
      || current.page !== expected.page
      || current.parent !== expected.parent
      || current.raw !== expected.raw
      || current.collapsed !== expected.collapsed
      || current.children.length !== expected.children.length
      || current.children.some((child, index) => child !== expected.children[index])) return false;
  }
  return !checkUiAuthority || !plan.uiAuthority || plan.uiAuthority.isCurrent(plan.value);
}

function applyPageMutationPlanNow<T>(
  plan: InternalPageMutationPlan<T>,
  checkUiAuthority: boolean,
): boolean {
  if (appliedPageMutationPlans.has(plan)
    || !pageMutationPlanCurrent(plan, checkUiAuthority)
    || !replayMatchesCandidate(plan)) return false;
  const pageIndex = doc.pages.findIndex((page) => page.name === plan.pageName);
  if (pageIndex < 0) return false;
  const livePage = mutablePage(doc.pages[pageIndex]);
  const liveNodes = Object.fromEntries(
    Object.values(doc.byId)
      .filter((node) => node.page === plan.pageName)
      .map((node) => [node.id, cloneNode(unwrap(node))]),
  );
  if (!applyEffectsToMutablePage(livePage, liveNodes, plan.effects)) return false;
  const projected = projectPageDto(livePage, liveNodes, false);
  if (!projected || JSON.stringify(projected) !== JSON.stringify(plan.candidate)) return false;
  pushUndo(plan.tag, [plan.pageName]);
  setDoc(produce((state) => {
    if (!applyEffectsToMutablePage(state.pages[pageIndex], state.byId, plan.effects)) {
      throw new Error("validated page-mutation effects failed during atomic replay");
    }
  }));
  appliedPageMutationPlans.add(plan);
  markDirty(plan.pageName);
  return true;
}

const managedPageMutationBusyToast = "This Sheet is still checking its previous change. Nothing was changed.";
const managedPageMutationRefusedToast = "Tine-managed storage could not accept this Sheet change. Nothing was changed.";

/** Apply Direct plans synchronously. Managed plans claim one per-page slot,
 * await exact native preparation, recheck every captured authority and relation,
 * then publish once. */
export function applyPageMutationPlan<T>(
  publicPlan: PageMutationPlan<T>,
  afterApply?: (value: T) => void,
): PageMutationDispatch<T> {
  const plan = publicPlan as InternalPageMutationPlan<T>;
  if (!plan[pageMutationPlanSeal] || startedPageMutationPlans.has(plan)) {
    return { kind: "refused", claimed: plan.authority !== "direct" };
  }
  startedPageMutationPlans.add(plan);
  if (plan.authority === "direct") {
    if (!applyPageMutationPlanNow(plan, false)) return { kind: "refused", claimed: false };
    afterApply?.(plan.value);
    return { kind: "applied", value: plan.value };
  }
  if (plan.authority !== "managed_writable") {
    pushToast(managedPageMutationRefusedToast, "error");
    return { kind: "refused", claimed: true };
  }
  if (!pageMutationPlanCurrent(plan) || !replayMatchesCandidate(plan)) {
    return { kind: "refused", claimed: true };
  }
  if (pendingPageMutations.has(plan.pendingKey)) {
    pushToast(managedPageMutationBusyToast, "error");
    return { kind: "refused", claimed: true };
  }
  pendingPageMutations.set(plan.pendingKey, plan);
  const settled = backend()
    .preflightManagedPageMutation(plan.candidate, plan.saveBaseline, plan.bindingGeneration)
    .then((acceptance) => {
      const accepted = acceptance.status === "accepted"
        && acceptance.binding_generation === plan.bindingGeneration
        && acceptance.page_name === plan.candidate.name
        && acceptance.page_path === plan.candidate.path
        && acceptance.base_revision === plan.saveBaseline;
      if (!accepted || !applyPageMutationPlanNow(plan, true)) {
        pushToast(managedPageMutationRefusedToast, "error");
        return false;
      }
      if (!plan.uiAuthority || plan.uiAuthority.isCurrent(plan.value)) afterApply?.(plan.value);
      return true;
    })
    .catch(() => {
      pushToast(managedPageMutationRefusedToast, "error");
      return false;
    })
    .finally(() => {
      if (pendingPageMutations.get(plan.pendingKey) === plan) pendingPageMutations.delete(plan.pendingKey);
    });
  return { kind: "pending", value: plan.value, settled };
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

export interface ManagedBulkInsertionPlan {
  insertedDescendants: number;
  removedOrReusedDescendants: number;
  insertionRootDepth: number;
  maximumInputRelativeDepth: number;
  insertedRawTextUtf8Bytes: number;
}

interface ManagedBulkAdmissionLimits {
  applicationSavePageBlocks: number;
  applicationPageRequestTextBytes: number;
  applicationPageMaxDepth: number;
}

type BulkOutlineLike = { raw: string; children: readonly BulkOutlineLike[] };

/** Count only input already materialized by a selected caller. The limits cap
 * this pure work: no PageDto clone, actor call, block-id allocation, or scan of
 * any other page is involved. */
export function managedBulkOutlinePlan(
  nodes: readonly BulkOutlineLike[],
  insertionRootDepth: number,
  removedOrReusedDescendants: number,
  limits: ManagedBulkAdmissionLimits,
): ManagedBulkInsertionPlan {
  const blockCap = limits.applicationSavePageBlocks + 1;
  const textCap = limits.applicationPageRequestTextBytes + 1;
  let insertedDescendants = 0;
  let maximumInputRelativeDepth = 0;
  let insertedRawTextUtf8Bytes = 0;
  const stack = nodes.map((node) => ({ node, relativeDepth: 1 }));
  while (stack.length) {
    const { node, relativeDepth } = stack.pop()!;
    insertedDescendants = Math.min(blockCap, insertedDescendants + 1);
    maximumInputRelativeDepth = Math.max(maximumInputRelativeDepth, relativeDepth);
    insertedRawTextUtf8Bytes = Math.min(
      textCap,
      insertedRawTextUtf8Bytes + new TextEncoder().encode(node.raw).byteLength,
    );
    if (insertedDescendants === blockCap || insertedRawTextUtf8Bytes === textCap) break;
    for (let index = node.children.length - 1; index >= 0; index--) {
      stack.push({ node: node.children[index], relativeDepth: relativeDepth + 1 });
    }
  }
  return {
    insertedDescendants,
    removedOrReusedDescendants,
    insertionRootDepth,
    maximumInputRelativeDepth,
    insertedRawTextUtf8Bytes,
  };
}

const bulkInsertionAdmissionSeal = Symbol("bulk-insertion-admission");

export interface BulkInsertionAdmission {
  readonly [bulkInsertionAdmissionSeal]: true;
}

interface InternalBulkInsertionAdmission extends BulkInsertionAdmission {
  consumed: boolean;
  targetId: string | null;
  targetNode: Node | null;
  targetPage: string;
  targetGeneration: number;
  graphEpoch: number;
  graphRoot: string;
  bindingGeneration: number;
  authority: "managed_writable";
  plan: ManagedBulkInsertionPlan;
}

export type ManagedBulkInsertionPreflight =
  | { kind: "direct" }
  | { kind: "admitted"; token: BulkInsertionAdmission }
  | { kind: "refused"; toast: string };

const managedBulkOverflowToast = (limit: number): string =>
  `Can't insert: this page would exceed Tine-managed storage's ${limit}-block or request-size limit. Nothing was changed.`;

const managedBulkUnavailableToast =
  "Can't insert while Tine-managed storage is changing state. Nothing was changed.";

function boundedPageBlockCount(page: FeedPage, cap: number): number {
  let count = 0;
  const stack = [...page.roots];
  while (stack.length && count < cap) {
    const id = stack.pop()!;
    const node = doc.byId[id];
    if (!node) continue;
    count++;
    for (let index = node.children.length - 1; index >= 0; index--) stack.push(node.children[index]);
  }
  return count;
}

/**
 * Advisory, side-effect-free preflight for one selected bulk route. Direct
 * bindings return before the caller builds its plan; managed records only
 * reject a known lower-bound overflow and leave the native actor authoritative.
 */
export function preflightManagedBulkInsertion(
  targetId: string | null,
  buildPlan: (limits: ManagedBulkAdmissionLimits) => ManagedBulkInsertionPlan,
  targetPageName?: string,
): ManagedBulkInsertionPreflight {
  const admission = managedStorageRuntime.snapshot().applicationPageAdmission;
  if (admission?.authority === "direct") return { kind: "direct" };
  if (!admission) return { kind: "refused", toast: managedBulkUnavailableToast };
  if (admission.authority !== "managed_writable") {
    return { kind: "refused", toast: managedBulkUnavailableToast };
  }

  const target = targetId === null ? null : doc.byId[targetId];
  if (targetId !== null && (!target || !blockWritable(targetId))) {
    return { kind: "refused", toast: managedBulkUnavailableToast };
  }
  const pageName = target?.page ?? targetPageName;
  if (!pageName || !pageWritable(pageName)) return { kind: "refused", toast: managedBulkUnavailableToast };
  const page = pageByName(pageName);
  const targetGeneration = pageInstanceGeneration(pageName);
  if (!page || targetGeneration === null) return { kind: "refused", toast: managedBulkUnavailableToast };
  const limits: ManagedBulkAdmissionLimits = {
    applicationSavePageBlocks: admission.application_save_page_blocks,
    applicationPageRequestTextBytes: admission.application_page_request_text_bytes,
    applicationPageMaxDepth: admission.application_page_max_depth,
  };
  const plan = buildPlan(limits);
  const currentBlocks = boundedPageBlockCount(page, limits.applicationSavePageBlocks + 1);
  const exceedsBlockLimit =
    currentBlocks - plan.removedOrReusedDescendants + plan.insertedDescendants
      > limits.applicationSavePageBlocks;
  const exceedsDepthLimit = plan.maximumInputRelativeDepth > 0
    && plan.insertionRootDepth + plan.maximumInputRelativeDepth - 1
      > limits.applicationPageMaxDepth;
  const exceedsTextLimit = plan.insertedRawTextUtf8Bytes > limits.applicationPageRequestTextBytes;
  if (exceedsBlockLimit || exceedsDepthLimit || exceedsTextLimit) {
    return { kind: "refused", toast: managedBulkOverflowToast(limits.applicationSavePageBlocks) };
  }

  const token: InternalBulkInsertionAdmission = {
    [bulkInsertionAdmissionSeal]: true,
    consumed: false,
    targetId,
    targetNode: target ? unwrap(target) : null,
    targetPage: pageName,
    targetGeneration,
    graphEpoch: graphEpoch(),
    graphRoot: graphMeta()?.root ?? "",
    bindingGeneration: admission.binding_generation,
    authority: admission.authority,
    plan,
  };
  return { kind: "admitted", token };
}

/** Consume an admission immediately before its selected store publication. */
export function consumeManagedBulkInsertionAdmission(
  token: BulkInsertionAdmission,
  targetId: string | null,
): boolean {
  const internal = token as InternalBulkInsertionAdmission;
  if (!internal[bulkInsertionAdmissionSeal] || internal.consumed || internal.targetId !== targetId) return false;
  const admission = managedStorageRuntime.snapshot().applicationPageAdmission;
  const target = targetId === null ? null : doc.byId[targetId];
  if (
    !admission
    || admission.authority !== "managed_writable"
    || admission.binding_generation !== internal.bindingGeneration
    || internal.authority !== admission.authority
    || (targetId === null
      ? internal.targetNode !== null
      : !target
        || unwrap(target) !== internal.targetNode
        || target.page !== internal.targetPage)
    || pageInstanceGeneration(internal.targetPage) !== internal.targetGeneration
    || graphEpoch() !== internal.graphEpoch
    || (graphMeta()?.root ?? "") !== internal.graphRoot
  ) return false;
  internal.consumed = true;
  return true;
}

export function reportManagedBulkInsertionRefusal(toast: string): void {
  pushToast(toast, "error");
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
  /** Page-instance generations this entry was recorded against (GH #305). */
  instances: Record<string, number>;
  /** Identity-bearing clipboard paste whose redo must fail on a live conflict. */
  preservedIds?: string[];
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
  /** Page-instance generations this entry was recorded against (GH #305). */
  instances: Record<string, number>;
  preservedIds?: string[];
}
type UndoEntry = SnapEntry | RawEntry;
const undoStack: UndoEntry[] = [];
let redoStack: UndoEntry[] = [];
let lastUndoTag: string | null = null;
let undoSuppressionDepth = 0;

/**
 * Repeated selection moves have a deliberately narrower coalescing rule than
 * ordinary structural commands. Every key repeat still changes the live Solid
 * document immediately; this ledger only lets the repeats share their first
 * page-scoped undo snapshot.
 */
interface MoveSelectionBurst {
  /** Ordered top-level selection roots. A set is insufficient: order is state. */
  roots: string[];
  /** Sorted exact page-instance scope captured by the first nudge. */
  pages: Array<{ name: string; instance: number }>;
  /** Prevents reuse across Undo/Redo/history replacement. */
  historyEpoch: number;
  startedAt: number;
  lastCommandAt: number;
  idleTimer: ReturnType<typeof setTimeout> | null;
}

let moveSelectionBurst: MoveSelectionBurst | null = null;
let historyEpoch = 0;
const MOVE_SELECTION_BURST_IDLE_MS = 400;
const MOVE_SELECTION_BURST_MAX_MS = 3_000;

function endMoveSelectionBurst(): void {
  const idleTimer = moveSelectionBurst?.idleTimer;
  if (idleTimer !== null && idleTimer !== undefined) clearTimeout(idleTimer);
  moveSelectionBurst = null;
}

function advanceHistoryEpoch(): void {
  historyEpoch++;
}

function armMoveSelectionBurstIdleTimer(burst: MoveSelectionBurst): void {
  if (burst.idleTimer !== null) clearTimeout(burst.idleTimer);
  burst.idleTimer = setTimeout(() => {
    // A later repeat arms a different timer; an obsolete timer must not close it.
    if (moveSelectionBurst !== burst) return;
    if (Date.now() - burst.lastCommandAt >= MOVE_SELECTION_BURST_IDLE_MS) endMoveSelectionBurst();
  }, MOVE_SELECTION_BURST_IDLE_MS);
}

function currentMoveSelectionScope(ids: readonly string[]): Array<{ name: string; instance: number }> | null {
  const names = [...new Set(ids.map((id) => doc.byId[id]?.page).filter(Boolean) as string[])].sort();
  const pages: Array<{ name: string; instance: number }> = [];
  for (const name of names) {
    const instance = pageInstanceGeneration(name);
    if (instance === null) return null;
    pages.push({ name, instance });
  }
  return pages;
}

function sameMoveSelectionScope(
  left: readonly { name: string; instance: number }[],
  right: readonly { name: string; instance: number }[],
): boolean {
  return left.length === right.length
    && left.every((page, index) => page.name === right[index]?.name && page.instance === right[index]?.instance);
}

/**
 * Start a fresh selection-move undo gesture, or reuse its first snapshot for a
 * matching short repeat. Up and Down deliberately share this command family:
 * reversing direction is still one continuous selection-move gesture.
 */
function beginOrContinueMoveSelectionUndo(ids: readonly string[]): string[] | null {
  const pages = currentMoveSelectionScope(ids);
  if (!pages?.length) return null;
  const now = Date.now();
  const current = moveSelectionBurst;
  const matching = current
    && current.historyEpoch === historyEpoch
    && current.roots.length === ids.length
    && current.roots.every((id, index) => id === ids[index])
    && sameMoveSelectionScope(current.pages, pages)
    && now - current.lastCommandAt < MOVE_SELECTION_BURST_IDLE_MS
    && now - current.startedAt < MOVE_SELECTION_BURST_MAX_MS;
  if (matching) {
    current.lastCommandAt = now;
    armMoveSelectionBurstIdleTimer(current);
    return pages.map((page) => page.name);
  }

  endMoveSelectionBurst();
  // This one call opts out of the generic undo reset below. It is the only
  // structural command allowed to retain a previous snapshot.
  pushUndo("move-sel", pages.map((page) => page.name), undefined, { keepMoveSelectionBurst: true });
  const burst: MoveSelectionBurst = {
    roots: [...ids],
    pages,
    historyEpoch,
    startedAt: now,
    lastCommandAt: now,
    idleTimer: null,
  };
  moveSelectionBurst = burst;
  armMoveSelectionBurstIdleTimer(burst);
  return pages.map((page) => page.name);
}

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
  endMoveSelectionBurst();
  advanceHistoryEpoch();
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

/** The page-instance generations an entry is being recorded against.
 *
 *  An undo entry describes ONE loaded instance of each page it touches. Eviction
 *  deliberately keeps history, and re-opening the page installs a fresh instance
 *  carrying whatever the file says NOW — so replaying the old entry would restore
 *  pre-eviction content and mark the page dirty, and the save guard would accept
 *  it, because the baseline it submits under genuinely matches disk. Nothing in
 *  that path looks like a conflict. Stamping the generation makes the staleness
 *  visible at replay time, and covers every other way an instance is swapped
 *  (reload, rebind, forget) rather than only the reload-in-place path
 *  `invalidateUndoForPage` already handles. (GH #305) */
function captureInstances(names: readonly string[]): Record<string, number> {
  const instances: Record<string, number> = {};
  for (const name of names) {
    const generation = pageInstanceGeneration(name);
    if (generation !== null) instances[name] = generation;
  }
  return instances;
}

/** True when every page the entry describes is still the same loaded instance. */
function entryIsReplayable(e: UndoEntry): boolean {
  for (const name of Object.keys(e.instances)) {
    // peek, never the lazily-activating reader: minting a generation here would
    // make the check pass by inventing the identity it is supposed to compare.
    if (peekPageInstanceGeneration(name) !== e.instances[name]) return false;
  }
  return true;
}

/** Drop the popped stale entry's whole page history and say so once. Silence
 *  would read as "undo did nothing", which is how this class of bug hides. */
function discardStaleHistory(e: UndoEntry): void {
  for (const name of Object.keys(e.instances)) {
    if (peekPageInstanceGeneration(name) !== e.instances[name]) invalidateUndoForPage(name);
  }
  lastUndoTag = null;
  pushToast("Undo history for this page was discarded: the page was reloaded since those edits", "info");
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
  // A page-instance replacement is a history boundary for the whole working
  // set, not only for bursts whose snapshot named that page. An unrelated
  // sidebar/watcher reload still changes the history episode; allowing an
  // active move gesture to span it can reuse a pre-reload snapshot afterwards.
  endMoveSelectionBurst();
  advanceHistoryEpoch();
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
function snapEntry(affected?: string[] | null, preservedIds?: readonly string[]): SnapEntry {
  // Vite replaces MODE at build time, so this diagnostic is dead-code-eliminated
  // from production snapshots rather than adding an observer check to undo.
  if (import.meta.env.MODE === "test") {
    storeMutationObserverForTest?.({ kind: "undo-snapshot" });
  }
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
  if (import.meta.env.MODE === "test") {
    recordClipboardUndoSnapshotForTest(
      Object.keys(nodes).length,
      Object.values(nodes).reduce((total, node) => total + new TextEncoder().encode(node.raw).byteLength, 0),
    );
  }
  return {
    kind: "snap",
    pages: affected ?? null,
    pageObjs,
    nodes,
    dirty: names,
    context,
    instances: captureInstances(names),
    ...(preservedIds?.length ? { preservedIds: [...preservedIds] } : {}),
  };
}

/** Snapshot before a STRUCTURAL op. Pass the affected page name(s) so both the
 *  snapshot AND the undo re-save are scoped to just those pages; omit only when
 *  the op's page set isn't known (falls back to the whole working set — correct
 *  but O(loaded pages)). The affected set MUST include every page whose nodes the
 *  op changes, including a cross-page move's source AND destination, or undo
 *  would miss a page. `tag` resets the typing-coalesce marker. */
function pushUndo(
  tag: string,
  affected?: string[],
  preservedIds?: readonly string[],
  opts: { keepMoveSelectionBurst?: boolean } = {},
) {
  if (undoSuppressionDepth > 0) return;
  if (!opts.keepMoveSelectionBurst) endMoveSelectionBurst();
  advanceHistoryEpoch();
  undoStack.push(snapEntry(affected, preservedIds));
  if (undoStack.length > 200) undoStack.shift();
  redoStack = [];
  lastUndoTag = tag;
}

/** Record an O(1) inverse patch for a single-block text edit (typing). A typing
 *  burst in one block coalesces to a single entry holding the pre-burst text. */
function pushRawUndo(id: string, prevRaw: string) {
  if (undoSuppressionDepth > 0) return;
  endMoveSelectionBurst();
  advanceHistoryEpoch();
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
    instances: captureInstances([node.page]),
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
      instances: captureInstances([e.page]),
      ...(node && rootIndex >= 0 ? { headerRoot: { node: cloneNode(node), rootIndex } } : {}),
      ...(e.preservedIds?.length ? { preservedIds: [...e.preservedIds] } : {}),
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
  const inverse = snapEntry(e.pages, e.preservedIds);
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
  endMoveSelectionBurst();
  advanceHistoryEpoch();
  const entry = popHistoryEntry(undoStack);
  if (!entry) return;
  if (!entryIsReplayable(entry)) return discardStaleHistory(entry);
  redoStack.push(applyEntry(entry));
  lastUndoTag = null;
  endEdit("undo");
  scheduleSave();
  restoreEntryContext(entry.context);
}

export function redo() {
  endMoveSelectionBurst();
  advanceHistoryEpoch();
  const entry = popHistoryEntry(redoStack);
  if (!entry) return;
  if (!entryIsReplayable(entry)) return discardStaleHistory(entry);
  if (entry.preservedIds && hasLoadedIdentityCollision(entry.preservedIds)) {
    // The selected prerequisite is already popped. A later redo snapshot cannot
    // remain valid without it, including in page-only mode where the tagged
    // entry may have been selected from the middle of the global stack.
    redoStack = [];
    pushToast("Redo skipped: a block with the same id now exists", "error");
    return;
  }
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

/** Forward-merge (Delete at the END of a block): absorb the NEXT visible block's
 *  text into the current one, so caret stays at the join point in the current
 *  block. Exact structural mirror of mergeWithPrev: visible concatenation with
 *  no separator, current block keeps its identity and hidden props, the absorbed
 *  block's id:: line attaches only if the survivor has none (inbound ((id))
 *  references to the absorbed block must not orphan), absorbed children append to
 *  the current block's, one "merge" undo snapshot, same-page only (GH #213). */
export function mergeWithNext(
  id: string,
  scope: OutlineScope | null = null,
  editingSurface: string | null = null,
): boolean {
  if (!blockWritable(id)) return false;
  const next = nextVisible(id, scope);
  if (next === null) return false;
  const node = doc.byId[id];
  const nextNode = doc.byId[next];
  if (nextNode.page !== node.page) return false; // don't merge across pages
  pushUndo("merge", [node.page]);
  const fmt = formatForBlock(id); // next is same page (checked above) → same format
  // Merge visible content only; keep the current block's hidden props (it keeps
  // its identity) and drop the absorbed block's hidden props.
  const curSplit = splitProps(node.raw, isBuiltinHidden, fmt);
  const nextSplit = splitProps(nextNode.raw, isBuiltinHidden, fmt);
  const nextVisibleText = nextSplit.visible;
  const joinOffset = curSplit.visible.length;
  const pageName = node.page;

  let hidden = curSplit.hidden;
  const idPresent = fmt === "org" ? /(?:^|\n):id:\s/i : /(?:^|\n)id:: /i;
  const idLine = fmt === "org" ? /(?:^|\n)(:id:\s*\S+)/i : /(?:^|\n)(id:: \S+)/i;
  const survivorHasId = idPresent.test(curSplit.hidden);
  const absorbedId = idLine.exec(nextSplit.hidden)?.[1];
  if (!survivorHasId && absorbedId) {
    hidden = hidden ? `${hidden}\n${absorbedId}` : absorbedId;
  }

  setDoc(
    produce((s) => {
      s.byId[id].raw = joinProps(curSplit.visible + nextVisibleText, hidden, fmt);
      for (const c of nextNode.children) s.byId[c].parent = id;
      s.byId[id].children.push(...nextNode.children);
      const arr = nextNode.parent === null
        ? s.pages[s.pages.findIndex((p) => p.name === pageName)].roots
        : s.byId[nextNode.parent].children;
      arr.splice(arr.indexOf(next), 1);
      delete s.byId[next];
    })
  );
  startEditing(id, joinOffset, null, editingSurface);
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

/** Replace a leaf slash-template trigger with its expanded outline in one
 * publication. This is deliberately narrower than the generic insertion
 * helpers: selected template admission has already accounted for reusing this
 * host, so it must not first create an over-limit temporary sibling. */
export function replaceTemplateTriggerWithOutline(id: string, nodes: OutlineNode[]): string {
  const current = doc.byId[id];
  if (!nodes.length || !current || current.children.length || !blockWritable(id)) return id;
  const format = formatForBlock(id);
  const hidden = splitProps(current.raw, isBuiltinHidden, format).hidden;
  let lastId = id;
  setDoc(produce((state) => {
    const create = (outline: OutlineNode, parent: string | null, reuseId?: string): string => {
      const created = reuseId ?? freshId();
      const children = outline.children.map((child) => create(child, created));
      const sourceRaw = reuseId ? joinProps(outline.raw, hidden, format) : outline.raw;
      state.byId[created] = {
        id: created,
        raw: rawWithInheritedOrderListType(sourceRaw, format, id),
        collapsed: false,
        parent,
        page: current.page,
        children,
      };
      return created;
    };
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

/** Materialize a parsed quick-capture outline as the first roots of an empty
 * page. The caller owns one undo unit; no empty anchor is ever published. */
function insertCaptureOutlineIntoEmptyPage(pageName: string, nodes: OutlineNode[]): string | null {
  const page = pageByName(pageName);
  if (!page || page.roots.length || !nodes.length || !pageWritable(pageName)) return null;
  const format = formatForPage(pageName);
  let lastId: string | null = null;
  setDoc(produce((state) => {
    const create = (outline: OutlineNode, parent: string | null): string => {
      const id = freshId();
      const children = outline.children.map((child) => create(child, id));
      state.byId[id] = {
        id,
        raw: rawWithInheritedOrderListType(outline.raw, format, null),
        collapsed: false,
        parent,
        page: pageName,
        children,
      };
      return id;
    };
    const roots = nodes.map((node) => create(node, null));
    state.pages[state.pages.findIndex((candidate) => candidate.name === pageName)].roots.push(...roots);
    lastId = roots[roots.length - 1] ?? null;
  }));
  markDirty(pageName);
  return lastId;
}

type ClipboardProperty = { key: string; value: string };

function clipboardProperties(raw: string, format: Format): ClipboardProperty[] {
  const hidden = splitProps(raw, hideAll, format).hidden;
  if (!hidden) return [];
  const properties: ClipboardProperty[] = [];
  for (const line of hidden.split("\n")) {
    const match = format === "org"
      ? /^\s*:([A-Za-z0-9_@./-]+):\s*(.*)$/.exec(line)
      : /^\s*([A-Za-z0-9_./-]+)::\s*(.*)$/.exec(line);
    if (match) properties.push({ key: match[1], value: match[2] });
  }
  return properties;
}

function clipboardIdsForBlock(block: ClipboardBlock): string[] {
  return clipboardProperties(block.raw, block.sourceFormat)
    .filter((property) => property.key.toLowerCase() === "id")
    .map((property) => property.value.trim());
}

function clipboardRawForTarget(
  block: ClipboardBlock,
  targetFormat: Format,
  preserveIds: boolean,
): string {
  if (block.sourceFormat === targetFormat) {
    return preserveIds
      ? block.raw
      : splitProps(block.raw, (key) => key.toLowerCase() === "id", block.sourceFormat).visible;
  }

  // splitProps/joinProps classify metadata but deliberately do not translate
  // syntax. Map the ordered key/value stream explicitly so every property keeps
  // its relative order across Markdown `key:: value` and Org drawer forms.
  const visible = splitProps(block.raw, hideAll, block.sourceFormat).visible;
  const properties = clipboardProperties(block.raw, block.sourceFormat)
    .filter((property) => preserveIds || property.key.toLowerCase() !== "id");
  const translated = properties.map(({ key, value }) =>
    targetFormat === "org" ? `:${key}: ${value}` : `${key}:: ${value}`
  ).join("\n");
  return joinProps(visible, translated, targetFormat);
}

function clipboardCollapsed(block: ClipboardBlock): boolean {
  return clipboardProperties(block.raw, block.sourceFormat)
    .some(({ key, value }) => key.toLowerCase() === "collapsed" && value.trim().toLowerCase() === "true");
}

function liveDocReferences(id: string): boolean {
  const escaped = id.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const reference = new RegExp(`\\(\\(${escaped}\\)\\)`, "i");
  return Object.values(doc.byId).some((node) => reference.test(node.raw));
}

interface ClipboardPasteAuthority {
  epoch: number;
  root: string;
  targetId: string;
  targetNode: Node;
  targetPage: string;
  targetGeneration: number;
}

function captureClipboardPasteAuthority(targetId: string): ClipboardPasteAuthority | null {
  const target = doc.byId[targetId];
  if (!target || graphTransitioning()) return null;
  const targetGeneration = pageInstanceGeneration(target.page);
  if (targetGeneration === null) return null;
  return {
    epoch: graphEpoch(),
    root: graphMeta()?.root ?? "",
    targetId,
    targetNode: unwrap(target),
    targetPage: target.page,
    targetGeneration,
  };
}

function clipboardPasteAuthorityCurrent(authority: ClipboardPasteAuthority): boolean {
  const target = doc.byId[authority.targetId];
  return !graphTransitioning()
    && graphEpoch() === authority.epoch
    && (graphMeta()?.root ?? "") === authority.root
    && !!target
    && unwrap(target) === authority.targetNode
    && target.page === authority.targetPage
    && pageInstanceGeneration(authority.targetPage) === authority.targetGeneration;
}

function clipboardTargetReusesEmptyHost(target: Node, targetFormat: Format): boolean {
  const visible = splitProps(target.raw, isBuiltinHidden, targetFormat).visible;
  return target.children.length === 0
    && visible.trim() === ""
    && existingBlockId(target.raw, targetFormat) === null
    && !liveDocReferences(target.id);
}

function insertClipboardBlocksSync(
  targetId: string,
  blocks: readonly ClipboardBlock[],
  preserveIds: boolean,
  preservedIds: readonly string[],
  reuseEmptyHost?: boolean,
): string | null {
  const target = doc.byId[targetId];
  if (!blocks.length || !target || !blockWritable(targetId)) return null;
  const targetFormat = formatForPage(target.page);
  const prepared = blocks.map(function prepare(block): {
    id: string;
    raw: string;
    collapsed: boolean;
    children: ReturnType<typeof prepare>[];
  } {
    if (import.meta.env.MODE === "test") recordClipboardWorkForTest("prepared_destination_nodes");
    const sourceIds = clipboardIdsForBlock(block);
    return {
      id: preserveIds && sourceIds.length === 1 ? sourceIds[0].toLowerCase() : freshId(),
      raw: clipboardRawForTarget(block, targetFormat, preserveIds),
      collapsed: clipboardCollapsed(block),
      children: block.children.map(prepare),
    };
  });
  const replaceHost = reuseEmptyHost ?? clipboardTargetReusesEmptyHost(target, targetFormat);
  const parent = target.parent;
  const pageName = target.page;
  let lastId: string | null = null;

  if (import.meta.env.MODE === "test") {
    recordClipboardWorkForTest("target_insertion_phases");
    recordClipboardPhaseForTest("target-insertion");
  }
  pushUndo("clipboard-paste", [pageName], preserveIds ? preservedIds : []);
  setDoc(produce((state) => {
    const create = (block: typeof prepared[number], blockParent: string | null): string => {
      if (import.meta.env.MODE === "test") recordClipboardWorkForTest("allocated_destination_nodes");
      const children = block.children.map((child) => create(child, block.id));
      state.byId[block.id] = {
        id: block.id,
        raw: block.raw,
        collapsed: block.collapsed,
        parent: blockParent,
        page: pageName,
        children,
      };
      return block.id;
    };
    const created = prepared.map((block) => create(block, parent));
    const siblings = parent === null
      ? state.pages[state.pages.findIndex((page) => page.name === pageName)].roots
      : state.byId[parent].children;
    const at = siblings.indexOf(targetId);
    if (replaceHost) {
      siblings.splice(at, 1, ...created);
      delete state.byId[targetId];
    } else {
      siblings.splice(at + 1, 0, ...created);
    }
    lastId = created[created.length - 1] ?? null;
  }));
  markDirty(pageName);
  return lastId;
}

/** Associate an already-captured private clipboard slot with one target. The
 * wrapper is intentionally non-async: a cut grant is consumed synchronously,
 * before the returned continuation can reach retirement or any other await. */
export function pasteClipboardPayload(
  targetId: string,
  slot: ClipboardPayloadSlot,
): Promise<string | null> {
  const authority = captureClipboardPasteAuthority(targetId);
  if (!authority) return Promise.resolve(null);

  let managedReuseEmptyHost: boolean | null = null;
  const admission = preflightManagedBulkInsertion(targetId, (limits) => {
    const target = doc.byId[targetId]!;
    managedReuseEmptyHost = clipboardTargetReusesEmptyHost(target, formatForPage(target.page));
    return managedBulkOutlinePlan(
      slot.blocks,
      depthOf(targetId) + 1,
      managedReuseEmptyHost ? 1 : 0,
      limits,
    );
  });
  if (admission.kind === "refused") {
    reportManagedBulkInsertionRefusal(admission.toast);
    return Promise.resolve(null);
  }

  // This must remain after the initial synchronous admission: a known target
  // overflow leaves a Cut grant intact for a smaller retry.
  const grant = slot.op === "cut" ? consumeCutGrant(slot.generation) : null;

  const idLists: string[][] = [];
  const visit = (block: ClipboardBlock) => {
    idLists.push(clipboardIdsForBlock(block));
    block.children.forEach(visit);
  };
  slot.blocks.forEach(visit);
  const ids = idLists.flat();
  const normalizedIds = ids.map((id) => id.toLowerCase());
  const idsValid = idLists.every((blockIds) => blockIds.length <= 1)
    && ids.every((id) => UUID_RE.test(id))
    && new Set(normalizedIds).size === normalizedIds.length;

  return (async () => {
    let preserveIds = !!grant
      && ids.length > 0
      && idsValid
      && slot.graph === authority.root;

    if (preserveIds) {
      if (import.meta.env.MODE === "test") {
        recordClipboardWorkForTest("source_retirement_phases");
        recordClipboardPhaseForTest("source-retirement");
      }
      preserveIds = await flushCutSourcePages(grant!.sourcePages);
      if (preserveIds && !clipboardPasteAuthorityCurrent(authority)) return null;
    }
    if (preserveIds) {
      try {
        if (import.meta.env.MODE === "test") {
          recordClipboardWorkForTest("resolve_blocks_phases");
          recordClipboardPhaseForTest("resolve-blocks");
        }
        const resolved = await backend().resolveBlocks(normalizedIds);
        preserveIds = resolved.length === normalizedIds.length && resolved.every((block) => block === null);
      } catch {
        preserveIds = false;
      }
    }

    // Final JS-single-thread section: every authority and retirement check is
    // synchronous and insertion follows immediately with no await boundary.
    if (!clipboardPasteAuthorityCurrent(authority)) return null;
    if (preserveIds) {
      if (import.meta.env.MODE === "test") {
        recordClipboardWorkForTest("final_identity_guard_phases");
        recordClipboardPhaseForTest("final-identity-guard");
      }
      preserveIds = cutSourcePagesRetired(grant!.sourcePages)
        && !hasLoadedIdentityCollision(normalizedIds);
    }
    if (admission.kind === "admitted" && !consumeManagedBulkInsertionAdmission(admission.token, targetId)) {
      return null;
    }
    const reuseEmptyHost = admission.kind === "admitted"
      ? managedReuseEmptyHost!
      : clipboardTargetReusesEmptyHost(doc.byId[targetId], formatForPage(doc.byId[targetId].page));
    return insertClipboardBlocksSync(
      targetId,
      slot.blocks,
      preserveIds,
      preserveIds ? normalizedIds : [],
      reuseEmptyHost,
    );
  })();
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
    const binding = graphBinding();
    const dto: PageDto =
      (await backend().getPage(name, kind)) ??
      { name, kind, title: name, pre_block: null, blocks: [], rev: null };
    // Stop on a refusal rather than falling through to `pageByName` for the name
    // slot: that would append the capture into whichever editor is loaded under
    // this name, which on a refusal is a DIFFERENT file. Returning false keeps
    // the capture text where the caller can retry it. (GH #254 increment 3.)
    if (await ensurePageLoaded(dto, { expectedGraphBinding: binding })) return false;
  }
  const page = pageByName(name);
  if (!page || !pageWritable(name)) return false;
  const insertionTarget = page.roots.length ? page.roots[page.roots.length - 1] : null;
  const admission = preflightManagedBulkInsertion(
    insertionTarget,
    (limits) => managedBulkOutlinePlan(
      nodes,
      insertionTarget === null ? 1 : depthOf(insertionTarget) + 1,
      0,
      limits,
    ),
    name,
  );
  if (admission.kind === "refused") {
    reportManagedBulkInsertionRefusal(admission.toast);
    return false;
  }
  if (
    admission.kind === "admitted"
    && !consumeManagedBulkInsertionAdmission(admission.token, insertionTarget)
  ) return false;
  if (page.roots.length) {
    // Append after the last top-level block (end of the page).
    insertOutlineAfter(insertionTarget!, nodes);
  } else {
    withUndoUnit("capture", [name], () => insertCaptureOutlineIntoEmptyPage(name, nodes));
  }
  return await flushPage(name);
}

const PROP_LINE = /^([A-Za-z0-9_./-]+):: ?(.*)$/;

/** Pure Markdown property rewrite for one compound store mutation. It scans only
 * the canonical head (title, planning, contiguous properties) plus the legacy
 * trailing property block, so a `key::` lookalike in body text or a code fence is
 * never touched or reordered. Existing property order is retained. */
function markdownRawWithProperty(raw: string, key: string, value: string | null): string {
  const lines = raw.split("\n");
  const first = lines[0] ?? "";
  const PLANNING_LINE = /^\s*(SCHEDULED|DEADLINE):\s*</;
  let i = 1;
  while (i < lines.length && PLANNING_LINE.test(lines[i])) i++;
  const planningEnd = i;
  while (i < lines.length && PROP_LINE.test(lines[i])) i++;
  const propsEnd = i;
  let j = lines.length;
  while (j > propsEnd && PROP_LINE.test(lines[j - 1] ?? "")) j--;
  const notKey = (l: string) => PROP_LINE.exec(l)?.[1] !== key;
  const props = lines.slice(planningEnd, propsEnd);
  const at = props.findIndex((l) => PROP_LINE.exec(l)?.[1] === key);
  if (value !== null) {
    const line = `${key}:: ${value}`;
    if (at >= 0) props[at] = line;
    else props.push(line);
  } else if (at >= 0) {
    props.splice(at, 1);
  }
  return [
    first,
    ...lines.slice(1, planningEnd),
    ...props,
    ...lines.slice(propsEnd, j),
    ...lines.slice(j).filter(notKey),
  ].join("\n");
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
  // Canonical head-region placement plus legacy trailing-property cleanup lives
  // in the shared pure writer so compound mutations (heading transitions) can
  // remain one undo-safe raw rewrite.
  setDoc("byId", id, "raw", markdownRawWithProperty(node.raw, key, value));
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

export type HeadingState = number | true | null;

const MARKDOWN_HEADING = /^#+\s+/;
const clearMarkdownHeading = (raw: string): string => raw.replace(MARKDOWN_HEADING, "");
const setMarkdownHeading = (raw: string, level: number): string => {
  const prefix = `${"#".repeat(level)} `;
  return MARKDOWN_HEADING.test(raw)
    ? raw.replace(MARKDOWN_HEADING, prefix)
    : prefix + raw.trimStart();
};

/** Pure format-aware heading transition shared by single-block and selection
 * commands so their Markdown/Org serialization cannot drift apart. */
function rawWithHeading(raw: string, format: Format, state: HeadingState): string {
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
  markDirty(node.page);
}

/** Apply a context heading command to the active selection, falling back to the
 * pointer block only when no selection is active. The preflight makes a mixed
 * writable/read-only selection an exact no-op. */
export function setSelectionHeading(pointerId: string, state: HeadingState): boolean {
  const selected = selectedIds();
  const ids = selected.length ? selected : [pointerId];
  if (!ids.length || ids.some((id) => !blockWritable(id))) return false;

  const changes = ids.map((id) => ({
    id,
    page: doc.byId[id].page,
    raw: rawWithHeading(doc.byId[id].raw, formatForBlock(id), state),
  })).filter((change) => change.raw !== doc.byId[change.id].raw);
  if (!changes.length) return true;

  const pages = [...new Set(changes.map((change) => change.page))];
  pushUndo("heading-selection", pages);
  setDoc(produce((stateDoc) => {
    for (const change of changes) stateDoc.byId[change.id].raw = change.raw;
  }));
  for (const page of pages) markDirty(page);
  return true;
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
  const targetLine = new RegExp(`^\\s*${tag}:`);
  const targetTimestamp = new RegExp(`^\\s*${tag}:\\s*<[^>]+>(.*)$`);
  const keptHead: string[] = [];
  const trailingBody: string[] = [];
  for (const line of all.slice(1, headEnd)) {
    if (!targetLine.test(line)) {
      keptHead.push(line);
      continue;
    }
    const match = targetTimestamp.exec(line);
    if (match?.[1].trim()) trailingBody.push(match[1]);
  }
  // A glued suffix is user body content, not part of the replaced timestamp.
  // Keep the canonical planning/property head contiguous and split that suffix
  // into body lines immediately after it instead of dropping bytes.
  const lines = [all[0], ...keptHead, ...trailingBody, ...all.slice(headEnd)];
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

/** The identity other blocks and persisted UI state must use for a loaded node.
 * A freshly-created node keeps its transient `b…` store key for the whole live
 * session even after Copy block ref writes a UUID property into `raw`; external
 * references must follow that property while render/edit paths keep the key. */
export function blockExternalId(id: string): string | null {
  const node = doc.byId[id];
  if (!node) return null;
  return existingBlockId(node.raw, formatForBlock(id)) ?? node.id;
}

export interface LoadedBlockRef {
  uuid: string;
  page: string;
  pageKind: PageKind;
  path?: string;
}

/** Resolve a durable external UUID back to the current live store key. The page
 * descriptor is part of the identity: even a direct `byId[uuid]` hit is rejected
 * when it belongs to another page kind or physical path. */
export function resolveBlockRef(ref: LoadedBlockRef): string | null {
  const owner = pageByName(ref.page);
  if (
    !owner
    || owner.kind !== ref.pageKind
    || (ref.path !== undefined && owner.path !== ref.path)
  ) return null;

  const matches = (id: string): boolean => {
    const node = doc.byId[id];
    return !!node && node.page === ref.page && blockExternalId(id) === ref.uuid;
  };
  if (matches(ref.uuid)) return ref.uuid;

  const stack = [...owner.roots];
  const seen = new Set<string>();
  while (stack.length) {
    const id = stack.pop()!;
    if (seen.has(id)) continue;
    seen.add(id);
    const node = doc.byId[id];
    if (!node || node.page !== ref.page) continue;
    if (matches(id)) return id;
    stack.push(...node.children);
  }
  return null;
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

/** A live reference to a loaded block: its durable external UUID plus its exact
 * owner. The UUID can differ from the live store key until the page is reloaded. */
export function blockRef(id: string): LoadedBlockRef {
  const n = doc.byId[id];
  const owner = pageByName(n.page);
  return {
    uuid: blockExternalId(id) ?? n.id,
    page: n.page,
    pageKind: owner?.kind ?? "page",
    ...(owner?.path ? { path: owner.path } : {}),
  };
}

const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

/** Ensure a block has a durable external UUID synchronously, while deliberately
 * leaving its live store key unchanged. Existing ids win; otherwise a fresh
 * transient key receives a UUID in the page's Markdown/Org property syntax. */
export function ensureStableBlockId(id: string): string | null {
  const node = doc.byId[id];
  if (!node || !blockWritable(id)) return null;
  const fmt = formatForBlock(id);
  const existing = existingBlockId(node.raw, fmt);
  if (existing) return existing;
  const uuid = UUID_RE.test(id) ? id : crypto.randomUUID();
  setDoc("byId", id, "raw", rawWithBlockId(node.raw, uuid, fmt));
  markDirty(node.page);
  // Persist now, not on the 400ms debounce: the user may quit right after
  // parking the block, and a pending timer is lost when the webview closes.
  void flushPage(node.page);
  return uuid;
}

/** Like `blockRef`, but first persists the block's `id::` so the reference
 *  resolves after a restart. Used for parking a block durably: the right sidebar,
 *  a new tab, and zoom all stamp `id::` so the spot survives a relaunch (Martin's
 *  call — he wants these to persist; the `id::` is harmless in the file and is
 *  stripped from clipboard copies anyway, see `blockSubtreeMarkdown`). */
export function persistentBlockRef(id: string): LoadedBlockRef {
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
  const ref: LoadedBlockRef = { uuid, page, pageKind: kind, ...(path ? { path } : {}) };
  // The GRAPH BINDING, not the render epoch: toggling typography or the journal
  // format bumps the epoch without the graph moving, and dropping a committed
  // reference's request because the user changed a display preference is loss
  // with no safety benefit at all. (GH #254 increment 3, round 12.)
  const epoch = graphBinding();
  if (!resolveBlockRef(ref)) {
    const dto = path
      ? await backend().getPageByPath(path)
      : await backend().getPage(page, kind);
    // A read that crossed a graph switch must not install into the NEW graph.
    if (epoch !== graphBinding()) return;
    // Nor may one that crossed a DELETION. This read may have been issued before
    // the user deleted the page; installing its pre-delete bytes puts the page
    // back, and `upsertPage` lifts the tombstone as it does so, after which the
    // stamp's own save recreates the file the user just deleted — with stale
    // content. Routing deletion through the store exists precisely to stop a
    // queued write resurrecting a page, and this is the same hazard arriving by
    // a different door. (GH #254 increment 3.)
    // Path-aware, not name-level: two files legitimately share one page name, and
    // deleting one must not refuse the other. Refusing by name loses the surviving
    // owner's durable target — work lost rather than protected.
    if (isTombstonedFile(page, dto?.path ?? path)) {
      // RETAIN, don't discard. A tombstone is raised BEFORE the backend delete
      // and lifted again if that delete fails (an ambiguous by-name delete of a
      // duplicated page name is rejected by core). Dropping the request here
      // threw away an already-committed reference's durable target on a delete
      // that never happened. Retaining costs nothing: the retry re-checks the
      // tombstone before it reads, so while the page stays deleted this waits
      // silently, and it re-drives if the page comes back.
      retainStamp({ uuid, page, kind, path, epoch });
      return;
    }
    if (dto && await ensurePageLoaded(dto, { expectedGraphBinding: epoch })) {
      // RETAIN the request. The user-visible mutation has already happened —
      // autocomplete committed `((uuid))`, or the sidebar item is already open —
      // and this stamp is what makes those survive a restart. Skipping it leaves
      // a reference that resolves now and is gone after a restart; rolling it
      // back would undo what the user just typed.
      //
      // Driven by the "became replaceable" transition, NOT by polling on
      // unrelated saves: three poll-shaped designs were each reproduced failing,
      // and the liveness half is why — a request stranded whenever the incumbent
      // resolved through a route that produced no such save.
      // (GH #254 increment 3, acceptance row C5.)
      retainStamp({ uuid, page, kind, path, epoch });
      return;
    }
  }
  // Re-check: a concurrent navigation may have loaded the page meanwhile, or the
  // cache may have been rebuilt (external change) and reassigned the block a new
  // uuid — in which case there's nothing safe to stamp.
  const id = resolveBlockRef(ref);
  if (id) {
    pendingBlockRefStamps.delete(uuid);
    ensureStableBlockId(id);
  }
}

/** Stamps deferred by a refused replacement, keyed by the referenced uuid. */
const pendingBlockRefStamps = new Map<
  string,
  { uuid: string; page: string; kind: PageKind; path?: string; epoch: number }
>();

type PendingStamp = {
  uuid: string;
  page: string;
  kind: PageKind;
  path?: string;
  epoch: number;
};

/** Stop-handles for the armed watchers, so re-retaining one request replaces its
 *  watcher instead of stacking a second one that would re-read the same page. */
const stampWatchers = new Map<string, () => void>();

/** Is a deferred stamp still waiting? The distinction that matters is "retained"
 *  versus "dropped": a retained request will resume, a dropped one is work the
 *  user committed and silently lost. Nothing else can observe that difference. */
export function hasPendingBlockRefStamp(uuid: string): boolean {
  return pendingBlockRefStamps.has(uuid);
}

/** A retained stamp belongs to the graph that deferred it. */
export function clearPendingBlockRefStamps(): void {
  pendingBlockRefStamps.clear();
  stampWatchers.clear();
  clearReplaceableWatchers();
}

/** Hold a deferred stamp and (re-)arm exactly one watcher for it. */
function retainStamp(req: PendingStamp) {
  stampWatchers.get(req.uuid)?.();
  pendingBlockRefStamps.set(req.uuid, req);
  const stop = onPageBecameReplaceable(req.page, () => {
    // Stay armed and read nothing only when the tombstone PROVABLY covers this
    // request — which means the request itself names the deleted file. Anything
    // weaker is unsound: a request that cannot name its file must READ, because
    // nothing else can tell it the page came back. (Caching the file a previous
    // read found looks like a cheap way to skip that read, and is wrong: an
    // unloaded page recreated at a DIFFERENT path never upserts, so the
    // tombstone is never lifted and the cached path refuses forever. Re-reading
    // on each announcement is the price of not stranding the request.)
    if (tombstoneCovers(req.page, req.path)) return;
    stop();
    stampWatchers.delete(req.uuid);
    pendingBlockRefStamps.delete(req.uuid);
    if (req.epoch !== graphBinding()) return;
    void persistBlockRefTarget(req.uuid, req.page, req.kind, req.path);
  });
  stampWatchers.set(req.uuid, stop);
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
  if (import.meta.env.MODE === "test") {
    recordClipboardWorkForTest("public_markdown_visits");
    recordClipboardWorkForTest("public_markdown_raw_bytes", new TextEncoder().encode(n.raw).byteLength);
  }
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

/**
 * Build the private clipboard forest from selection roots. Unlike the public
 * text flavor this always includes the complete subtree and exact raw strings,
 * including id::/collapsed:: and hidden properties. Returns null (without
 * affecting the public copy) when the bounded in-memory payload is too large.
 */
export function buildClipboardPayload(ids: string[]): ClipboardPayloadData | null {
  const selected = new Set(ids.filter((id) => !!doc.byId[id]));
  const hasSelectedAncestor = (id: string): boolean => {
    let parent = doc.byId[id]?.parent ?? null;
    while (parent !== null) {
      if (selected.has(parent)) return true;
      parent = doc.byId[parent]?.parent ?? null;
    }
    return false;
  };
  const roots = [...selected].filter((id) => !hasSelectedAncestor(id));
  if (roots.length === 0) return null;

  let blockCount = 0;
  let rawBytes = 0;
  const encoder = new TextEncoder();
  const pages = new Map<string, ClipboardSourcePage>();

  const build = (id: string): ClipboardBlock | null => {
    const node = doc.byId[id];
    if (!node) return null;
    if (import.meta.env.MODE === "test") {
      recordClipboardWorkForTest("private_payload_visits");
      recordClipboardWorkForTest("private_payload_raw_bytes", new TextEncoder().encode(node.raw).byteLength);
    }
    blockCount++;
    rawBytes += encoder.encode(node.raw).byteLength;
    if (blockCount > CLIPBOARD_PAYLOAD_MAX_BLOCKS || rawBytes > CLIPBOARD_PAYLOAD_MAX_RAW_BYTES) return null;

    const page = pageByName(node.page);
    const generation = pageInstanceGeneration(node.page);
    if (!page || generation === null) return null;
    if (!pages.has(page.name)) {
      pages.set(page.name, {
        name: page.name,
        kind: page.kind,
        ...(page.path ? { path: page.path } : {}),
        generation,
      });
    }

    const children: ClipboardBlock[] = [];
    for (const child of node.children) {
      const built = build(child);
      if (!built) return null;
      children.push(built);
    }
    return { raw: node.raw, children, sourceFormat: page.format };
  };

  const blocks: ClipboardBlock[] = [];
  for (const id of roots) {
    const built = build(id);
    if (!built) return null;
    blocks.push(built);
  }
  return { blocks, sourcePages: [...pages.values()] };
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

/** Selection endpoints are part of the move gesture identity even when their
 * normalized `topSelected()` roots happen to stay the same (for example moving
 * focus from a selected child back to its selected parent). */
function setSelectionAnchor(next: string | null): void {
  if (selAnchor() !== next) endMoveSelectionBurst();
  setSelAnchor(next);
}
function setSelectionFocus(next: string | null): void {
  if (selFocus() !== next) endMoveSelectionBurst();
  setSelFocus(next);
}

/** True when `ancestor` is a strict ancestor of `id` in the block tree. */
function isAncestorId(ancestor: string, id: string): boolean {
  let p = doc.byId[id]?.parent ?? null;
  while (p !== null) {
    if (p === ancestor) return true;
    p = doc.byId[p]?.parent ?? null;
  }
  return false;
}

/** Index of the last visible descendant of order[headIdx] within `order`.
 *  (A subtree occupies one contiguous DFS slice of the visible order.) */
function subtreeEndIndex(order: string[], headIdx: number): number {
  const head = order[headIdx];
  let end = headIdx;
  for (let k = headIdx + 1; k < order.length && isAncestorId(head, order[k]); k++) end = k;
  return end;
}

export function selectedIds(): string[] {
  const a = selAnchor();
  const f = selFocus();
  if (!a || !f) return [];
  const order = selectionOrder(a);
  const i = order.indexOf(a);
  const j = order.indexOf(f);
  if (i < 0 || j < 0) return [];
  const lo = Math.min(i, j);
  let hi = Math.max(i, j);
  // GH #262 Shift+Up: extending UP from inside a subtree onto its parent left
  // the slice [parent … anchor], omitting the parent's later children — a
  // partial-subtree selection that copy/cut/move received as "parent without
  // its children". (Shift+Down is the semantic reverse and never produces it:
  // a parent's children all follow it in visible order.) In a reverse slice,
  // any member that is an ancestor of the anchor may hold a partial subtree
  // inside the slice; complete its visible subtree.
  if (i > j) {
    for (let k = lo; k <= hi; k++) {
      if (isAncestorId(order[k], a)) {
        const end = subtreeEndIndex(order, k);
        if (end > hi) hi = end;
      }
    }
  }
  return order.slice(lo, hi + 1);
}
// Memoized set of selected ids. `isSelected` is read in the render of EVERY
// block (Block.tsx classList), and selectedIds() rebuilds visibleOrder() each
// call — so without this, a selection over N visible blocks costs O(N²). The
// memo recomputes only when the anchor/focus or the visible tree changes.
const selectedSet = createRoot(() => createMemo(() => new Set(selectedIds())));
export function isSelected(id: string): boolean {
  return selectedSet().has(id);
}
// Hierarchical Ctrl/Cmd+A (GH #262): the ancestor whose visible subtree the
// current select-all sequence covers. Any other selection mutation resets it.
let selectAllHead: string | null = null;

export function selectBlock(id: string, scope: OutlineScope | null = null) {
  endEdit("select-block");
  notifyOutlineSelectionStarted(id);
  activeSelectionScope = scope;
  selectAllHead = null;
  setSelectionAnchor(id);
  setSelectionFocus(id);
}
export function clearSelection() {
  setSelectionAnchor(null);
  setSelectionFocus(null);
  activeSelectionScope = null;
  selectAllHead = null;
}
/** Extend the current block selection's focus to `id` (mouse-drag / shift-click).
 *  Starts a fresh selection anchored at `id` if none is active. */
export function extendSelectionTo(id: string, scope: OutlineScope | null = activeSelectionScope) {
  notifyOutlineSelectionStarted(id);
  if (selAnchor() === null) {
    activeSelectionScope = scope;
    setSelectionAnchor(id);
  }
  if (activeSelectionScope && !scopedVisibleOrder(activeSelectionScope).includes(id)) return;
  selectAllHead = null;
  setSelectionFocus(id);
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
  selectAllHead = null;
  setSelectionFocus(next);
  if (!extend) setSelectionAnchor(next);
  scrollBlockRowIntoView(next);
}

/** Ctrl/Cmd+A while editing a block, with the block's text already fully
 *  selected: escalate to a block selection covering the block's whole visible
 *  subtree, and mark it the head of the hierarchy ladder (GH #262). */
export function selectBlockSubtree(id: string, scope: OutlineScope | null = null) {
  selectBlock(id, scope);
  const order = selectionOrder(id);
  const idx = order.indexOf(id);
  if (idx < 0) return;
  setSelectionFocus(order[subtreeEndIndex(order, idx)]);
  selectAllHead = id;
}

/** Repeated Ctrl/Cmd+A in block-selection mode: widen the selection one
 *  ancestor level at a time — subtree, parent subtree, …, the whole visible
 *  outline, where it stays (idempotent) — or start the ladder at the current
 *  selection's anchor when no select-all sequence is in progress (GH #262). */
export function expandBlockSelection() {
  const a = selAnchor();
  const f = selFocus();
  if (!a || !f) return;
  const order = selectionOrder(a);
  if (order.length < 2) return;
  // Whole-outline selection is the top of the ladder; further presses no-op.
  if (a === order[0] && f === order[order.length - 1]) return;
  let head = selectAllHead;
  if (head === null || !order.includes(head)) head = a;
  let idx = order.indexOf(head);
  if (idx < 0) return;
  // A head whose subtree is fully inside the current selection (exactly or
  // because the user extended past it) is covered: climb from there.
  if (a === head && order.indexOf(f) >= subtreeEndIndex(order, idx)) {
    // This subtree is already fully selected: climb to its parent, or to the
    // whole outline when the head is already a root (within the active scope).
    const parent = doc.byId[head]?.parent ?? null;
    if (parent !== null && order.includes(parent)) {
      head = parent;
      idx = order.indexOf(head);
    } else {
      setSelectionAnchor(order[0]);
      setSelectionFocus(order[order.length - 1]);
      selectAllHead = null;
      return;
    }
  }
  setSelectionAnchor(head);
  setSelectionFocus(order[subtreeEndIndex(order, idx)]);
  selectAllHead = head;
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

/**
 * Confirm the assumptions that the old per-root `moveBlockInternal` loop made
 * before we start the one-shot selection mutation. A normal visible selection
 * always meets these; a stale or malformed tree instead remains a guarded
 * no-op, rather than committing only a prefix of the selection.
 */
function canBatchMoveSelectionRoots(ids: readonly string[], destPage: string, newParent: string | null): boolean {
  if (newParent !== null) {
    const parent = doc.byId[newParent];
    if (!parent || !blockWritable(newParent) || parent.page !== destPage) return false;
  }

  for (const id of ids) {
    const node = doc.byId[id];
    if (!node || !blockWritable(id) || node.page !== destPage || id === newParent) return false;
    if (!rootsOf(id).includes(id)) return false;

    // Keep the old "never make a block its own ancestor" guard, but fail
    // closed on a malformed parent cycle instead of spinning forever.
    const seen = new Set<string>();
    let cursor = newParent;
    while (cursor !== null) {
      if (cursor === id || seen.has(cursor)) return false;
      seen.add(cursor);
      const ancestor = doc.byId[cursor];
      if (!ancestor) return false;
      cursor = ancestor.parent;
    }
  }
  return true;
}

/** Remove every selected root from its actual present sibling array, then put
 * them at one destination in visible/document order. This is deliberately one
 * Solid/Immer publication: selection indent/outdent is one editor command, not
 * N independent drag operations. */
function moveSelectionRootsInOneMutation(
  ids: readonly string[],
  destPage: string,
  destinationParent: string | null,
  destinationIndex: (state: DocState) => number,
  expandParent: string | null = null,
) {
  setDoc(
    produce((state) => {
      const siblingsFor = (id: string): string[] | null => {
        const node = state.byId[id];
        if (!node) return null;
        if (node.parent === null) return state.pages.find((page) => page.name === node.page)?.roots ?? null;
        return state.byId[node.parent]?.children ?? null;
      };

      // Gather every removal before changing any array. This matters when a
      // visible selection crosses parent arrays: each root must leave its own
      // original array exactly once.
      const removals = ids.map((id) => {
        const siblings = siblingsFor(id);
        return siblings ? { siblings, index: siblings.indexOf(id) } : null;
      });
      if (removals.some((removal) => !removal || removal.index < 0)) return;

      // Re-read each index while removing: multiple selected roots can share a
      // sibling array, so their preflight indices shift after the first splice.
      // We have already established every membership above, before any mutation.
      for (const id of ids) {
        const siblings = siblingsFor(id)!;
        siblings.splice(siblings.indexOf(id), 1);
      }

      const destination = destinationParent === null
        ? state.pages.find((page) => page.name === destPage)?.roots
        : state.byId[destinationParent]?.children;
      if (!destination) return;
      const at = destinationIndex(state);
      if (at < 0) return;

      for (const id of ids) state.byId[id].parent = destinationParent;
      destination.splice(Math.min(at, destination.length), 0, ...ids);

      if (expandParent !== null) {
        const target = state.byId[expandParent];
        if (!target) return;
        // One raw rewrite plus the structure move, rather than the old
        // writeCollapsed() publication after every selected root had moved.
        target.raw = rawWithCollapsed(target.raw, false, formatForBlock(expandParent));
        target.collapsed = false;
      }
    })
  );
  markDirty(destPage);
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
  if (!same.length || !canBatchMoveSelectionRoots(same, destPage, newParent)) return;
  pushUndo("indent-sel", [destPage]);
  moveSelectionRootsInOneMutation(
    same,
    destPage,
    newParent,
    (state) => state.byId[newParent].children.length,
    newParent,
  );
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
  if (!same.length || !canBatchMoveSelectionRoots(same, destPage, grand)) return;
  if (!rootsOf(parentId).includes(parentId)) return;
  pushUndo("outdent-sel", [destPage]);
  moveSelectionRootsInOneMutation(
    same,
    destPage,
    grand,
    (state) => {
      const siblings = grand === null
        ? state.pages.find((page) => page.name === destPage)?.roots
        : state.byId[grand]?.children;
      const parentIndex = siblings?.indexOf(parentId) ?? -1;
      return parentIndex < 0 ? -1 : parentIndex + 1;
    },
  );
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

interface RelativeMovePlan {
  roots: string[];
  sourcePages: string[];
  sourcePageByRoot: string[];
  destinationPage: string;
}

/** Build the complete target-relative move plan without mutating. Captured IDs
 * are stable-deduped, then descendants of another captured ID are subsumed. */
function relativeMovePlan(capturedIds: readonly string[], targetId: string): RelativeMovePlan | null {
  const unique = [...new Set(capturedIds)];
  if (!unique.length || unique.some((id) => !doc.byId[id])) return null;
  const captured = new Set(unique);
  const roots: string[] = [];

  for (const id of unique) {
    const seen = new Set([id]);
    let parent = doc.byId[id].parent;
    let subsumed = false;
    while (parent !== null) {
      if (seen.has(parent)) return null;
      seen.add(parent);
      if (captured.has(parent)) {
        subsumed = true;
        break;
      }
      const ancestor = doc.byId[parent];
      if (!ancestor) return null;
      parent = ancestor.parent;
    }
    if (!subsumed) roots.push(id);
  }
  if (!roots.length) return null;

  const target = doc.byId[targetId];
  if (!target || !pageWritable(target.page)) return null;
  const destinationParent = target.parent;
  if (destinationParent !== null) {
    const parent = doc.byId[destinationParent];
    if (!parent || parent.page !== target.page || !blockWritable(destinationParent)) return null;
  }
  const targetSiblings = destinationParent === null
    ? pageByName(target.page)?.roots
    : doc.byId[destinationParent]?.children;
  if (!targetSiblings || targetSiblings.filter((id) => id === targetId).length !== 1) return null;

  const moved = new Set<string>();
  const visit = (id: string, page: string, ancestry: Set<string>): boolean => {
    const node = doc.byId[id];
    if (!node || node.page !== page || moved.has(id) || ancestry.has(id)) return false;
    moved.add(id);
    const childSet = new Set(node.children);
    if (childSet.size !== node.children.length) return false;
    const nextAncestry = new Set(ancestry).add(id);
    return node.children.every((childId) => {
      const child = doc.byId[childId];
      return !!child && child.parent === id && visit(childId, page, nextAncestry);
    });
  };

  const sourcePageByRoot: string[] = [];
  for (const id of roots) {
    const node = doc.byId[id];
    if (!blockWritable(id)) return null;
    const siblings = node.parent === null
      ? pageByName(node.page)?.roots
      : doc.byId[node.parent]?.children;
    if (!siblings || siblings.filter((sibling) => sibling === id).length !== 1) return null;
    if (node.parent !== null) {
      const parent = doc.byId[node.parent];
      if (!parent || parent.page !== node.page) return null;
    }
    if (!visit(id, node.page, new Set())) return null;
    sourcePageByRoot.push(node.page);
  }
  if (moved.has(targetId)) return null;

  const sourcePages = [...new Set(sourcePageByRoot)];
  if (sourcePages.some((page) => !pageWritable(page))) return null;
  return {
    roots,
    sourcePages,
    sourcePageByRoot,
    destinationPage: target.page,
  };
}

/** Move captured selection roots together before/after a live target ID. This is
 * intentionally separate from same-page selection indent/outdent and never loops
 * over moveBlock: arbitrary source sibling arrays and pages form one transaction. */
export async function moveBlocksRelative(
  capturedIds: readonly string[],
  targetId: string,
  position: "before" | "after",
): Promise<boolean> {
  let plan = relativeMovePlan(capturedIds, targetId);
  if (!plan) return false;

  const crossSources = plan.sourcePages.filter((page) => page !== plan!.destinationPage);
  if (crossSources.length) {
    if (!(await prepareCrossPageSources(crossSources))) {
      pushToast("Couldn't move — a source page has unsaved changes that need resolving first.", "error");
      return false;
    }
    const rebuilt = relativeMovePlan(capturedIds, targetId);
    if (!rebuilt) return false;
    // Every non-destination source in the rebuilt plan must be one we flushed
    // while it still contained its roots. A concurrent cross-page reparent is a
    // safe abort, not permission to mutate a newly unprepared source.
    const rebuiltCross = rebuilt.sourcePages.filter((page) => page !== rebuilt.destinationPage);
    if (rebuilt.destinationPage !== plan.destinationPage
      || rebuilt.roots.length !== plan.roots.length
      || rebuilt.roots.some((id, index) => id !== plan!.roots[index])
      || rebuilt.sourcePageByRoot.some((page, index) => page !== plan!.sourcePageByRoot[index])
      || rebuiltCross.length !== crossSources.length
      || rebuiltCross.some((page, index) => page !== crossSources[index])) return false;
    plan = rebuilt;
  }

  const destinationFormat = formatForPage(plan.destinationPage);
  const movedRaw = new Map(plan.roots.map((id) => {
    const sourceRaw = doc.byId[id].raw;
    const raw = orderListTypeFromRaw(sourceRaw, formatForBlock(id)) !== null
      ? sourceRaw
      : rawWithInheritedOrderListType(sourceRaw, destinationFormat, targetId);
    return [id, raw];
  }));
  const affectedPages = [...new Set([plan.destinationPage, ...plan.sourcePages])];
  pushUndo("move-selection-relative", affectedPages);
  setDoc(produce((state) => {
    const siblingsFor = (id: string): string[] => {
      const node = state.byId[id];
      return node.parent === null
        ? state.pages.find((page) => page.name === node.page)!.roots
        : state.byId[node.parent].children;
    };
    for (const id of plan!.roots) {
      const siblings = siblingsFor(id);
      siblings.splice(siblings.indexOf(id), 1);
    }

    const target = state.byId[targetId];
    const destination = target.parent === null
      ? state.pages.find((page) => page.name === target.page)!.roots
      : state.byId[target.parent].children;
    const targetIndex = destination.indexOf(targetId);
    for (const id of plan!.roots) {
      state.byId[id].parent = target.parent;
      state.byId[id].raw = movedRaw.get(id)!;
    }
    destination.splice(targetIndex + (position === "after" ? 1 : 0), 0, ...plan!.roots);

    const reassign = (id: string) => {
      state.byId[id].page = plan!.destinationPage;
      for (const child of state.byId[id].children) reassign(child);
    };
    for (const id of plan!.roots) reassign(id);
  }));

  const persistenceSources = plan.sourcePages.filter((page) => page !== plan!.destinationPage);
  if (persistenceSources.length) persistCrossPage(plan.destinationPage, persistenceSources);
  else markDirty(plan.destinationPage);
  return true;
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
  const ended = !v && blockMovingPage !== null;
  blockMovingPage = v ? (page ?? blockMovingPage ?? "") : null;
  setBlockMoveRev((n) => n + 1);
  // A move in progress makes `reloadDisposition` return "skip", so it refuses
  // replacement exactly like a dirty page does — but unlike every other refusal
  // it announced nothing when it ended. A deferred stamp whose read landed
  // during an unrelated drag then waited for a coincidental later sweep to
  // resume, which may never come. Every state that can REFUSE has to announce
  // when it stops refusing. (GH #254 increment 3, round 13.)
  if (ended) sweepReplaceable();
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
    const pages = beginOrContinueMoveSelectionUndo(ids);
    if (!pages) return;
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
  // This route awaits source durability and has its own cross-page snapshot;
  // never let it borrow an in-page burst's inverse across that await.
  endMoveSelectionBurst();
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

/** Expand every collapsed ancestor of `id` so the block itself can render, as
 *  one undo step. Returns true if anything changed.
 *
 *  Needed because a collapsed parent does not render its children into the DOM
 *  at all (`Block.tsx`'s `<Show when={… && !collapsed()}>`), so "navigate to this
 *  block, scroll to it and highlight it" silently does nothing when the target is
 *  hidden — GH #258, reported against Ctrl+Shift+K block results.
 *
 *  The expansion is deliberately persistent, exactly like expanding by hand:
 *  `collapsed::` is on-disk state, and leaving the outline visually expanded but
 *  unsaved would revert under the user on the next load. One `withUndoUnit`
 *  keeps the whole chain a single Ctrl+Z. */
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
  withUndoUnit("reveal-block", [target.page], () => {
    for (const ancestor of collapsedAncestors) writeCollapsed(ancestor, false);
  });
  markDirty(target.page);
  return true;
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
