import { msg } from "../../i18n";
import { revealOutlineBlock } from "../../outlineViewport";
import { prepareReferenceChangeNotice } from "./blocks";
import { existingBlockId } from "./identity";
import { captureReferenceChangeCount } from "../../toasts";
import { OutlineScope, depthOf, scopedVisibleOrder, visibleData, visibleOrder, pageVisibleOrder } from "../tree";
import { commentsFirst } from "../commentOrder";
import { doc, setDoc, bumpCollapseEpochs, writeRaw } from "../model";
import { applyOutlineChange, markOutlineDirty, noteOutlineRefusal, outlineChange, outlineChangePages, type OutlineChange } from "../kernel";
import { batch, createSignal, createRoot, createMemo, createComputed, getOwner, onCleanup, runWithOwner, untrack, type Accessor } from "solid-js";
import { ChangeKeyRegistry } from "../../changeKeyRegistry";
import { threadRootOf } from "../commentThreads";
import { previousBodySibling } from "../bodyStructure";
import { activeSurface, endEdit, editingId } from "../../editorController";
import { installClearOutlineSelection, notifyOutlineSelectionStarted } from "../../modeHooks";
import { clearOnBindingInvalidated } from "../../binding";
import { blockWritable, rawWithHeading, type HeadingState } from "./properties";
import { formatForBlock } from "../model";
import { pushUndo } from "../history";
import { produce } from "solid-js/store";
import { cycleMarkerSmart } from "../../editor/repeat";
import { workflow, timetrackingEnabled, logbookWithSecondSupport } from "../../ui";
import { markDirty } from "../host/wiring";
import { copyStripCollapsed } from "../../copySettings";
import { blockSubtreeMarkdown } from "./serialize";
import { pushToast } from "../../toasts";
import { activeDocument, queryAllWindows, requestFrame } from "../../windowRealm";

installClearOutlineSelection(() => clearSelection());
// I-20: selected ids name blocks of the bound graph; a same-id block in the next
// graph must not inherit the selection (Delete would remove it).
clearOnBindingInvalidated(() => clearSelection());

let activeSelectionScope: OutlineScope | null = null;
type SelectionSurface = { parent?: SelectionSurface };
// Ifods deliberately retires selection on every route change; Logseq comparison is in the receipt.
const selectionSurfaces = new Map<string, SelectionSurface>();
let selectedSurfaces: Set<SelectionSurface> | undefined;
// The surface key the current selection was started in: a selection the owner makes
// itself (the survivor of a delete) stays in it, never in a surface since unmounted.
let selectedSurfaceKey: string | undefined;

/** Own a rendered surface's selection lifetime. A changed route accessor or
 * unmount retires only selections initiated here or in a registered child
 * surface. Call under the component's Solid owner; `parent` is its enclosing
 * SurfaceContext key. Registration and retirement are O(1), plus existing
 * selection notifications. Starting selection carries its O(nesting) ancestry
 * once; no I/O or document/DOM scan is needed.
 * Route equality belongs to the supplied accessor (PageView uses sameRoute).
 * No failure or readiness state escapes to callers. */
export function ownSelectionSurface(surface: string, route: () => unknown = () => null, parent?: string): void {
  createComputed(() => {
    route();
    const owner: SelectionSurface = { parent: parent ? selectionSurfaces.get(parent) : undefined };
    selectionSurfaces.set(surface, owner);
    onCleanup(() => {
      if (selectionSurfaces.get(surface) === owner) selectionSurfaces.delete(surface);
      if (selectedSurfaces?.has(owner)) clearSelection();
    });
  });
}

export function selectionScope(): OutlineScope | null { return activeSelectionScope; }

/** Visible order to resolve a block SELECTION against. The journals feed lives in
 *  visibleData(); a routed single page is loaded via ensurePageLoaded and is NOT in
 *  doc.feed, so its blocks aren't in visibleOrder() — fall back to that block's own
 *  page order, mirroring prevVisible/nextVisible. Without this, block-select (Esc,
 *  Arrow, Shift+Arrow) is dead on any routed page / reference / embed. */
function selectionOrder(id: string | null, scope: OutlineScope | null = activeSelectionScope): string[] {
  if (!id) return [];
  const page = doc.byId[id]?.page;
  const order = scope ? scopedVisibleOrder(scope) : visibleData().index.has(id) ? visibleOrder() : page ? pageVisibleOrder(page) : [];
  return untrack(() => threadRootOf(id) === null ? order.filter(at => threadRootOf(at) === null) : order);
}

