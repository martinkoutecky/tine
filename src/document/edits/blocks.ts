import { removeSubtree } from "./subtree";
import { blockRegions } from "../../render/parse";
import { blockWritable, pageWritable, rawWithInheritedOrderListType, isOrdered, rawWithOrderListType, rawWithCollapsed, writeCollapsed } from "./properties";
import { doc, docHasBlockIdentity, formatForBlock, setDoc, freshId, type DocState, formatForPage, pageByName } from "../model";

/** Reveal a search result without creating an edit, undo entry, or save. */
export function revealNode(id: string): void {
  if (doc.byId[id]?.collapsed) setDoc("byId", id, "collapsed", false);
}
import { applyMarkerTransition } from "../../logbook";
import { timetrackingEnabled, logbookWithSecondSupport, logicalOutdenting, removeDeletedBlocksFromSidebar } from "../../ui";
import { pushRawUndo, pushUndo, undo, undoTopTag } from "../history";
import { markDirty, noteTitleIdentityIntent } from "../host/wiring";
import { produce } from "solid-js/store";
import { OUTLINE_MAX_DEPTH, outlineDepth, type OutlineNode } from "../../editor/outline";
import { splitProps, isBuiltinHidden, joinProps, isPropertiesOnly, readPropertyValue } from "../../editor/properties";
import { startEditing, editingId, endEdit, type EditorSelection } from "../../editorController";
import { batch } from "solid-js";
import type { EditKind, EditKinds } from "../../editKind";
import { isBlockMoving, setBlockMoving } from "./moves";
import { depthOf, existingSubtreeFits, indexInSiblings, rootsOf, OutlineScope, prevVisible, nextVisible } from "../tree";
import { blockExternalId, existingBlockId, isBlockRefUuid } from "./identity";
import { captureReferenceChangeCount, pushReferenceChangeNotice, pushToast } from "../../toasts";
import { graphEpoch } from "../../graphSession";
import { blockRefsInText } from "../../render/pageRefs";

let referenceEditSeq = 0;
/** Capture the edit's targets before mutation, then publish against its existing
 * undo unit. Work is bounded by the removed subtree and its reference targets. */
export function prepareReferenceChangeNotice(roots: readonly string[], subtree: boolean, transferred = false): { tag: string; show: () => void } {
  const removed = new Set<string>();
  const visit = (id: string) => {
    if (removed.has(id) || !doc.byId[id]) return;
    removed.add(id);
    if (subtree) doc.byId[id].children.forEach(visit);
  };
  roots.forEach(visit);
  const targets = new Set([...removed].map((id) => blockExternalId(id)).filter((id): id is string => !!id && isBlockRefUuid(id)));
  // A reference removed with its source is not left broken. Each source counts
  // once per target, matching blockRefCounts (repeated ((id)) is one referrer).
  let removedReferences = 0;
  if (subtree) for (const id of removed) {
    for (const target of new Set(blockRefsInText(doc.byId[id].raw, formatForBlock(id)))) {
      if (targets.has(target)) removedReferences++;
    }
  }
  const count = captureReferenceChangeCount([...targets]);
  const epoch = graphEpoch();
  const tag = `reference-edit:${++referenceEditSeq}`;
  const showCount = (count: number | null | undefined) => {
    if (count === undefined) return; // A failed count read cannot prove any reference was affected.
    if (graphEpoch() !== epoch || undoTopTag() !== tag) return;
    pushReferenceChangeNotice(count === null ? null : Math.max(0, count - removedReferences), transferred, () => {
      if (graphEpoch() === epoch && undoTopTag() === tag) undo();
    });
  };
  return { tag, show: () => { if (count === undefined || count === null || typeof count === "number") showCount(count); else void count.then(showCount); } };
}

// ---------------------------------------------------------------------------
// Mutations (each schedules a debounced save of the affected page)
// ---------------------------------------------------------------------------

/** Keep imported properties except an id already owned by a live block. */
function outlineRaw(raw: string, format: "md" | "org", incoming: Set<string>): string {
  const id = existingBlockId(raw, format)?.toLowerCase();
  if (!id) return raw;
  if (incoming.has(id) || docHasBlockIdentity(id))
    return splitProps(raw, (key) => key.toLowerCase() === "id", format).visible;
  incoming.add(id);
  return raw;
}

