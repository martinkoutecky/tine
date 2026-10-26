import { msg } from "../../i18n";
import { blockWritable, pageWritable, rawWithInheritedOrderListType, writeCollapsed } from "./properties";
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
import { applyOutlineChange, markOutlineDirty, outlineChange, type OutlineChange } from "../kernel";
import { outlineMaxDepth, outlineDepth, type OutlineNode } from "../../editor/outline";
import { splitProps, BUILTIN_HIDE, HIDE_ID, joinProps, isPropertiesOnly, readPropertyValue, visibleOffsetToRawByte } from "../../editor/properties";
import { startEditing, editingId, endEdit, type EditorSelection } from "../../editorController";
import { batch } from "solid-js";
import { isBlockMoving, setBlockMoving } from "./moves";
import { depthOf, existingSubtreeFits, OutlineScope, prevVisible, nextVisible } from "../tree";
import { blockExternalId, existingBlockId, isBlockRefUuid } from "./identity";
import { captureReferenceChangeCount, pushReferenceChangeNotice, pushToast } from "../../toasts";
import { graphEpoch } from "../../graphSession";
import { blockRefsInText } from "../../render/pageRefs";
import { blockRegions, parserReady, soleBlockMacro } from "../../render/parse";
import { queryMacroExtent } from "../../editor/queryMacro";

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
    return splitProps(raw, HIDE_ID, format).visible;
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
    // Imported raw owns persistent collapse state, just as on page reload.
    // Ask lsdoc so property-looking text in code never folds a block.
    const collapsed = blockRegions(raw, format).properties
      .find((p) => p.primary && p.key.toLowerCase() === "collapsed")?.value === "true";
    state.byId[id] = { id, raw, collapsed, parent, page, children };
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
  const change = outlineChange({ verb: "insert_empty_child", parent: parentId, at }, [parent.page]);
  if ("refused" in change) {
    if (change.refused === "depth_cap") pushToast(msg("blocks-outline-is-too-deep-to-insert-a-child"), "error");
    return null;
  }
  pushUndo(`insert-child:${parentId}`, [parent.page]);
  applyOutlineChange(change);
  markOutlineDirty(change);
  return change.focus?.key ?? null;
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
        pushToast(msg("blocks-outline-is-too-deep-to-move"), "error");
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
  return !!doc.byId[hostId] && depthOf(hostId) + levelsBelowHost + outlineDepth(nodes) <= outlineMaxDepth();
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

/** Enter: split the block at `offset`, a caret in the editor's visible text
 *  (built-in properties hidden). The kernel's `split` owns the rule
 *  (outline::split): hidden props stay with the original block, a split at the
 *  head inserts an empty block before it, and GH #361's line boundary becomes the
 *  block separator. Returns false when the outline cap refuses a child. */
export function splitBlock(
  id: string,
  offset: number,
  forceChild: boolean = false,
  keepStartInScope: boolean = false,
  editingSurface: string | null = null,
) {
  const node = doc.byId[id];
  if (!node || !blockWritable(id)) return;
  const change = outlineChange({
    verb: "split",
    key: id,
    raw_offset: visibleOffsetToRawByte(node.raw, offset, BUILTIN_HIDE, formatForBlock(id)),
    force_child: forceChild,
    children_visible: node.children.length > 0 && !node.collapsed,
    keep_start_in_scope: keepStartInScope,
  }, [node.page]);
  if ("refused" in change) {
    if (change.refused !== "depth_cap") return;
    pushToast(msg("blocks-outline-is-too-deep-to-insert-a-child"), "error");
    return false;
  }
  pushUndo("split", [node.page]);
  applyOutlineChange(change);
  if (change.focus) {
    startEditing(change.focus.key, change.focus.editorOffset ?? 0, null, editingSurface);
  }
  markOutlineDirty(change);
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
  const pageName = doc.byId[id].page;
  // The kernel refuses the first sibling, a page-header parent (GH #638) and
  // the outline cap; only the cap is reported to the caller.
  const change = outlineChange({ verb: "indent", keys: [id] }, [pageName]);
  if ("refused" in change) return change.refused === "depth_cap" ? false : undefined;
  pushUndo("indent", [pageName]);
  // Reparenting remounts the editor. Publish its selection and ownership in the
  // same reactive flush as the tree change, before the replacement can focus.
  reparentEditingBlock(change, () => {
    applyOutlineChange(change);
    startEditing(id, caretOffset, null, editingSurface);
  });
}

/** Whether Tab would indent `id`: the kernel's verb, its answer discarded
 *  (DESIGN §5.4), so a preflight never restates the rule. O(page blocks). */
export function canIndentBlock(id: string): boolean {
  const node = doc.byId[id];
  return !!node && blockWritable(id) && !("refused" in outlineChange({ verb: "indent", keys: [id] }, [node.page]));
}

/** Run a reparenting tree update and the editor handoff in one reactive batch,
 *  marked as a block move so the old editor's blur during the flush does not
 *  end edit mode (#519, #495). An already active move (another page) is left
 *  to its owner. */