// ---------------------------------------------------------------------------
// Multi-block selection (Escape from editing; Shift+Arrows extend) + ops
// ---------------------------------------------------------------------------

const [selAnchor, setSelAnchor] = createSignal<string | null>(null);
const [selFocus, setSelFocus] = createSignal<string | null>(null);
const [selectionView, setSelectionView] = createSignal<{ id: string; scope: OutlineScope | null } | null>(null);

// View topology belongs to the selection, not its moving endpoints. Rebuild
// only when the view/tree changes; keypresses use its index and subtree ends.
let topologyGeneration = 0;
const selectionTopology = createRoot(() => createMemo(() => {
  const view = selectionView();
  const order = view ? selectionOrder(view.id, view.scope) : [];
  const index = new Map<string, number>();
  for (let at = 0; at < order.length; at++) index.set(order[at], at);
  const end = order.map((_, at) => at);
  const previous: number[] = [];
  const stack: number[] = [];
  const stacked = new Map<string, number>();
  const depths = order.map(id => depthOf(id));
  for (let at = 0; at < order.length; at++) {
    let ancestor: number | undefined;
    for (let parent = doc.byId[order[at]]?.parent; parent; parent = doc.byId[parent]?.parent) {
      ancestor = stacked.get(parent);
      if (ancestor !== undefined) break;
    }
    while (stack.length > (ancestor ?? -1) + 1) {
      const popped = stack.pop()!;
      end[popped] = at - 1;
      stacked.delete(order[popped]);
    }
    stacked.set(order[at], stack.length);
    stack.push(at);
    let prev = at - 1;
    while (prev >= 0 && depths[prev] > depths[at]) {
      const parent = doc.byId[order[prev]]?.parent;
      const parentAt = parent ? index.get(parent) : undefined;
      if (parentAt === undefined) break;
      prev = parentAt;
    }
    previous.push(prev);
  }
  for (const at of stack) end[at] = order.length - 1;
  return { order, index, end, previous, generation: ++topologyGeneration,
    duplicates: index.size !== order.length };
}));

// An ordinary outline selection is one interval in visible preorder.
// Keep that interval, rather than enumerating its old contents on every key.
// Repeated query rows need an explicit union of their subtrees; distinct query
// rows use the same interval and ancestor membership as an ordinary outline.
const selectionState = createRoot(() => createMemo(() => {
  const a = selAnchor();
  const f = selFocus();
  const topology = selectionTopology();
  const { order, index, end, duplicates } = topology;
  const i = a ? index.get(a) : undefined;
  const j = f ? index.get(f) : undefined;
  let lo = 0, hi = -1;
  if (i !== undefined && j !== undefined) {
    lo = Math.min(i, j);
    let last = order[Math.max(i, j)];
    // An endpoint can be inside a parent the range already selected. Complete
    // that outermost selected parent too, in either direction (GH #262/#941).
    for (let parent = doc.byId[last]?.parent; parent; parent = doc.byId[parent]?.parent) {
      const at = index.get(parent);
      if (at === undefined || at < Math.min(i, j) || at > Math.max(i, j)) break;
      last = parent;
    }
    hi = end[index.get(last)!];
  }
  if (!duplicates) return { topology, lo, hi, exact: null };
  // O(view rows in range): keep the selected roots only. Membership is an
  // ancestor walk and the descendant list is materialized on demand, so an
  // endpoint step never pays for folded subtrees.
  const roots: string[] = [];
  const rootSet = new Set<string>();
  if (i !== undefined && j !== undefined) {
    for (let at = Math.min(i, j); at <= Math.max(i, j); at = end[at] + 1) {
      if (doc.byId[order[at]] && !rootSet.has(order[at])) { rootSet.add(order[at]); roots.push(order[at]); }
    }
  }
  // Query rows may list a child before its (folded) parent: keep only roots no
  // selected ancestor already carries, so each block is emitted once.
  const covered = (id: string) => {
    for (let p = doc.byId[id]?.parent; p; p = doc.byId[p]?.parent) if (rootSet.has(p)) return true;
    return false;
  };
  if (roots.some(covered)) {
    const kept = roots.filter((id) => !covered(id));
    rootSet.clear();
    for (const id of kept) rootSet.add(id);
    roots.splice(0, roots.length, ...kept);
  }
  return { topology, lo, hi, exact: { roots, rootSet } };
}));