/** Build imported nodes inside the existing transaction. The first root may
 * reuse an empty host's identity/hidden props; all other nodes get fresh ids. */
function createOutline(
  state: DocState, nodes: readonly OutlineNode[], parent: string | null,
  page: string, inheritFrom: string, incoming: Set<string>,
  reuse?: { id: string; hidden: string },
): string[] {
  const format = formatForPage(page);
  const create = (outline: OutlineNode, parent: string | null, host?: typeof reuse): string => {
    const id = host?.id ?? freshId();
    const children = outline.children.map((child) => create(child, id));
    const imported = outlineRaw(outline.raw, format, incoming);
    const source = host ? joinProps(imported, host.hidden, format) : imported;
    const raw = rawWithInheritedOrderListType(source, format, inheritFrom);
    state.byId[id] = { id, raw, collapsed: false, parent, page, children };
    return id;
  };
  return nodes.map((outline, index) => create(outline, parent, index === 0 ? reuse : undefined));
}

export function setRaw(id: string, raw: string, opts?: { timetracking?: boolean }) {
  if (!blockWritable(id)) return;
  const prev = doc.byId[id].raw;
  const page = pageByName(doc.byId[id].page);
  if (page?.kind === "page" && page.roots[0] === id
      && (doc.byId[id].originatedFromPageHeader || (!page.preBlock && isPropertiesOnly(prev)))
      && readPropertyValue(prev, "title") !== readPropertyValue(raw, "title")) {
    noteTitleIdentityIntent(page.name);
  }
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
  markDirty(doc.byId[id].page, "save-block");
}

export function insertEmptyChildBlock(parentId: string, at: number): string | null {
  const parent = doc.byId[parentId];
  if (!parent || !blockWritable(parentId) || at < 0 || at > parent.children.length) return null;
  if (!outlineFits(parentId, [{ raw: "", children: [] }], 1)) {
    pushToast("Outline is too deep to insert a child", "error");
    return null;
  }
  pushUndo(`insert-child:${parentId}`, [parent.page]);
  const id = freshId();
  const pageName = parent.page;
  setDoc(
    produce((s) => {
      s.byId[id] = { id, raw: "", collapsed: false, parent: parentId, page: pageName, children: [] };
      s.byId[parentId].children.splice(at, 0, id);
    })
  );
  markDirty(pageName, "insert-blocks");
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
      if (!existingSubtreeFits(childId, parentId)) {
        pushToast("Outline is too deep to move", "error");
        return false;
      }
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
  for (const pageName of pages) markDirty(pageName, "move-blocks");
  return true;
}

/** Whether an outline fits beside `hostId`, or `levelsBelowHost` levels below
 * it, under the shared depth ceiling. Bounds every recursive outline insert. */
export function outlineFits(hostId: string, nodes: readonly OutlineNode[], levelsBelowHost = 0): boolean {
  return !!doc.byId[hostId] && depthOf(hostId) + levelsBelowHost + outlineDepth(nodes) <= OUTLINE_MAX_DEPTH;
}

/** Append parsed outline blocks as children of `parentId`; null means empty,
 *  read-only, missing parent, or over-depth refusal. Shared by editor and sheet paste. */
