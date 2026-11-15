import { type PageKind, type Format } from "../types";
import { createStore, produce, reconcile, unwrap } from "solid-js/store";
import { batch, createRoot, createMemo, createSignal, untrack } from "solid-js";
import { ChangeKeyRegistry } from "../changeKeyRegistry";
import { acceptedBlockIdentityClaims, existingBlockId } from "../blockIdentity";
import { graphMeta } from "../graphSession";
import { graphConfigUnreadable } from "../graphConfig";
import { ownerPath, resolvedTarget } from "../pageIndex";
import { page_file_format } from "../render/wasm/lsdoc_wasm.js";
import { sheetConfigFromRaw } from "../sheet/config";
import { beginResultsChange } from "../queryResultsEpoch";
import { clearOnBindingInvalidated } from "../binding";
import { flushFacetOwners } from "../render/seededFacets";
import { pageIdentityKey } from "../pageIdentity";
import { initParser, parserReady } from "../render/parse";

export interface Node {
  id: string;
  raw: string;
  /** Raw bytes from the loaded DTO; unchanged blocks keep their trailing space on save. */
  loadedRaw?: string;
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
  /** Concrete file identity returned by the backend; absent until first save. */
  id?: string;
}

export interface DocState {
  byId: Record<string, Node>;
  // The working set: every page currently loaded in the frontend — the main
  // view's pages PLUS any page a satellite surface (sidebar, query result,
  // embed) has pulled in on demand. All share one `byId` keyed by stable block
  // uuid, so a block rendered in two places is the SAME node and edits to it
  // propagate everywhere via SolidJS reactivity (OG's "everything is a block",
  // adapted to lazy loading — the Rust cache is the full graph DB).
  pages: FeedPage[];
  // Page names the MAIN content area shows, in order (a single page, or the
  // journals feed). Retired feed days retain only their name and UI geometry.
  feed: string[];
  loaded: boolean;
}

const [doc, writeDoc] = createStore<DocState>({ byId: {}, pages: [], feed: [], loaded: false });
export { doc };
/** Invalidate before structural working-set mutations, including producer-based
 * insert/delete/move/paste/undo and page replacement/rename. Raw-only typing and
 * folds leave mounted live objects intact; the host gate covers publication. */
let hydrating = 0;
/** Working-set hydration that cannot change what an existing live node means: a
 * first load adds new nodes, and eviction removes whole pages (their rows then
 * fail to resolve). Neither invalidates other results, so an open Table-cell
 * draft survives browsing. Replacing a loaded page is not hydration. */
export function hydrateWorkingSet<T>(write: () => T): T {
  hydrating++;
  try { return write(); } finally { hydrating--; }
}
// BIGLOAD-090b: the one per-block raw-change notification. A block's raw
// changes only through setDoc's field form or `writeRaw` inside a producer
// (rawFunnel.guard.test.ts), so an outline window keeps one listener, not one
// tracked computation per row. Replacing or removing a node is structure, which
// readers observe through `node(id)`.
const rawListeners = new Map<string, Set<(id: string) => void>>();
let producedRaw: string[] = [];
/** Calls `fn(id)` after each committed raw write to a member; returns the
 * unsubscribe. Owners replace their registration when membership changes. */
export function onBlockRawChanged(fn: (id: string) => void, ids: Iterable<string>): () => void {
  const members = new Set(ids);
  for (const id of members) {
    let listeners = rawListeners.get(id);
    if (!listeners) rawListeners.set(id, listeners = new Set());
    listeners.add(fn);
  }
  return () => {
    for (const id of members) {
      const listeners = rawListeners.get(id);
      listeners?.delete(fn);
      if (listeners?.size === 0) rawListeners.delete(id);
    }
  };
}
// The same notification for owners that follow every block of whole pages
// (comment membership, GH #891 review round 3): one registration per page
// instead of one per block, so a structural rescan re-registers nothing per row.
const pageRawListeners = new Map<string, Set<(id: string) => void>>();
/** Calls `fn(id)` after each committed raw write to a block of one of `pages`;
 * returns the unsubscribe. Same funnel and timing as `onBlockRawChanged`. */