/** Subtree ids of `roots` in display preorder; stops and returns null once more
 *  than `limit` ids would be listed, so callers can bound their work. */
function subtreeIds(roots: readonly string[], limit = Infinity): string[] | null {
  const ids: string[] = [];
  const stack = [...roots].reverse();
  while (stack.length) {
    const id = stack.pop()!;
    if (!doc.byId[id]) continue;
    // Check the bound before touching the children: O(limit), never O(width).
    if (ids.push(id) + stack.length + doc.byId[id].children.length > limit) return null;
    const children = commentsFirst(doc.byId[id].children);
    for (let at = children.length - 1; at >= 0; at--) stack.push(children[at]);
  }
  return ids;
}

const selectionSnapshot = createRoot(() => createMemo(() => {
  const state = selectionState();
  const owner = getOwner();
  let ids: Accessor<string[]> | undefined, roots: string[] | undefined;
  const selectedRoots = () => roots ??= selectionRoots(state)!;
  return {
    ids: () => (ids ??= runWithOwner(owner, () => createMemo(() => subtreeIds(selectedRoots())!))!)(),
    roots: selectedRoots,
  };
}));

function selectionRoots(state: ReturnType<typeof selectionState>, limit = Infinity, lo = state.lo, hi = state.hi): string[] | null {
  if (state.exact) return state.exact.roots.length > limit ? null : state.exact.roots;
  const roots: string[] = [];
  for (let at = lo; at <= hi; at = state.topology.end[at] + 1) {
    const id = state.topology.order[at];
    let covered = false;
    // A query can put a selected ancestor after its child. The ancestor
    // carries that child once, regardless of their display positions.
    for (let parent = doc.byId[id]?.parent; parent; parent = doc.byId[parent]?.parent) {
      const parentAt = state.topology.index.get(parent);
      if (parentAt !== undefined && parentAt >= lo && parentAt <= hi) { covered = true; break; }
    }
    if (covered) continue;
    if (roots.length === limit) return null;
    roots.push(id);
  }
  return roots;
}

/** Selected IDs in display preorder, including every selected parent's hidden
 * descendants. O(1) cached read; O(selected blocks) to materialize after an
 * endpoint change; an already materialized closure follows selected subtree
 * edits. Every selection operation consumes this same closure. */
export function selectedIds(): string[] {
  return selectionSnapshot().ids();
}
/** The lone selected subtree root, or null. O(1) cached read; materializing
 * changed roots costs O(selected roots), independently of descendant count. */