export function insertOutlineChildren(parentId: string, nodes: OutlineNode[]): string | null {
  if (!nodes.length) return null;
  const parent = doc.byId[parentId];
  if (!parent || !blockWritable(parentId) || !outlineFits(parentId, nodes, 1)) return null;
  const pageName = parent.page;
  let lastId: string | null = null;
  pushUndo("paste-children", [pageName]);
  const incoming = new Set<string>();
  setDoc(
    produce((s) => {
      const created = createOutline(s, nodes, parentId, pageName, parentId, incoming);
      s.byId[parentId].children.push(...created);
      lastId = created[created.length - 1] ?? null;
    })
  );
  markDirty(pageName, "insert-blocks");
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
  const fmt = formatForBlock(id);
  // The caret offset is in editor-visible space (hidden props aren't shown), so
  // split the visible text and keep the hidden props on the original block.
  const { visible, hidden } = splitProps(node.raw, isBuiltinHidden, fmt);
  // GH #361: the caret can report either side of the same source-line boundary
  // (end of line one or start of line two). In both cases that newline becomes
  // the structural block separator instead of content in either block.
  const boundaryBefore = offset < visible.length && visible[offset] === "\n";
  const boundaryAfter = offset > 0 && visible[offset - 1] === "\n";
  const splitBefore = boundaryAfter ? offset - 1 : offset;
  const splitAfter = !boundaryAfter && boundaryBefore ? offset + 1 : offset;
  const before = visible.slice(0, splitBefore);
  const after = visible.slice(splitAfter);
  const childSplit = before.trim() === "" && after.trim() !== ""
    ? keepStartInScope
    : (node.children.length > 0 && !node.collapsed) || forceChild;
  if (childSplit && !outlineFits(id, [{ raw: "", children: [] }], 1)) {
    pushToast("Outline is too deep to insert a child", "error");
    return false;
  }
  pushUndo("split", [node.page]);
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
        // At offset zero the original block is untouched. At a later line
        // boundary, however, the blank prefix and its separator become the new
        // empty block, so the original must retain only the post-boundary text.
        if (offset > 0) s.byId[id].raw = joinProps(after, hidden, fmt);
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
    markDirty(pageName, offset > 0 ? ["insert-blocks", "save-block"] : "insert-blocks");
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
  markDirty(pageName, ["save-block", "insert-blocks"]);
}

/** Tab: make the block the last child of its previous sibling. Returns false
 *  when that would exceed the outline cap; the caller shows the refusal.
 *
 *  `editingSurface` names the surface the caret must stay on, exactly as the
 *  split/merge operations take it. Without it the caret leaves a block embed
 *  mid-keystroke: `editing()` in Block.tsx prefers the NON-embed rendering when
 *  no surface is named, so the editor remounts on the source copy of the same
 *  block further down the page (GH #477). */
export function indentBlock(id: string, caretOffset: number | EditorSelection, editingSurface: string | null = null) {
  if (!blockWritable(id)) return;
  const i = indexInSiblings(id);
  if (i <= 0) return;
  const sibs = rootsOf(id);
  const newParent = sibs[i - 1];
  // A materialized page preamble is not an outline parent (GH #638). Keep the
  // first body block a root; an ordinary imported properties block may nest.
  if (doc.byId[newParent].originatedFromPageHeader) return;
  if (!existingSubtreeFits(id, newParent)) return false;
  pushUndo("indent", [doc.byId[id].page]);
  const pageName = doc.byId[id].page;
  // Reparenting remounts the editor. Publish its selection and ownership in the
  // same reactive flush as the tree change, before the replacement can focus.
  reparentEditingBlock(pageName, ["move-blocks", "save-block"], () => {
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
    startEditing(id, caretOffset, null, editingSurface);
  });
}

/** Run a reparenting tree update and the editor handoff in one reactive batch,
 *  marked as a block move so the old editor's blur during the flush does not
 *  end edit mode (#519, #495). An already active move (another page) is left
 *  to its owner. */
function reparentEditingBlock(page: string, kinds: EditKind | EditKinds, update: () => void): void {
  const ownsMove = !isBlockMoving();
  if (ownsMove) setBlockMoving(true, page);
  try {
    batch(update);
    markDirty(page, kinds);
  } finally {
    if (ownsMove) setBlockMoving(false);
  }
}

/** Shift+Tab: move the block out to be the next sibling of its parent.
 *  `editingSurface` as in `indentBlock` (GH #477). */
export function outdentBlock(id: string, caretOffset: number | EditorSelection, editingSurface: string | null = null) {
  const node = doc.byId[id];
  if (!node || !blockWritable(id) || node.parent === null) return;
  pushUndo("outdent", [node.page]);
  const parentId = node.parent;
  const grandParent = doc.byId[parentId].parent;
  const pageName = node.page;

  reparentEditingBlock(pageName, "move-blocks", () => {
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
    startEditing(id, caretOffset, null, editingSurface);
  });
}

/** Backspace at offset 0: merge into the previous visible block (same page). */
export function mergeWithPrev(
  id: string,
  scope: OutlineScope | null = null,
  editingSurface: string | null = null,
): boolean {
  if (!blockWritable(id)) return false;
  // A navOnly display list (ref/query/embed group) is never a merge topology:
  // merging into a rendered neighbour could weld unrelated subtrees that merely
  // sit adjacent in the RESULT list. Fall back to page order.
  if (scope?.navOnly) scope = null;
  const prev = prevVisible(id, scope);
  if (prev === null) return false;
  return absorbInto(prev, id, editingSurface);
}

