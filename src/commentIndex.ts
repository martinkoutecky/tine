// The one owner of comment membership for the trees a surface shows (vision
// §3.7; GH #891 review round 3, AR-15, PRINCIPLES 12). It answers: is block B a
// comment (its selector, its resolution)? which comments are thread roots, and
// so which are replies? which roots lie under subtree S? The resolution scope
// (the "N resolved" control), the folded "N below" count, and every block's
// comment children (body outline filter, quote highlights, popup, margin) read
// it; nothing else keeps comment membership.
//
// Maintained from deltas (the old `createCommentChildren` pattern, one level
// up): a structural change rescans the trees once with untracked reads (the
// scope's existing cost); a raw edit of any block on the trees' pages runs one
// `tine.quote` screen. A comment whose selector or resolution changed is
// updated in place and notifies only its parent's readers (and root readers if
// it is a root). A change of membership itself (an ordinary block gains a
// quote, a comment loses it) rescans, since it can turn roots into replies and
// back. Typing in an ordinary block therefore costs one screen, whatever the
// page holds, and subscribes nothing per block.

import { createContext, createMemo, createRoot, createSignal, onCleanup, untrack, useContext, type Accessor } from "solid-js";
import { formatForPage, node as docNode, onPageRawChanged, someBlockInTrees, threadRootOf } from "./document";
import { commentChildOf, NO_COMMENT_CHILDREN, sameChild, type CommentChild, type CommentChildren } from "./commentChildren";
import type { Format } from "./types";

export interface CommentIndex {
  /** Whether `id` lies in the indexed trees. O(depth), untracked. */
  covers(id: string): boolean;
  /** Comment children of `parentId` in file order; notifies only when that
   * list (membership, selector or resolution) changes. Call under an owner. */
  childrenOf(parentId: string): Accessor<CommentChildren>;
  /** Thread roots in file order; notifies on every rescan and root change. */
  roots(): readonly CommentChild[];
  /** Parents of thread roots, in source outline order. Shares the membership
   * scan; reading is O(1), including parents hidden by collapsed ancestors. */
  parents(): readonly string[];
  /** Open thread roots strictly below `id`'s direct children (a direct child's
   * thread is the block's own entry). 0 when `id` is itself in a thread, since
   * nothing below a comment is a root. O(roots × depth). */
  openBelow(id: string): number;
  /** The same roots as `openBelow`, in file order (the unread count reads their threads). */
  openRootsBelow(id: string): readonly string[];
}

interface Watcher { notify: () => void; value: Accessor<CommentChildren>; dispose: () => void; leases: number }

function sameEntries(a: readonly CommentChild[], b: readonly CommentChild[]): boolean {
  return a.length === b.length && a.every((entry, i) => entry === b[i]);
}
function childrenValue(entries: readonly CommentChild[] | undefined): CommentChildren {
  if (!entries?.length) return NO_COMMENT_CHILDREN;
  const ids = new Set(entries.map(entry => entry.id));
  return { entries, has: id => ids.has(id) };
}