export function singleSelectedId(): string | null {
  const roots = selectionSnapshot().roots();
  return roots.length === 1 ? roots[0] : null;
}
const selectedMembership = new ChangeKeyRegistry();
function membership(state: ReturnType<typeof selectionState> | null, id: string): boolean {
  // No selection: answer in O(1) and read no ancestry, so ordinary rendering
  // neither walks nor subscribes to every row's parents.
  if (!state || (state.exact ? state.exact.rootSet.size === 0 : state.hi < state.lo)) return false;
  if (state.exact) {
    for (let p: string | null | undefined = id; p; p = doc.byId[p]?.parent) if (state.exact.rootSet.has(p)) return true;
    return false;
  }
  for (let p: string | null | undefined = id; p; p = doc.byId[p]?.parent) {
    const at = state.topology.index.get(p);
    if (at !== undefined && at >= state.lo && at <= state.hi) return true;
  }
  return false;
}
createRoot(() => {
  let previous: ReturnType<typeof selectionState> | null = null;
  createComputed(() => {
    const next = selectionState();
    // Notify only keys someone reads: walk the changed members or the registered
    // keys, whichever is fewer, so extending past a folded parent never pays for
    // its hidden descendants (advance() on an unread key is a no-op anyway).
    const prev = previous;
    const interval = prev && !prev.exact && !next.exact && prev.topology === next.topology;
    // O(1) visible intervals; enumerate source descendants only up to the reader count.
    const ranges: [number, number][] = interval ? [
      [prev.lo, Math.min(prev.hi, next.lo - 1)], [Math.max(prev.lo, next.hi + 1), prev.hi],
      [next.lo, Math.min(next.hi, prev.lo - 1)], [Math.max(next.lo, prev.hi + 1), next.hi]] : [];
    // Otherwise list both member sets, but give up (and walk the registered keys)
    // once that exceeds the registry: O(min(changed members, read keys)).
    const listed = (state: ReturnType<typeof selectionState> | null) => {
      const roots = state ? selectionRoots(state, selectedMembership.size) : [];
      return roots && subtreeIds(roots, selectedMembership.size);
    };
    let changedIds: Set<string> | null = new Set();
    if (interval) for (const [lo, hi] of ranges) {
      const roots = selectionRoots(next, selectedMembership.size, lo, hi);
      const ids = roots && subtreeIds(roots, selectedMembership.size);
      if (!ids || ids.length + changedIds.size > selectedMembership.size) { changedIds = null; break; }
      for (const id of ids) changedIds.add(id);
    }
    const prevIds = interval ? [] : listed(prev);
    const nextIds = interval || !prevIds ? [] : listed(next);
    batch(() => {
      if (!changedIds || !prevIds || !nextIds) {
        for (const id of [...selectedMembership.keys()]) {
          if (membership(prev, id) !== membership(next, id)) selectedMembership.advance(id);
        }
      } else if (interval) {
        for (const id of changedIds) if (membership(prev, id) !== membership(next, id)) selectedMembership.advance(id);
      } else {
        for (const id of prevIds) if (!membership(next, id)) selectedMembership.advance(id);
        for (const id of nextIds) if (!membership(prev, id)) selectedMembership.advance(id);
      }
    });
    previous = next;
  });
});
export function isSelected(id: string): boolean {
  selectedMembership.read(id);
  // Parent changes must update mounted descendants even when a tree mutation
  // already changed their old ancestry before the membership diff runs.
  return membership(untrack(selectionState), id);
}
// Hierarchical Ctrl/Cmd+A (GH #262): the ancestor whose visible subtree the
// current select-all sequence covers. Any other selection mutation resets it.
let selectAllHead: string | null = null;
// Remember crossed subtree boundaries so reversal can restore a nested anchor
// after an upward step selected its parent. Entries are indices in this topology.
let keyboardPath: number[] = [];
let keyboardTopology: number | null = null;

export function selectBlock(id: string, scope: OutlineScope | null = null, surface = activeSurface() ?? "main") {
  const owner = selectionSurfaces.get(surface);
  endEdit("select-block");
  notifyOutlineSelectionStarted(id);
  if (owner && selectionSurfaces.get(surface) !== owner) return;
  selectedSurfaces = owner ? new Set() : undefined;
  selectedSurfaceKey = owner ? surface : undefined;
  for (let selected = owner; selected; selected = selected.parent) selectedSurfaces!.add(selected);
  activeSelectionScope = scope;
  selectAllHead = null;
  keyboardPath = [];
  keyboardTopology = null;
  batch(() => { setSelectionView({ id, scope }); setSelAnchor(id); setSelFocus(id); });
}
export function clearSelection() {
  selectedSurfaces = undefined;
  selectedSurfaceKey = undefined;
  batch(() => { setSelAnchor(null); setSelFocus(null); setSelectionView(null); });
  activeSelectionScope = null;
  selectAllHead = null;
  keyboardPath = [];
  keyboardTopology = null;
}
/** Extend the current block selection's focus to `id` (mouse-drag / shift-click).
 *  Starts a fresh selection anchored at `id` if none is active. */