/** Delete at the END of a block: absorb the NEXT visible block into this one,
 *  caret staying at the join (GH #213). Exact mirror of `mergeWithPrev` —
 *  same page only, one "merge" undo, returns false when nothing merged. */
export function mergeWithNext(
  id: string,
  scope: OutlineScope | null = null,
  editingSurface: string | null = null,
): boolean {
  if (!blockWritable(id)) return false;
  // See mergeWithPrev: navOnly display lists are never a merge topology.
  if (scope?.navOnly) scope = null;
  const next = nextVisible(id, scope);
  if (next === null) return false;
  return absorbInto(id, next, editingSurface);
}

/** The one merge answer for Backspace-at-start and Delete-at-end: `absorbed`'s
 *  visible text is appended to `survivor` (no separator), its children are
 *  appended to survivor's, and it is removed. The survivor keeps its identity
 *  and hidden props; the absorbed `id::` is kept only when the survivor has
 *  none. Refuses (false) across pages. */
function absorbInto(survivor: string, absorbed: string, editingSurface: string | null): boolean {
  const node = doc.byId[absorbed];
  if (doc.byId[survivor].page !== node.page) return false; // don't merge across pages
  // The absorbed block's children move under the survivor, which may sit deeper
  // (Backspace into the last leaf of a deep chain). Refuse before mutating, as
  // every other reparenting door does; work is the moved subtrees only (I-22).
  for (const child of node.children) {
    if (!existingSubtreeFits(child, survivor)) {
      pushToast("Outline is too deep to merge", "error");
      return false;
    }
  }
  const fmt = formatForBlock(absorbed); // same page (checked above) → same format
  // Merge visible content only; keep the survivor's hidden props (it keeps its
  // identity) and drop the absorbed block's — otherwise the id::/collapsed::
  // lines would be concatenated mid-line and a block could end up with two ids.
  const keepSplit = splitProps(doc.byId[survivor].raw, isBuiltinHidden, fmt);
  const goneSplit = splitProps(node.raw, isBuiltinHidden, fmt);
  const joinOffset = keepSplit.visible.length;
  const pageName = node.page;

  // Preserve the absorbed block's id if the survivor has none — otherwise inbound
  // ((id)) references to the absorbed block would orphan on merge. Match the id
  // line in the block's on-disk syntax (md `id:: x` vs org drawer `:id: x`).
  let hidden = keepSplit.hidden;
  const survivorHasId = existingBlockId(doc.byId[survivor].raw, fmt) !== null;
  const absorbedProperty = blockRegions(node.raw, fmt).id;
  const absorbedId = absorbedProperty
    ? new TextDecoder().decode(new TextEncoder().encode(node.raw).subarray(...absorbedProperty.line)).trim()
    : null;
  if (!survivorHasId && absorbedId) {
    hidden = hidden ? `${hidden}\n${absorbedId}` : absorbedId;
  }

  const notice = prepareReferenceChangeNotice([absorbed], false, !survivorHasId && absorbedId !== null);
  pushUndo(notice.tag, [node.page]);

  setDoc(
    produce((s) => {
      s.byId[survivor].raw = joinProps(keepSplit.visible + goneSplit.visible, hidden, fmt);
      for (const c of node.children) s.byId[c].parent = survivor;
      s.byId[survivor].children.push(...node.children);
      const arr = node.parent === null
        ? s.pages[s.pages.findIndex((p) => p.name === pageName)].roots
        : s.byId[node.parent].children;
      arr.splice(arr.indexOf(absorbed), 1);
      delete s.byId[absorbed];
    })
  );
  startEditing(survivor, joinOffset, null, editingSurface);
  markDirty(pageName, ["save-block", "move-blocks", "delete-blocks"]);
  notice.show();
  return true;
}

/** Insert parsed outline siblings after `afterId`. Returns the last inserted id
 *  for focus, or null for empty input, read-only, missing host or excess depth. */
export function insertOutlineAfter(afterId: string, nodes: OutlineNode[]): string | null {
  return insertOutlineBeside(afterId, nodes, "after", "paste");
}

