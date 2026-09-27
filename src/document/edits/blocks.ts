import { blockWritable, rawWithInheritedOrderListType, isOrdered, rawWithOrderListType, rawWithCollapsed, writeCollapsed } from "./properties";
import { doc, formatForBlock, setDoc, freshId, formatForPage, pageByName } from "../model";

/** Reveal a search result without creating an edit, undo entry, or save. */
export function revealNode(id: string): void {
  if (doc.byId[id]?.collapsed) setDoc("byId", id, "collapsed", false);
}
import { applyMarkerTransition } from "../../logbook";
import { timetrackingEnabled, logbookWithSecondSupport, logicalOutdenting, removeDeletedBlocksFromSidebar } from "../../ui";
import { pushRawUndo, pushUndo } from "../history";
import { markDirty } from "../save/engine";
import { produce } from "solid-js/store";
import { type OutlineNode } from "../../editor/outline";
import { splitProps, isBuiltinHidden, joinProps, isPropertiesOnly } from "../../editor/properties";
import { startEditing, editingId, endEdit } from "../../editorController";
import { indexInSiblings, rootsOf, OutlineScope, prevVisible } from "../tree";
import { existingBlockId } from "./identity";

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