export function extendSelectionTo(id: string, scope: OutlineScope | null = activeSelectionScope) {
  notifyOutlineSelectionStarted(id);
  if (selAnchor() === null) {
    activeSelectionScope = scope;
    setSelectionView({ id, scope });
    setSelAnchor(id);
  }
  if (scope !== activeSelectionScope || !selectionTopology().index.has(id)) return;
  selectAllHead = null;
  keyboardPath = [];
  setSelFocus(id);
}
export function hasSelection(): boolean {
  return selAnchor() !== null;
}
export function moveSelection(dir: 1 | -1, extend: boolean) {
  const f = selFocus();
  if (!f) return;
  const topology = selectionTopology();
  const { order, index, end, previous } = topology;
  const i = index.get(f);
  if (i === undefined) return;
  // D-20261112-SU10 makes both directions sibling-subtree steps. Logseq's
  // select-block-up-down/get-next-block-non-collapsed-skip supplies the downward
  // skip; upward promotion also avoids stopping inside the preceding subtree.
  if (keyboardTopology !== topology.generation || keyboardPath.at(-1) !== i) keyboardPath = [i];
  keyboardTopology = topology.generation;
  const prior = keyboardPath.at(-2);
  const reversing = extend && prior !== undefined && Math.sign(i - prior) !== dir;
  // Downward, skip the subtree of the outermost selected ancestor of the focus
  // too (Logseq get-next-block-non-collapsed-skip): Shift+click into a selected
  // parent, then Shift+Down, continues after that parent.
  let skip = i;
  if (extend && !reversing && dir > 0) {
    const a = selAnchor();
    const ai = a ? index.get(a) ?? i : i;
    const lo = Math.min(i, ai), hi = Math.max(i, ai);
    // Only an ancestor inside the selected range counts: query rows may list a
    // folded parent after its child, and that parent is not selected.
    for (let p = doc.byId[f]?.parent; p; p = doc.byId[p]?.parent) {
      const at = index.get(p);
      if (at === undefined || at < lo || at > hi) break;
      skip = at;
    }
  }
  const ni = reversing ? prior : !extend ? i + dir : dir > 0 ? end[skip] + 1 : previous[i];
  if (ni < 0 || ni >= order.length) return;
  if (!extend) keyboardPath = [];
  else if (reversing) keyboardPath.pop();
  else keyboardPath.push(ni);
  const next = order[ni];
  selectAllHead = null;
  batch(() => {
    setSelFocus(next);
    if (!extend) setSelAnchor(next);
  });
  scrollBlockRowIntoView(next);
}

/** Ctrl/Cmd+A while editing a block, with the block's text already fully
 *  selected: escalate to a block selection covering the block's whole source
 *  subtree, and mark it the head of the hierarchy ladder (GH #262). */
export function selectBlockSubtree(id: string, scope: OutlineScope | null = null, surface?: string) {
  selectBlock(id, scope, surface);
  selectAllHead = id;
}

/** Repeated Ctrl/Cmd+A in block-selection mode: widen the selection one
 *  ancestor level at a time — subtree, parent subtree, …, the whole visible
 *  outline, where it stays (idempotent) — or start the ladder at the current
 *  selection's anchor when no select-all sequence is in progress (GH #262). */
export function expandBlockSelection() {
  keyboardPath = [];
  const a = selAnchor();
  const f = selFocus();
  if (!a || !f) return;
  const { order, index, end } = selectionTopology();
  if (order.length < 2) return;
  // Whole-outline selection is the top of the ladder; further presses no-op.
  if (a === order[0] && end[index.get(f)!] === order.length - 1) return;
  let head = selectAllHead;
  if (head === null || !index.has(head)) head = a;
  const idx = index.get(head);
  if (idx === undefined) return;
  // A head whose subtree is fully inside the current selection (exactly or
  // because the user extended past it) is covered: climb from there.
  if (a === head && end[index.get(f)!] >= end[idx]) {
    // This subtree is already fully selected: climb to its parent, or to the
    // whole outline when the head is already a root (within the active scope).
    const parent = doc.byId[head]?.parent ?? null;
    if (parent !== null && index.has(parent)) {
      head = parent;
    } else {
      batch(() => { setSelAnchor(order[0]); setSelFocus(order[order.length - 1]); });
      selectAllHead = null;
      return;
    }
  }
  batch(() => { setSelAnchor(head); setSelFocus(head); });
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
        // OG cycle-todos! saves through with-timetracking. Reuse the shared
        // old/new-raw transition without adding per-block undo or save work.
        const format = formatForBlock(id);
        writeRaw(state, id, cycleMarkerSmart(node.raw, workflow(), format, {
          format, enabled: timetrackingEnabled(), withSeconds: logbookWithSecondSupport(),
        }).raw);
      }
    })
  );
  // No block changes page: one independent edit per page (STEP3 §8).
  for (const page of pages) markDirty(page, "save-block");
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
  requestFrame(undefined, () => {
    const sel = typeof CSS !== "undefined" && CSS.escape ? CSS.escape(id) : id;
    revealOutlineBlock(id);
    const row = activeDocument().querySelector(`.ls-block[data-block-id="${sel}"] > .block-main`)
      ?? queryAllWindows(`.ls-block[data-block-id="${sel}"] > .block-main`);
    row?.scrollIntoView({ block: "nearest" });
  });
}