export function onPageRawChanged(fn: (id: string) => void, pages: Iterable<string>): () => void {
  const members = new Set(pages);
  for (const page of members) {
    let listeners = pageRawListeners.get(page);
    if (!listeners) pageRawListeners.set(page, listeners = new Set());
    listeners.add(fn);
  }
  return () => {
    for (const page of members) {
      const listeners = pageRawListeners.get(page);
      listeners?.delete(fn);
      if (listeners?.size === 0) pageRawListeners.delete(page);
    }
  };
}
/** The only in-place raw assignment, for `setDoc(produce(...))` drafts. */
export function writeRaw(state: DocState, id: string, raw: string): void {
  state.byId[id].raw = raw;
  producedRaw.push(id);
}
export const setDoc: typeof writeDoc = ((...args: unknown[]) => {
  const fieldOnly = args.length === 4 && ((args[0] === "byId" && ["raw", "loadedRaw", "collapsed"].includes(String(args[2])))
    || (args[0] === "pages" && ["preBlock", "title"].includes(String(args[2]))));
  batch(() => {
    const finish = hydrating || fieldOnly || args[0] === "feed" || args[0] === "loaded" ? null : beginResultsChange();
    try {
      producedRaw = [];
      (writeDoc as (...args: unknown[]) => void)(...args);
      flushFacetOwners();
      const rawField = args.length === 4 && args[0] === "byId" && args[2] === "raw" && typeof args[1] === "string";
      const changedRaw = rawField ? [args[1] as string] : producedRaw;
      producedRaw = [];
      untrack(() => {
        for (const id of changedRaw) {
          for (const listener of rawListeners.get(id) ?? []) listener(id);
          const page = pageRawListeners.size ? doc.byId[id]?.page : undefined;
          if (page !== undefined) for (const listener of pageRawListeners.get(page) ?? []) listener(id);
        }
        if (rawField) {
          updateAuthoredIdentity(args[1] as string);
        } else if (!fieldOnly && args[0] !== "feed" && args[0] !== "loaded") {
          publishPageIndex();
          // Producers can replace/reparent/remove arbitrary nodes. Rebuild on
          // the next lookup, never on ordinary raw-only typing or folding.
          if (scopedStructurePages) {
            // GH #886: admitting, replacing or evicting whole pages changes only
            // those pages' trees. Clearing every page's index made each mounted
            // reference group rebuild its source's claims per page load, so a
            // scroll through N groups did N x N work.
            for (const page of scopedStructurePages) {
              authoredIdentityPages.delete(page);
              setPageStructureRevisions(page, (v) => (v ?? 0) + 1);
            }
          } else {
            authoredIdentityPages.clear();
            setIdentityStructureRevision(v => v + 1);
          }
        }
      });
    }
    finally {
      // A whole-state reset is complete here (workingSet.resetStore); edit
      // producers stay pending through their following markDirty call.
      if (finish) { if (typeof args[0] === "object") finish(); else queueMicrotask(finish); }
    }
  });
}) as typeof writeDoc;

// Monotonic per-block count of SOURCE collapse writes (writeCollapsed and the
// descendants batch). Query bulk-fold requests capture these generations so a
// delayed result cannot apply an old fold over a newer source change (GH #792).
// Never persisted; cleared with the working set. Embeds follow source folds.
const [collapseEpochState, setCollapseEpochState] = createStore<{ byId: Record<string, number> }>({ byId: {} });
export const collapseEpochOf = (id: string): number => collapseEpochState.byId[id] ?? 0;
export function bumpCollapseEpochs(ids: readonly string[]): void {
  setCollapseEpochState("byId", produce((epochs) => {
    for (const id of ids) epochs[id] = (epochs[id] ?? 0) + 1;
  }));
}
export const clearCollapseEpochs = (): void => setCollapseEpochState("byId", {});

export type ReadonlyNode = Readonly<Omit<Node, "children">> & { readonly children: readonly string[] };
export type ReadonlyFeedPage = Readonly<Omit<FeedPage, "roots">> & { readonly roots: readonly string[] };

/** Effective read-only presentation: the store's graph restriction plus the
 * page's intrinsic round-trip restriction. O(1), reactive, no page reload. */
export function pageReadOnly(page: ReadonlyFeedPage | undefined): boolean {
  return graphConfigUnreadable() || !!page?.readOnly;
}

/** Live document queries. Each call reads the Solid store in the caller's tracking scope. */
export function node(id: string): ReadonlyNode { return doc.byId[id]; }
export function childIds(id: string): readonly string[] { return doc.byId[id]?.children ?? []; }
export function pageRoots(name: string): readonly string[] { return pageByName(name)?.roots ?? []; }
export function loadedPage(name: string): ReadonlyFeedPage | undefined { return pageByName(name); }
export function feedNames(): readonly string[] { return doc.feed; }
export function isLoaded(name?: string): boolean { return name === undefined ? doc.loaded : !!pageByName(name); }

