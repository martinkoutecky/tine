import { FeedPage, Node, doc, pageByName, setDoc, docHasBlockIdentity } from "./model";
import { addDirty, persistTogether, scheduleSave, type TransferEdge } from "./save/engine";
import { type Route } from "../routeTypes";
import { type HistorySidebarContext, captureHistorySidebarContext, restoreHistorySidebarContext } from "../ui";
import { type HistoryEditorContext, captureHistoryEditorContext, editingId, endEdit, restoreHistoryEditorContext } from "../editorController";
import { unwrap, produce } from "solid-js/store";
import { createSignal } from "solid-js";
import { purgePageNodes } from "./convert";
import { invalidateAllMatrixDimensions } from "../sheet/matrix";
import { pageWritable } from "./edits/properties";
import { graphRewriteFrozen } from "./graphRewriteState";
import { pushToast } from "../toasts";

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
  /** Identity-bearing clipboard paste whose redo must fail on a live conflict. */
  preservedIds?: string[];
  /** The `pushUndo` tag, so a caller can ask whether Undo would take back ITS change. */
  tag?: string;
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
  preservedIds?: string[];
}
type UndoEntry = SnapEntry | RawEntry;
const tagOf = (entry: UndoEntry | undefined): string | null => (entry && "tag" in entry ? entry.tag ?? null : null);
const undoStack: UndoEntry[] = [];
let redoStack: UndoEntry[] = [];
let lastUndoTag: string | null = null;
let undoSuppressionDepth = 0;
// Bumped on every stack change so `undoTopTag` is reactive (query crossing notice).
const [historyRev, setHistoryRev] = createSignal(0);
const bumpHistory = () => setHistoryRev((n) => n + 1);

/** The tag of the entry the ordinary Undo would take back next, or null. */
export function undoTopTag(): string | null {
  historyRev();
  if (!pageOnlyHistoryMode) return tagOf(undoStack[undoStack.length - 1]);
  const page = activeHistoryPage();
  for (let i = undoStack.length - 1; i >= 0; i--) {
    if (!page || entryTouchesPage(undoStack[i], page)) return tagOf(undoStack[i]);
  }
  return null;
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
  undoStack.length = 0;
  redoStack = [];
  lastUndoTag = null;
  bumpHistory();
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
  bumpHistory();
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
  return {
    kind: "snap",
    pages: affected ?? null,
    pageObjs,
    nodes,
    dirty: names,
    context,
    ...(preservedIds?.length ? { preservedIds: [...preservedIds] } : {}),
  };
}

/** Snapshot before a STRUCTURAL op. Pass the affected page name(s) so both the
 *  snapshot AND the undo re-save are scoped to just those pages; omit only when
 *  the op's page set isn't known (falls back to the whole working set — correct
 *  but O(loaded pages)). The affected set MUST include every page whose nodes the
 *  op changes, including a cross-page move's source AND destination, or undo
 *  would miss a page. `tag` resets the typing-coalesce marker. */
export function pushUndo(tag: string, affected?: string[], preservedIds?: readonly string[]) {
  if (undoSuppressionDepth > 0) return;
  undoStack.push({ ...snapEntry(affected, preservedIds), tag });
  if (undoStack.length > 200) undoStack.shift();
  redoStack = [];
  lastUndoTag = tag;
  bumpHistory();
}

/** Record an O(1) inverse patch for a single-block text edit (typing). A typing
 *  burst in one block coalesces to a single entry holding the pre-burst text. */
export function pushRawUndo(id: string, prevRaw: string) {
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
  bumpHistory();
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
      addDirty(e.page, "save-block");
    } else if (e.headerRoot) {
      const restored = { ...cloneNode(e.headerRoot.node), raw: e.raw };
      setDoc(produce((s) => {
        s.byId[e.id] = restored;
        const page = s.pages.find((p) => p.name === e.page);
        if (page) page.roots.splice(Math.min(e.headerRoot!.rootIndex, page.roots.length), 0, e.id);
      }));
      inverse.headerRoot = { node: cloneNode(restored), rootIndex: e.headerRoot.rootIndex };
      inverse.removeHeaderOnApply = true;
      addDirty(e.page, "save-block");
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
          if (i >= 0) {
            // Page views key their lifetime by this object: restore its complete
            // snapshot in place so undo/redo never unmounts an open editor or
            // query sheet (master 7fcd4c98d).
            const current = s.pages[i];
            for (const key of Object.keys(current)) {
              if (!Object.hasOwn(restored, key)) Reflect.deleteProperty(current, key);
            }
            Object.assign(current, restored);
          } else s.pages.push(restored);
        }
      })
    );
  }
  // Multi-page replay is registered as one group by undo/redo immediately after
  // applyEntry returns; a one-page replay needs only its ordinary dirty bit.
  if (e.dirty.length === 1) addDirty(e.dirty[0], "replace-page");
  invalidateAllMatrixDimensions();
  return inverse;
}

/** Run a synchronous edit as one undo step over `pages`, O(their blocks) to
 *  snapshot. Nested units fold into the outer unit. An exception restores the
 *  snapshot and stacks, then rethrows. A frozen rewrite or loaded read-only page
 *  skips `fn` and returns undefined; success-reporting callers must check it. */
export function withUndoUnit<T>(tag: string, pages: string[], fn: () => T): T {
  if (graphRewriteFrozen()) return undefined as T;
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
    bumpHistory();
    throw err;
  } finally {
    if (undoSuppressionDepth > 0) undoSuppressionDepth--;
  }
}

function transferOrder(entry: UndoEntry, inverse: UndoEntry): TransferEdge[] {
  if (entry.kind !== "snap" || inverse.kind !== "snap") return [];
  const transfers: TransferEdge[] = [];
  for (const [id, next] of Object.entries(entry.nodes)) {
    const previous = inverse.nodes[id];
    if (!previous || previous.page === next.page) continue;
    transfers.push([previous.page, next.page]);
  }
  return transfers;
}

/** Undo the selected global or page-scoped entry, restore its UI context and
 *  schedule affected pages for save. O(blocks of those pages). Returns false if
 *  empty or a graph rewrite is frozen. */
export function undo(): boolean {
  if (graphRewriteFrozen()) return false;
  const entry = popHistoryEntry(undoStack);
  if (!entry) return false;
  const inverse = applyEntry(entry);
  if (entry.kind === "snap" && entry.dirty.length > 1) void persistTogether(entry.dirty, "replace-page", transferOrder(entry, inverse));
  redoStack.push(inverse);
  lastUndoTag = null;
  bumpHistory();
  endEdit("undo");
  scheduleSave();
  restoreEntryContext(entry.context);
  return true;
}

/** Redo the selected entry unless empty or frozen. If it would recreate an id
 *  now present elsewhere, show an error and clear the redo stack. Otherwise
 *  restore its pages and UI context and schedule a save. */
export function redo() {
  if (graphRewriteFrozen()) return;
  const entry = popHistoryEntry(redoStack);
  if (!entry) return;
  if (entry.preservedIds?.some(docHasBlockIdentity)) {
    // The selected prerequisite is already popped. A later redo snapshot cannot
    // remain valid without it, including in page-only mode where the tagged
    // entry may have been selected from the middle of the global stack.
    redoStack = [];
    pushToast("Redo skipped: a block with the same id now exists", "error");
    return;
  }
  const inverse = applyEntry(entry);
  if (entry.kind === "snap" && entry.dirty.length > 1) void persistTogether(entry.dirty, "replace-page", transferOrder(entry, inverse));
  undoStack.push(inverse);
  lastUndoTag = null;
  bumpHistory();
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