/** Selected subtree roots. O(1) cached read; O(selected roots) to materialize
 * after an endpoint/tree change, without enumerating their descendants. */
export function topSelected(): string[] {
  return selectionSnapshot().roots();
}

function selectionRemovalSurvivor(): string | null {
  const roots = topSelected();
  const first = roots[0];
  const last = roots.at(-1);
  if (!first || !last) return null;
  const { order, index, end } = selectionTopology();
  return order[end[index.get(last)!] + 1] ?? order[index.get(first)! - 1] ?? doc.byId[first]?.parent ?? null;
}

function reselectSurvivingBlock(id: string | null, surface: string | undefined) {
  // An unowned selection (no registered surface) reselects as before: `undefined`
  // takes selectBlock's default surface.
  if (id && doc.byId[id]) selectBlock(id, null, surface);
  else clearSelection();
}

/** Run a selection's outline change: one undo entry, one store mutation, one
 *  save. A refusal stops before the undo entry. */
function commitSelectionChange(change: OutlineChange, undoKind: string): void {
  pushUndo(undoKind, outlineChangePages(change));
  applyOutlineChange(change);
  markOutlineDirty(change);
}

export function indentSelection() {
  const ids = topSelected();
  if (!ids.length || ids.some((id) => !blockWritable(id))) return;
  const first = ids[0];
  const newParent = previousBodySibling(first);
  if (!newParent) return;
  // A view rule: inside a scope, the new parent must be visible there.
  if (activeSelectionScope && !scopedVisibleOrder(activeSelectionScope).includes(newParent)) return;
  // Structural indent is single-page ONLY. The target (newParent) is on first's
  // page; moving a block from another feed day under it would be a cross-page
  // structural move (removal-before-add hazard) — and indenting under a different
  // day's block is nonsensical anyway. So move only the selected blocks that are
  // already on the target page.
  const destPage = doc.byId[newParent].page;
  const same = ids.filter((id) => doc.byId[id]?.page === destPage);
  // The kernel refuses a page-header parent (GH #638), a cycle and the cap.
  const change = outlineChange({ verb: "indent", keys: same }, [destPage]);
  if ("refused" in change) {
    if (change.refused === "depth_cap") pushToast(msg("selection-outline-is-too-deep-to-indent"), "error");
    else noteOutlineRefusal(change.refused, "indent");
    return;
  }
  commitSelectionChange(change, "indent-sel");
  bumpCollapseEpochs([newParent]);
}

export function outdentSelection() {
  const ids = topSelected();
  if (!ids.length || ids.some((id) => !blockWritable(id))) return;
  const parentId = doc.byId[ids[0]].parent;
  if (parentId === null) return;
  if (activeSelectionScope?.forceExpandedRoot === parentId) return;
  // Single-page only (see indentSelection): outdent moves blocks to the
  // grandparent, on ids[0]'s page — so restrict to the blocks already on that
  // page. A selection outdents its blocks alone (`logical`).
  const destPage = doc.byId[parentId].page;
  const same = ids.filter((id) => doc.byId[id]?.page === destPage);
  const change = outlineChange({ verb: "outdent", keys: same, logical: true }, [destPage]);
  if ("refused" in change) return;
  commitSelectionChange(change, "outdent-sel");
}

export function deleteSelection() {
  const survivor = selectionRemovalSurvivor();
  const surface = selectedSurfaceKey;
  const ids = topSelected().filter((id) => doc.byId[id]);
  if (!ids.length) return;
  const pages = [...new Set(ids.map((id) => doc.byId[id].page))];
  // One change for the whole selection — deleting each block separately fires a
  // reactive update per block (15 reflows for 15 bullets); one mutation
  // collapses it to a single update so the cut feels instant.
  const change = outlineChange({ verb: "delete", keys: ids }, pages);
  if ("refused" in change) return;
  const notice = prepareReferenceChangeNotice(ids, true);
  pushUndo(notice.tag, pages);
  applyOutlineChange(change);
  const ed = editingId();
  if (ed && !doc.byId[ed]) endEdit("delete-selection");
  markOutlineDirty(change);
  reselectSurvivingBlock(survivor, surface);
  notice.show();
}