export function createCommentIndex(trees: Accessor<readonly string[]>): CommentIndex {
  const [revision, bump] = createSignal(undefined, { equals: false });
  const [membership, remeasure] = createSignal(undefined, { equals: false });
  const [scanned, didScan] = createSignal(undefined, { equals: false });
  let comments = new Map<string, CommentChild>();
  let byParent = new Map<string, CommentChild[]>();
  let rootList: readonly CommentChild[] = [];
  let rootParents: readonly string[] = [];
  let treeRoots = new Set<string>();
  // Ancestors of the tree roots: their membership decides whether a tree root's
  // comments are roots or replies (a zoomed reply, a reference inside a thread).
  let above = new Map<string, boolean>();
  const watchers = new Map<string, Watcher>();
  onCleanup(() => { for (const watcher of watchers.values()) watcher.dispose(); });
  const formats = new Map<string, Format>();
  const formatOf = (page: string) => {
    let format = formats.get(page);
    if (!format) formats.set(page, format = formatForPage(page));
    return format;
  };
  const covers = (id: string) => untrack(() => {
    for (let at: string | null = id; at !== null; at = docNode(at)?.parent ?? null) if (treeRoots.has(at)) return true;
    return false;
  });
  // A stable scan result preserves Solid's freshness ordering without
  // invalidating unchanged child lists. Root readers track `scanned` instead.
  const scan = createMemo(() => {
    membership();
    const ids = trees();
    formats.clear();
    const previous = comments;
    const found = new Map<string, CommentChild>();
    const parents = new Map<string, CommentChild[]>();
    const positions = new Map<string, number>();
    // Reads plain nodes untracked; tracks the trees' pages and structural edits.
    someBlockInTrees(ids, n => {
      if (n.children.length) positions.set(n.id, positions.size);
      const entry = n.parent !== null ? commentChildOf(n.id, n.raw, formatOf(n.page)) : null;
      if (entry) {
        const old = previous.get(n.id);
        const kept = old && sameChild(old, entry) ? old : entry;
        found.set(n.id, kept);
        const siblings = parents.get(n.parent!);
        if (siblings) siblings.push(kept); else parents.set(n.parent!, [kept]);
      }
      return false;
    });
    untrack(() => {
      const pages = new Set<string>();
      const ancestors = new Map<string, boolean>();
      for (const id of ids) {
        const n = docNode(id);
        if (!n) continue;
        pages.add(n.page);
        for (let at = n.parent; at !== null && !ancestors.has(at); at = docNode(at)?.parent ?? null) {
          const a = docNode(at);
          ancestors.set(at, !!a && a.parent !== null && !!commentChildOf(at, a.raw, formatOf(a.page)));
        }
      }
      const previousParents = byParent;
      comments = found; byParent = parents; treeRoots = new Set(ids); above = ancestors;
      // The one root predicate (`threadRootOf`), asked once per comment.
      rootList = [...found.values()].filter(entry => threadRootOf(entry.id, formatOf(docNode(entry.id)!.page)) === entry.id);
      rootParents = [...new Set(rootList.map(entry => docNode(entry.id)!.parent!))]
        .sort((a, b) => positions.get(a)! - positions.get(b)!);
      for (const [parent, entries] of previousParents) {
        if (!sameEntries(entries, parents.get(parent) ?? [])) watchers.get(parent)?.notify();
      }
      for (const parent of parents.keys()) {
        if (!previousParents.has(parent)) watchers.get(parent)?.notify();
      }
      onCleanup(onPageRawChanged(changed => {
        const n = docNode(changed);
        const known = comments.get(changed);
        const next = n && n.parent !== null ? commentChildOf(changed, n.raw, formatOf(n.page)) : null;
        if (!known && !next && !above.get(changed)) return;
        if (!known) {
          // Membership changed in or above the trees: roots may now be replies, or back.
          if (above.has(changed) ? above.get(changed) !== !!next : covers(changed)) remeasure();
          return;
        }
        if (!next) { remeasure(); return; }
        if (sameChild(known, next)) return;
        comments.set(changed, next);
        const parentId = n!.parent!;
        byParent.set(parentId, byParent.get(parentId)!.map(entry => entry === known ? next : entry));
        watchers.get(parentId)?.notify();
        if (rootList.includes(known)) { rootList = rootList.map(entry => entry === known ? next : entry); bump(); }
      }, pages));
      didScan();
    });
  });

  /** Open thread roots strictly below `id`'s direct children, in file order. O(roots × depth). */
  const openRootsBelow = (id: string): readonly string[] => {
    scan(); scanned(); revision();
    return untrack(() => {
      const roots: string[] = [];
      for (const root of rootList) {
        if (root.resolved) continue;
        const parent = docNode(root.id)?.parent ?? null;
        if (parent === id) continue;
        for (let at = parent; at !== null; at = docNode(at)?.parent ?? null) if (at === id) { roots.push(root.id); break; }
      }
      return roots;
    });
  };

  return {
    covers,
    childrenOf(parentId) {
      let watcher = watchers.get(parentId);
      if (!watcher) {
        watcher = createRoot(dispose => {
          const [track, notify] = createSignal(undefined, { equals: false });
          const value = createMemo<CommentChildren>(previous => {
            scan(); track();
            const next = byParent.get(parentId) ?? [];
            return sameEntries(previous.entries, next) ? previous : childrenValue(next);
          }, NO_COMMENT_CHILDREN);
          return { notify, value, dispose, leases: 0 };
        });
        watchers.set(parentId, watcher);
      }
      const held = watcher;
      held.leases++;
      onCleanup(() => { if (--held.leases === 0 && watchers.get(parentId) === held) { watchers.delete(parentId); held.dispose(); } });
      // One leased producer per parent, shared by its outline and review views.
      return held.value;
    },
    roots: () => { scan(); scanned(); revision(); return rootList; },
    parents: () => { scan(); scanned(); return rootParents; },
    openBelow: id => openRootsBelow(id).length,
    openRootsBelow,
  };
}

/** The index of the enclosing resolution scope (a page, journal day, sidebar,
 * reference group or zoomed block). */
export const CommentIndexContext = createContext<CommentIndex>();

// Blocks outside every scope's trees (quick capture, an isolated test) share
// one index per top-level tree, released with its last reader.
const shared = new Map<string, { index: CommentIndex; dispose: () => void; leases: number }>();
function treeIndex(top: string): CommentIndex {
  let held = shared.get(top);
  if (!held) {
    held = createRoot(dispose => ({ index: createCommentIndex(() => [top]), dispose, leases: 0 }));
    shared.set(top, held);
  }
  const lease = held;
  lease.leases++;
  onCleanup(() => {
    if (--lease.leases > 0) return;
    lease.dispose();
    if (shared.get(top) === lease) shared.delete(top);
  });
  return lease.index;
}

const SCOPE = Symbol("scope");
/** The index that covers `id`: the enclosing scope's, else its tree's. Re-picked
 * when its ancestry changes (O(depth)); standalone blocks use their tree. */
export function createCommentIndexFor(id: Accessor<string | null>): Accessor<CommentIndex | null> {
  const scope = useContext(CommentIndexContext);
  const key = createMemo<string | typeof SCOPE | null>(() => {
    const at = id();
    if (at === null) return null;
    const pick = () => {
      let top = at;
      for (let parent = docNode(top)?.parent ?? null; parent !== null; parent = docNode(parent)?.parent ?? null) top = parent;
      return scope?.covers(at) ? SCOPE : top;
    };
    return scope ? pick() : untrack(pick);
  });
  return createMemo(() => {
    const k = key();
    return k === null ? null : k === SCOPE ? scope! : treeIndex(k);
  });
}

/** Which children of a block are comments, with their selectors and resolution:
 * a view of the covering index. Typing in an ordinary child costs one screen
 * and re-runs nothing here; a comment child's edit notifies only when its
 * selector or resolution changed. */
export function createCommentChildren(parentId: Accessor<string | null>): Accessor<CommentChildren> {
  const index = createCommentIndexFor(parentId);
  const view = createMemo(() => {
    const own = index();
    const id = parentId();
    return own && id !== null ? own.childrenOf(id) : () => NO_COMMENT_CHILDREN;
  });
  return () => view()();
}