// GH #248: references and zoom routes need authored-ID precedence and ambiguity
// checks, but must not walk the page on every render/input. Index membership
// once, then maintain only the edited node's claim at the shared write boundary.
// This is scoped to reachable outline nodes, just like the former resolver walk
// (pre-blocks and orphan store entries are not authored claimants).
interface AuthoredIdentityPage {
  byNode: Map<string, string | null>;
  claims: Map<string, Set<string>>;
  changes: ChangeKeyRegistry;
}
const authoredIdentityPages = new Map<string, AuthoredIdentityPage>();
clearOnBindingInvalidated(() => authoredIdentityPages.clear());
const [identityStructureRevision, setIdentityStructureRevision] = createSignal(0);
const [pageStructureRevisions, setPageStructureRevisions] = createStore<Record<string, number | undefined>>({});
let scopedStructurePages: readonly string[] | null = null;
/** Run a setDoc write that adds, replaces or removes only the whole pages in
 * `pages` (read after the write, so a producer may fill the array as it goes).
 * Their authored-identity indexes rebuild; other pages' do not. Only page
 * admission, reload, forget and eviction use this: an edit producer can move
 * nodes between pages and keeps the global invalidation. */
export function writeWholePages<T>(pages: readonly string[], write: () => T): T {
  const outer = scopedStructurePages;
  scopedStructurePages = pages;
  try { return write(); } finally { scopedStructurePages = outer; }
}
/** Moves on every write that can change any page's structure (each setDoc write
 * except the raw/loadedRaw/collapsed/preBlock/title fields, feed and loaded),
 * except whole-page admission, reload, forget and eviction (`writeWholePages`),
 * which publish the affected pages' own structure revisions. */
export const structureRevision = (): number => identityStructureRevision();
/** Whether `test` holds for some block in the trees under `roots`; stops at the
 * first. Reads plain nodes untracked and tracks the roots' owning pages plus
 * unscoped structural edits, with no per-block subscriptions (BIGLOAD-090b). */
export function someBlockInTrees(roots: readonly string[], test: (n: ReadonlyNode) => boolean): boolean {
  structureRevision();
  const pages = untrack(() => new Set(roots.map(id => doc.byId[id]?.page).filter((page): page is string => page !== undefined)));
  for (const page of pages) void pageStructureRevisions[page];
  return untrack(() => {
    const byId = unwrap(doc.byId);
    const visit = (ids: readonly string[]): boolean => {
      for (const id of ids) {
        const n = byId[id];
        if (n && (test(n) || visit(n.children))) return true;
      }
      return false;
    };
    return visit(roots);
  });
}

function updateAuthoredIdentity(id: string): void {
  const node = doc.byId[id];
  const index = node && authoredIdentityPages.get(node.page);
  if (!index || !index.byNode.has(id)) return;
  const before = index.byNode.get(id)!;
  const after = existingBlockId(node.raw, formatForBlock(id));
  if (before === after) return;
  if (before !== null) {
    const claims = index.claims.get(before)!;
    claims.delete(id);
    if (claims.size === 0) index.claims.delete(before);
  }
  if (after !== null) {
    const claims = index.claims.get(after) ?? new Set<string>();
    claims.add(id);
    index.claims.set(after, claims);
  }
  index.byNode.set(id, after);
  if (before !== null) index.changes.advance(before);
  if (after !== null) index.changes.advance(after);
}

/** All reachable authored claimants, including duplicates. Reactive to ID and
 * structural changes; ordinary body edits update just one index entry. */
export function authoredBlockClaims(page: string, identity: string): ReadonlySet<string> | undefined {
  identityStructureRevision();
  void pageStructureRevisions[page];
  let index = authoredIdentityPages.get(page);
  if (!index) {
    index = untrack(() => {
      const next: AuthoredIdentityPage = { byNode: new Map(), claims: new Map(), changes: new ChangeKeyRegistry() };
      const stack = [...(pageByName(page)?.roots ?? [])];
      while (stack.length) {
        const id = stack.pop()!;
        if (next.byNode.has(id)) continue;
        const node = doc.byId[id];
        if (!node || node.page !== page) continue;
        const authored = existingBlockId(node.raw, formatForBlock(id));
        next.byNode.set(id, authored);
        if (authored !== null) {
          const claims = next.claims.get(authored) ?? new Set<string>();
          claims.add(id);
          next.claims.set(authored, claims);
        }
        stack.push(...node.children);
      }
      authoredIdentityPages.set(page, next);
      return next;
    });
  }
  index.changes.read(identity);
  return index.claims.get(identity);
}