/** GH #764: merge the selected blocks into the first one. Their texts follow in
 *  source preorder, including folded descendants, one line break between them;
 *  the first block keeps its place, its identity and its properties. One
 *  kernel verb (`merge_many`), one undo step, the ordinary save path. The kernel
 *  refuses, changing nothing, when a merged block's referenced `id::` would be
 *  lost (the first block already has one, or an earlier merged block's moved
 *  there), across pages, past the depth cap, and on a read-only page. A caller
 *  vets what a merge must not swallow (queries, embeds, sheets, highlights).
 *  Returns whether anything merged; the merged block stays selected. */
export function mergeSelection(): boolean {
  const ids = selectedIds().filter((id) => doc.byId[id]);
  if (ids.length < 2) {
    pushToast(msg("selection-merge-needs-two-blocks"), "warn");
    return false;
  }
  const scope = activeSelectionScope;
  const surface = selectedSurfaceKey;
  // A result list orders blocks by the query, not by the outline.
  if (scope?.navOnly) {
    pushToast(msg("selection-merge-not-in-results"), "warn");
    return false;
  }
  const page = doc.byId[ids[0]].page;
  if (ids.some((id) => doc.byId[id].page !== page)) {
    pushToast(msg("selection-merge-one-page-only"), "warn");
    return false;
  }
  if (ids.some((id) => !blockWritable(id))) {
    pushToast(msg("selection-merge-read-only"), "warn");
    return false;
  }
  const [survivor, ...absorbed] = ids;
  // The same reference test as Backspace: an unknown count counts as referenced.
  const referenced = absorbed.filter((id) => {
    const durable = existingBlockId(doc.byId[id].raw, formatForBlock(id));
    if (!durable) return false;
    const count = captureReferenceChangeCount([durable]);
    return typeof count !== "number" || count > 0;
  });
  const change = outlineChange({ verb: "merge_many", survivor, absorbed, referenced }, [page]);
  if ("refused" in change) {
    if (change.refused === "depth_cap") pushToast(msg("blocks-outline-is-too-deep-to-merge"), "error");
    else if (change.refused === "referenced_identity") pushToast(msg("selection-merge-would-break-a-reference"), "error");
    else if (change.refused === "cross_page_merge") pushToast(msg("selection-merge-one-page-only"), "warn");
    else pushToast(msg("selection-merge-refused"), "error");
    return false;
  }
  const transferred = change.notices.some((n) => n.kind === "id_transferred");
  const notice = prepareReferenceChangeNotice(absorbed, false, transferred);
  pushUndo(notice.tag, outlineChangePages(change));
  applyOutlineChange(change);
  markOutlineDirty(change);
  selectBlock(survivor, scope, surface);
  notice.show();
  return true;
}

/** Public clipboard text for the selected roots, including folded descendants.
 * O(selected subtree bytes). Identity is stripped; collapsed:: follows its preference. */
export function selectionMarkdown(): string {
  const stripCollapsed = copyStripCollapsed();
  return topSelected()
    .map((id) => blockSubtreeMarkdown(id, 0, true, stripCollapsed))
    .join("\n");
}

/** Apply a context heading command to the active selection, falling back to the
 * pointer block only when no selection is active (master 6eea5b70c, GH #240).
 * The preflight makes a mixed writable/read-only selection an exact no-op. */
export function setSelectionHeading(pointerId: string, state: HeadingState): boolean {
  const selected = selectedIds();
  const ids = selected.length ? selected : [pointerId];
  if (!ids.length || ids.some((id) => !blockWritable(id))) return false;
  const changes = ids
    .map((id) => ({ id, page: doc.byId[id].page, raw: rawWithHeading(doc.byId[id].raw, formatForBlock(id), state) }))
    .filter((change) => change.raw !== doc.byId[change.id].raw);
  if (!changes.length) return true;
  const pages = [...new Set(changes.map((change) => change.page))];
  pushUndo("heading-selection", pages);
  setDoc(produce((stateDoc) => {
    for (const change of changes) writeRaw(stateDoc, change.id, change.raw);
  }));
  // No block changes page: one independent edit per page (STEP3 §8).
  for (const page of pages) markDirty(page, "save-block");
  return true;
}
