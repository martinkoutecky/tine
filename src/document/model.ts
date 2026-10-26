import { type PageKind, type Format } from "../types";
import { createStore, produce } from "solid-js/store";
import { batch, createRoot, createMemo, createSignal, untrack } from "solid-js";
import { acceptedBlockIdentityClaims, existingBlockId } from "../blockIdentity";
import { graphMeta } from "../graphSession";
import { sheetConfigFromRaw } from "../sheet/config";
import { beginResultsChange } from "../queryResultsEpoch";
import { clearOnBindingInvalidated } from "../binding";

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
  // journals feed). A subset of `pages`.
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
export const setDoc: typeof writeDoc = ((...args: unknown[]) => {
  const fieldOnly = args.length === 4 && ((args[0] === "byId" && ["raw", "loadedRaw", "collapsed"].includes(String(args[2])))
    || (args[0] === "pages" && ["preBlock", "title"].includes(String(args[2]))));
  batch(() => {
    const finish = hydrating || fieldOnly || args[0] === "feed" || args[0] === "loaded" ? null : beginResultsChange();
    try {
      (writeDoc as (...args: unknown[]) => void)(...args);
      untrack(() => {
        if (args.length === 4 && args[0] === "byId" && args[2] === "raw" && typeof args[1] === "string") {
          updateAuthoredIdentity(args[1]);
        } else if (!fieldOnly && args[0] !== "feed" && args[0] !== "loaded") {
          // Producers can replace/reparent/remove arbitrary nodes. Rebuild on
          // the next lookup, never on ordinary raw-only typing or folding.
          authoredIdentityPages.clear();
          setIdentityStructureRevision(v => v + 1);
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
// descendants batch). An embed occurrence keeps an ephemeral local fold for
// nested rows (GH #360); the fold records the epoch at fold time, and once the
// source writes again the epoch has moved, the fold is stale and the source
// reclaims authority. Never persisted; cleared with the working set.
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
  revision: () => number;
  changed: () => void;
}
const authoredIdentityPages = new Map<string, AuthoredIdentityPage>();
clearOnBindingInvalidated(() => authoredIdentityPages.clear());
const [identityStructureRevision, setIdentityStructureRevision] = createSignal(0);

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
  index.changed();
}

/** All reachable authored claimants, including duplicates. Reactive to ID and
 * structural changes; ordinary body edits update just one index entry. */
export function authoredBlockClaims(page: string, identity: string): ReadonlySet<string> | undefined {
  identityStructureRevision();
  let index = authoredIdentityPages.get(page);
  if (!index) {
    index = untrack(() => {
      const [revision, setRevision] = createSignal(0);
      const next: AuthoredIdentityPage = { byNode: new Map(), claims: new Map(), revision, changed: () => setRevision(v => v + 1) };
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
  index.revision();
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
  createMemo((): readonly ReadonlyFeedPage[] => {
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
export function pageByName(name: string): ReadonlyFeedPage | undefined {
  const i = pageIndexByName().get(name);
  return i === undefined ? undefined : doc.pages[i];
}

/** Record the identity chosen for a new page after its first successful save. */
export function setPageId(name: string, id: string): void {
  const i = pageIndexByName().get(name);
  if (i !== undefined) setDoc("pages", i, "id", id);
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