// Retain claims with each live node, beyond the bounded render AST cache. A
// second paste over >8k loaded blocks must not reparse the entire working set.
const identityClaimsByNode = new WeakMap<Node, { raw: string; format: Format; ids: readonly string[] }>();

/** Which of `incomingIds` collide with a live identity in the loaded document
 *  (a runtime key or parser-accepted reserved id, case-insensitive)? ONE pass over the loaded
 *  document however many candidates are asked about — the per-id predicate this
 *  replaces re-scanned every key and raw for each candidate (master 8c495c1ce).
 *  This is the only answer to "is this id live?". */
export function loadedIdentityCollisions(incomingIds: readonly string[]): Set<string> {
  const collisions = new Set<string>();
  if (!incomingIds.length) return collisions;
  const loaded = new Set<string>();
  for (const [key, node] of Object.entries(doc.byId)) {
    if (node) loaded.add(key.toLowerCase());
    if (node) {
      const format = formatForPage(node.page);
      let claims = identityClaimsByNode.get(node);
      if (!claims || claims.raw !== node.raw || claims.format !== format) {
        claims = { raw: node.raw, format, ids: acceptedBlockIdentityClaims(node.raw, format) };
        identityClaimsByNode.set(node, claims);
      }
      for (const identity of claims.ids) loaded.add(identity.toLowerCase());
    }
  }
  for (const id of incomingIds) if (loaded.has(id.toLowerCase())) collisions.add(id);
  return collisions;
}

/** Does any candidate collide with a live identity, or with another candidate?
 *  A moved payload that repeats an id (redo history can carry preserved ids from
 *  an older snapshot) is refused just as a live collision is. */
export function hasLoadedIdentityCollision(incomingIds: readonly string[]): boolean {
  const seen = new Set<string>();
  for (const id of incomingIds) {
    const normalized = id.toLowerCase();
    if (seen.has(normalized)) return true;
    seen.add(normalized);
  }
  return loadedIdentityCollisions(incomingIds).size > 0;
}

export function docHasBlockIdentity(id: string): boolean {
  if (doc.byId[id]) return true;
  return loadedIdentityCollisions([id]).size > 0;
}


// name → index into `doc.pages`, rebuilt only when the working set's membership
// changes (add / remove / rename / evict), NOT on a keystroke. Turns the O(pages)
// linear `find` in `pageByName`/`formatForPage`/`mainPages` (per-block render hot
// path, ~7×/page render) into an O(1) lookup. Readers subscribe per name to a
// token naming that page's record, not to its position: evicting the oldest page
// shifts every later page's position, and position subscribers would all re-run,
// once per mounted reference group, on every eviction while scrolling (GH #886).
// Field reads (roots/preBlock/format) stay fine-grained on the record itself.
// Prefix keys so Solid's reserved property keys remain ordinary page names.
const recordTokens = new WeakMap<object, { token: number; name: string; key: string | null }>();
let nextRecordToken = 0;
const recordIdentity = (page: FeedPage) => {
  const raw = unwrap(page);
  let record = recordTokens.get(raw);
  if (!record || record.name !== page.name) {
    record = { token: record?.token ?? ++nextRecordToken, name: page.name, key: null };
    recordTokens.set(raw, record);
  }
  return record;
};
const [pageRecordByName, setPageRecordByName] = createStore<Record<string, number | undefined>>({});
let pagePositions: Record<string, number> = {};
let pageNamesByKey = new Map<string, string>();
let pageNamesByPath = new Map<string, string>();
let nativePageKeysReady = false;
/** Republish names, positions and record tokens. `setDoc` calls this inside the
 *  write's own batch, so a reader learns of a replaced or removed record in the
 *  same update as the rest of the write (a lagging memo let a block's memo run
 *  against its deleted node before the owning list disposed it). */
function publishPageIndex(): void {
  const positions: Record<string, number> = {};
  const tokens: Record<string, number> = {};
  const names = new Map<string, string>();
  const paths = new Map<string, string>();
  const nativeReady = parserReady();
  doc.pages.forEach((page, i) => {
    const key = `page:${page.name}`;
    const record = recordIdentity(page);
    positions[key] = i;
    tokens[key] = record.token;
    if (nativeReady) {
      record.key ??= pageIdentityKey(page.name);
      names.set(record.key, page.name);
    }
    if (page.id) paths.set(page.id, page.name);
  });
  pagePositions = positions;
  pageNamesByKey = names;
  pageNamesByPath = paths;
  nativePageKeysReady = nativeReady;
  setPageRecordByName(reconcile(tokens));
}