/** Insert parsed outline siblings BEFORE `beforeId` — the only way to put a
 *  block above one that owns its own Enter key (a code block first on a page,
 *  GH #480). Returns the FIRST inserted id for focus, or null under the same
 *  refusals as `insertOutlineAfter`. One "insert-block" undo. */
export function insertOutlineBefore(beforeId: string, nodes: OutlineNode[]): string | null {
  return insertOutlineBeside(beforeId, nodes, "before", "insert-block");
}

/** Shared body of insertOutlineAfter/Before. Focus answer: the inserted block
 *  the reading order ends on beside the anchor — last after it, first before. */
function insertOutlineBeside(
  anchorId: string,
  nodes: OutlineNode[],
  side: "before" | "after",
  undoLabel: string,
): string | null {
  if (!nodes.length) return null;
  // Read-only gate at the choke point — file drops (and any future caller)
  // must not mutate a page the round-trip self-check marked read-only
  // (Phase-6 review finding, validated).
  if (!blockWritable(anchorId) || !outlineFits(anchorId, nodes)) return null;
  pushUndo(undoLabel, [doc.byId[anchorId].page]);
  const parent = doc.byId[anchorId].parent;
  const pageName = doc.byId[anchorId].page;
  const incoming = new Set<string>();
  let focusId = anchorId;
  setDoc(
    produce((s) => {
      const created = createOutline(s, nodes, parent, pageName, anchorId, incoming);
      const sibs =
        parent === null
          ? s.pages[s.pages.findIndex((p) => p.name === pageName)].roots
          : s.byId[parent].children;
      sibs.splice(sibs.indexOf(anchorId) + (side === "after" ? 1 : 0), 0, ...created);
      focusId = side === "after" ? created[created.length - 1] : created[0];
    })
  );
  markDirty(pageName, "insert-blocks");
  return focusId;
}

/** Replace one empty leaf with a parsed outline in one transaction and undo
 * entry, reusing the host id for the first root and its hidden properties.
 * Returns the last root id, or null if the host is not empty/writable or the
 * outline exceeds the cap. */
export function replaceEmptyBlockWithOutline(id: string, nodes: OutlineNode[]): string | null {
  const current = doc.byId[id];
  if (!nodes.length || !current || current.children.length || !blockWritable(id)) return null;
  if (!outlineFits(id, nodes)) return null;
  const format = formatForBlock(id);
  const incoming = new Set<string>();
  const split = splitProps(current.raw, isBuiltinHidden, format);
  if (split.visible.trim()) return null;
  pushUndo("paste-replace-empty", [current.page]);
  let lastId = id;
  setDoc(produce((state) => {
    // Reuse the first host's identity and hidden props, preserving inbound refs.
    const created = createOutline(state, nodes, current.parent, current.page, id, incoming, { id, hidden: split.hidden });
    const siblings = current.parent === null
      ? state.pages[state.pages.findIndex((page) => page.name === current.page)].roots
      : state.byId[current.parent].children;
    siblings.splice(siblings.indexOf(id), 1, ...created);
    lastId = created[created.length - 1];
  }));
  markDirty(current.page, ["save-block", "insert-blocks"]);
  return lastId;
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
      removeSubtree(s, id);
    })
  );
  removeDeletedBlocksFromSidebar(removedSidebarIds);
  if (editingId() === id) endEdit("delete-block");
  markDirty(pageName, "delete-blocks");
}

export function deleteBlock(id: string) {
  if (!blockWritable(id)) return;
  const notice = prepareReferenceChangeNotice([id], true);
  pushUndo(notice.tag, [doc.byId[id].page]);
  deleteBlockInternal(id);
  notice.show();
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
  if (!page || !pageWritable(pageName)) return null;
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


export function toggleCollapse(id: string) {
  const n = doc.byId[id];
  if (!n || !blockWritable(id) || n.children.length === 0) return;
  pushUndo("collapse", [n.page]);
  writeCollapsed(id, !n.collapsed);
  markDirty(n.page, "save-block");
}

/** Explicitly collapse or expand a block (no-op if it has no children or is
 *  already in the requested state). */
export function setCollapsed(id: string, collapsed: boolean) {
  const n = doc.byId[id];
  if (!n || !blockWritable(id) || n.children.length === 0 || n.collapsed === collapsed) return;
  pushUndo("collapse", [n.page]);
  writeCollapsed(id, collapsed);
  markDirty(n.page, "save-block");
}