function reparentEditingBlock(change: OutlineChange, update: () => void): void {
  const ownsMove = !isBlockMoving();
  if (ownsMove) setBlockMoving(true, change.pages[0].name);
  try {
    batch(update);
    markOutlineDirty(change);
  } finally {
    if (ownsMove) setBlockMoving(false);
  }
}

/** Shift+Tab: move the block out to be the next sibling of its parent.
 *  `editingSurface` as in `indentBlock` (GH #477). */
export function outdentBlock(id: string, caretOffset: number | EditorSelection, editingSurface: string | null = null) {
  const node = doc.byId[id];
  if (!node || !blockWritable(id)) return;
  // Logical outdenting (OG `outliner/core.cljs:835-852` at `6e7afa8eb`) is a
  // setting, passed so keyboard, mobile and any future caller share the verb.
  const change = outlineChange({ verb: "outdent", keys: [id], logical: logicalOutdenting() }, [node.page]);
  if ("refused" in change) return;
  pushUndo("outdent", [node.page]);
  reparentEditingBlock(change, () => {
    applyOutlineChange(change);
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

/** The one merge answer for Backspace-at-start and Delete-at-end: the
 *  kernel's `merge` (outline::merge) appends `absorbed`'s visible text and its
 *  children to `survivor` and removes it; the survivor keeps its identity and
 *  hidden props, and takes the absorbed `id::` only when it has none. Refuses
 *  (false) across pages and past the outline cap. */
function absorbInto(survivor: string, absorbed: string, editingSurface: string | null): boolean {
  const pageName = doc.byId[absorbed].page;
  const change = outlineChange(
    { verb: "merge", survivor, absorbed, absorbed_referenced: false },
    [doc.byId[survivor].page, pageName],
  );
  if ("refused" in change) {
    if (change.refused === "depth_cap") pushToast(msg("blocks-outline-is-too-deep-to-merge"), "error");
    return false;
  }
  const transferred = change.notices.some((n) => n.kind === "id_transferred");
  const notice = prepareReferenceChangeNotice([absorbed], false, transferred);
  pushUndo(notice.tag, [pageName]);
  applyOutlineChange(change);
  if (change.focus) {
    startEditing(change.focus.key, change.focus.editorOffset ?? 0, null, editingSurface);
  }
  markOutlineDirty(change);
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
  const split = splitProps(current.raw, BUILTIN_HIDE, format);
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

/** Remove a block and its subtree (outline::delete). */
export function deleteBlock(id: string) {
  if (!blockWritable(id)) return;
  const pageName = doc.byId[id].page;
  const change = outlineChange({ verb: "delete", keys: [id] }, [pageName]);
  if ("refused" in change) return;
  const notice = prepareReferenceChangeNotice([id], true);
  pushUndo(notice.tag, [pageName]);
  const format = pageByName(pageName)?.format ?? "md";
  const removedSidebarIds = new Set<string>();
  for (const patch of change.pages) for (const key of patch.deleted) {
    removedSidebarIds.add(key);
    const durable = existingBlockId(doc.byId[key].raw, format);
    if (durable) removedSidebarIds.add(durable);
  }
  applyOutlineChange(change);
  removeDeletedBlocksFromSidebar(removedSidebarIds);
  if (editingId() === id) endEdit("delete-block");
  markOutlineDirty(change);
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


/** Page-embed bodies share the host's ordinary, persisted outline fold. */
export function blockHasPageEmbed(id: string): boolean {
  const n = doc.byId[id];
  if (!n) return false;
  const macro = soleBlockMacro(n.raw, formatForBlock(id));
  const target = macro?.args.join(", ").trim() ?? "";
  return macro?.name === "embed" && target.startsWith("[[") && target.endsWith("]]");
}

/** A block whose body folds with its outline bullet even without children: a
 *  page embed, or a block holding a query (OG editor.cljs `collapsable?`: a
 *  block with a query macro is collapsable, and query.cljs `custom-query*`
 *  hides the results of a collapsed block; GH #619 c6). */
export function blockHasFoldableBody(id: string): boolean {
  const n = doc.byId[id];
  if (!n) return false;
  return n.children.length > 0 || blockHasPageEmbed(id)
    || (parserReady() && n.raw.includes("{{") && queryMacroExtent(n.raw) !== null);
}

export function toggleCollapse(id: string) {
  const n = doc.byId[id];
  if (!n || !blockWritable(id) || !blockHasFoldableBody(id)) return;
  pushUndo("collapse", [n.page]);
  writeCollapsed(id, !n.collapsed, blockHasPageEmbed(id));
  markDirty(n.page, "save-block");
}

/** Explicitly collapse or expand a block (no-op if it has no foldable body or is
 *  already in the requested state). */
export function setCollapsed(id: string, collapsed: boolean) {
  const n = doc.byId[id];
  if (!n || !blockWritable(id) || !blockHasFoldableBody(id) || n.collapsed === collapsed) return;
  pushUndo("collapse", [n.page]);
  writeCollapsed(id, collapsed, blockHasPageEmbed(id));
  markDirty(n.page, "save-block");
}