/** The loaded record named by `key`, subscribing only to that name's record. */
function keyedPage(key: string): FeedPage | undefined {
  void pageRecordByName[key];
  const i = pagePositions[key];
  return i === undefined ? undefined : untrack(() => doc.pages[i]);
}

/** The pages shown in the main content area, in feed order. Memoized: the O(feed)
 *  resolve runs once per structural change, not on each of its ~7 calls per render. */
export const mainPages = createRoot(() =>
  createMemo((): readonly ReadonlyFeedPage[] => {
    return doc.feed
      .map((n) => keyedPage(`page:${n}`))
      .filter(Boolean) as FeedPage[];
  })
);

/** A loaded page record by name (anywhere in the working set), or undefined. */
export function pageByName(name: string): ReadonlyFeedPage | undefined {
  return keyedPage(`page:${name}`);
}

/** Loaded targets of native publication keys, O(keys), with no page I/O.
 * Cold Capture can seed text before wasm: its first lookup awaits the parser
 * and derives those loaded names' keys once, O(loaded names). Warm membership
 * writes carry the cached native keys. Initialization failures reject. */
export async function readLoadedPublicationPages(keys: readonly string[]): Promise<ReadonlyFeedPage[]> {
  if (!keys.length || !doc.pages.length) return [];
  if (!nativePageKeysReady) {
    await initParser();
    publishPageIndex();
  }
  return keys.flatMap(key => {
    const name = pageNamesByKey.get(key);
    const page = name === undefined ? undefined : pageByName(name);
    return page ? [page] : [];
  });
}

/** Resolve a physical owner in the same loaded membership index, O(1). */
export function loadedPageByPath(path: string): ReadonlyFeedPage | undefined {
  const name = pageNamesByPath.get(path);
  return name === undefined ? undefined : pageByName(name);
}

/** Record the identity chosen for a new page after its first successful save. */
export function setPageId(name: string, id: string): void {
  const i = pagePositions[`page:${name}`];
  if (i !== undefined) setDoc("pages", i, "id", id);
}

/** Set loaded page `name`'s kind when it holds file `id` and differs. O(1). */
export function setPageKind(name: string, id: string, kind: PageKind): void {
  const i = pagePositions[`page:${name}`];
  if (i !== undefined && doc.pages[i].id === id && doc.pages[i].kind !== kind) setDoc("pages", i, "kind", kind);
}

/** Source format for a loaded page or its physical owner in the shared inventory.
 *  Only an unknown/new page falls back to the graph's preferred format. */
export function formatForPage(name: string | undefined): Format {
  if (name) {
    const p = pageByName(name);
    if (p?.format) return p.format;
    const path = ownerPath(resolvedTarget(name) ?? resolvedTarget(name, "journal"));
    if (path) return page_file_format(path) as Format;
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

// One opaque-sheet answer per live node and format, memoized on the node so a
// keystroke re-reads only the edited block's facets, and a reader (the
// feed-wide visible order) reruns only when the answer flips (og C, I-25). The
// memo tracks nothing but its node's `raw`, so an evicted or replaced node
// takes its memo with it: the WeakMap holds it by the node, and the unowned
// root registers it with no long-lived owner or signal.
const opaqueSheetMemo = new WeakMap<Node, { format: Format; isOpaque: () => boolean }>();

export function blockIsOpaqueSheetView(id: string | undefined): boolean {
  const n = id ? doc.byId[id] : undefined;
  if (!n) return false;
  const format = formatForBlock(id);
  let entry = opaqueSheetMemo.get(n);
  if (!entry || entry.format !== format) {
    const isOpaque = createRoot(() => createMemo(() => {
      const view = sheetConfigFromRaw(n.raw, format).view;
      return view === "grid" || view === "table" || view === "board";
    }));
    entry = { format, isOpaque };
    opaqueSheetMemo.set(n, entry);
  }
  return entry.isOpaque();
}

let idCounter = 0;
export function freshId(): string {
  return `b${Date.now().toString(36)}-${idCounter++}`;
}
/** The outline kernel's source of fresh keys: the same scheme as {@link freshId}.
 *  Advance it with {@link keysUsed} by the `Change`'s `keys_used`. */
export function keySource(): { prefix: string; next: number } {
  return { prefix: `b${Date.now().toString(36)}-`, next: idCounter };
}
export function keysUsed(count: number): void {
  idCounter += count;
}
