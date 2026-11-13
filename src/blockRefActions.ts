import { msg } from "./i18n";
import { collapseCommandRow, queryResultFolds, type CommandCollapseSurface } from "./collapseCommandSurface";
/** Block navigation for sidebar, tab and pane actions. Opening a block never
 * writes the graph: OG stamps an `id::` only when a reference is created (copy
 * ref/embed, `((` autocomplete), so these actions hand the live ref straight to
 * the destination and a saved session names an ID-less block by position
 * (blockPositionRef, GH #623, I-2). */
import {
  blockRef,
  blockHasFoldableBody,
  buildClipboardPayload,
  hasSelection,
  blockHasPageEmbed,
  blockHasQueryBody,
  blockIsGridView,
  insertEmptyChildBlock,
  mergeSelection,
  formatForBlock,
  insertOutlineAfter,
  insertOutlineBefore,
  pageByName,
  pasteClipboardPayload,
  selectedIds,
  topSelected,
  singleSelectedId,
  setCollapsed,
  storedFold,
  withUndoUnit,
  blockProperty,
  blockWritable,
  setBlockProperty,
  node as docNode,
} from "./document";
import { openInNewTab, openPage, openPageAtBlock, zoomToBlock } from "./router";
import { openBlockInSidebar, openPageInSidebar, openSwitcher } from "./ui";
import { focusedPaneId, openRouteInOtherPane, paneRouter } from "./panes";
import { refClickZoom } from "./copySettings";
import type { InternalLinkDest } from "./linkGesture";
import { editingId, editingSurface, endEdit, startEditing } from "./editorController";
import { graphMeta } from "./graphSession";
import { pushToast } from "./toasts";
import { copyBlockLink } from "./components/blockLinkCopy";
import { type ClipboardPayloadSlot } from "./clipboard";
import { isPasteableUrl, wrapLink, type Edit } from "./editor/format";
import { activeElement, isElementTag, selectionOf } from "./windowRealm";
import { soleBlockMacro } from "./render/parse";
import { isAnnotationBlock } from "./editor/annotation";

/** Open a block in the sidebar, a new tab, or the other pane. Synchronous: no save. */
export function openDurableBlock(id: string, destination: "sidebar" | "tab" | "pane"): void {
  const ref = blockRef(id);
  if (destination === "sidebar") openBlockInSidebar(ref);
  else {
    const route = { kind: "page" as const, name: ref.page, pageKind: ref.pageKind, block: ref.uuid, ...(ref.path ? { path: ref.path } : {}) };
    if (destination === "pane") openRouteInOtherPane(route);
    else openInNewTab(route);
  }
}

/** Open what a reference or embed points at (a block, or a whole page when
 *  `uuid` is absent) at the shared link destination (linkGesture.ts): sidebar,
 *  background tab, the other pane, or here. Here, a block opens in place in its
 *  page, or zoomed in with Settings → ref-click-zoom (OG). Synchronous: no save. */
export function openRefTarget(
  target: { page: string; pageKind: "journal" | "page"; uuid?: string; path?: string },
  dest: InternalLinkDest,
  fromPane: string,
): void {
  const at = target.path ? { path: target.path } : {};
  const route = { kind: "page" as const, name: target.page, pageKind: target.pageKind, ...(target.uuid ? { block: target.uuid } : {}), ...at };
  if (dest === "background") openInNewTab(route);
  else if (dest === "pane") openRouteInOtherPane(route, fromPane);
  else if (!target.uuid) {
    if (dest === "sidebar") openPageInSidebar(target.page, target.pageKind, target.path);
    else openPage(target.page, target.pageKind);
  } else {
    const block = { name: target.page, pageKind: target.pageKind, block: target.uuid, ...at };
    if (dest === "sidebar") openBlockInSidebar({ uuid: target.uuid, page: target.page, pageKind: target.pageKind, ...at });
    else if (refClickZoom()) zoomToBlock(block);
    else openPageAtBlock(block);
  }
}

// ---------------------------------------------------------------------------
// Keyboard block commands (R2 B1). Each acts on the block selection when there
// is one (visible order), else on the block being edited, so a chord works from
// either mode. None has a default binding (OG binds none of them); each is a
// palette command and rebindable in Settings > Keyboard shortcuts.
// ---------------------------------------------------------------------------

/** The blocks a keyboard block command acts on: the complete selected subtrees,
 *  else the block being edited, else none. */
export function commandTargetIds(): string[] {
  if (hasSelection()) return selectedIds();
  const editing = editingId();
  return editing && docNode(editing) ? [editing] : [];
}

/** GH #576: copy `((uuid))` for every selected block (or the edited block), the
 *  selection-mode twin of builtin Mod+C. Same door as the context menu's "Copy
 *  block refs": ids are saved first, several refs are Markdown bullets in
 *  visible order. */
export function copyTargetBlockRefs(): boolean {
  const ids = commandTargetIds();
  if (!ids.length) {
    pushToast(msg("block-ref-actions-select-blocks-or-edit-one-to-copy-their"), "warn");
    return false;
  }
  void copyBlockLink(ids, "ref");
  return true;
}

/** "Remove author" (GH #918): the block it acts on, or null where it is not
 *  offered. The block context menu offers it on one writable block carrying
 *  `tine.author::`; the command takes the same block from the selection or the
 *  editor, so a selection of several offers nothing. Target lookup reuses the
 *  memoized selection; property checks read only the target block. */
export function removeAuthorTarget(ids?: readonly string[]): string | null {
  const id = ids ? (ids.length === 1 ? ids[0] : null)
    : hasSelection() ? singleSelectedId() : editingId();
  return id !== null && blockWritable(id) && blockProperty(id, "tine.author") !== null ? id : null;
}

/** The one "Remove author" action, for the block context menu and its command:
 *  leaves the editor and deletes the block's `tine.author::` through the
 *  ordinary property write (one undo step, the guarded save). */
export function removeAuthor(id: string): void {
  endEdit("select-block");
  setBlockProperty(id, "tine.author", null);
}

function ancestorsIn(id: string, set: ReadonlySet<string>): boolean {
  for (let parent = docNode(id)?.parent ?? null; parent; parent = docNode(parent)?.parent ?? null) {
    if (set.has(parent)) return true;
  }
  return false;
}

/** GH #379: copy the target block(s), children included, directly beside them. Fresh
 *  ids (an `id::` already owned by a live block is never copied), one undo step.
 *  The caret follows the copy when a single block is being edited, so repeated
 *  presses stack duplicates. Returns the first inserted root above, or the last below. */
export async function duplicateTargetBlocks(side: "below" | "above" = "below"): Promise<string | null> {
  const roots = hasSelection() ? topSelected() : commandTargetIds();
  if (!roots.length) return null;
  const last = side === "above" ? roots[0] : roots[roots.length - 1];
  const wasEditing = !hasSelection() && editingId() === last;
  const payload = buildClipboardPayload(roots);
  if (!payload) {
    pushToast(msg("block-ref-actions-too-much-selected-to-duplicate"), "error");
    return null;
  }
  const lastNode = docNode(last)!;
  const emptyLeaf = lastNode.children.length === 0 && lastNode.raw.trim() === "";
  let created: string | null;
  if (side === "above" || emptyLeaf) {
    // A paste into an empty leaf REPLACES it, which would swallow the copy.
    created = (side === "above" ? insertOutlineBefore : insertOutlineAfter)(last, payload.blocks.map(function toOutline(b): { raw: string; children: ReturnType<typeof toOutline>[] } {
      return { raw: b.raw, children: b.children.map(toOutline) };
    }));
  } else {
    const slot: ClipboardPayloadSlot = { ...payload, op: "copy", generation: 0, graph: graphMeta()?.root ?? "", text: "" };
    created = await pasteClipboardPayload(last, slot);
  }
  if (created && wasEditing && docNode(created)) startEditing(created, docNode(created)!.raw.length, null, editingSurface());
  return created;
}

/** GH #634: append an empty child to the target block and edit it. Reaches
 *  places Enter+Tab cannot (an embedded block with no children: Enter makes a
 *  sibling outside the embed). A collapsed target opens so the child shows. */
export function insertChildIntoTarget(): string | null {
  const ids = hasSelection() ? topSelected() : commandTargetIds();
  const id = ids[ids.length - 1];
  const n = id ? docNode(id) : undefined;
  if (!id || !n) return null;
  const child = withUndoUnit("insert-child-block", [n.page], () => {
    if (docNode(id)!.collapsed) setCollapsed(id, false);
    return insertEmptyChildBlock(id, docNode(id)!.children.length);
  });
  if (!child) return null;
  startEditing(child, 0, null, editingSurface());
  return child;
}

/** GH #764: merge the selected blocks into the first one (palette and keymap
 *  only). Blocks whose meaning lives in their whole text or their children are
 *  refused whole, not flattened into a paragraph: a query, a page or block
 *  embed, a sheet and a PDF highlight. Everything else is the kernel's
 *  `merge_many` (`mergeSelection`). */
export function mergeSelectedBlocks(): boolean {
  const ids = selectedIds();
  for (const id of ids) {
    const n = docNode(id);
    if (!n) continue;
    const format = formatForBlock(id);
    if (blockHasQueryBody(id) || blockHasPageEmbed(id) || blockIsGridView(id)
      || soleBlockMacro(n.raw, format)?.name === "embed" || isAnnotationBlock(n.raw, format)) {
      pushToast(msg("selection-merge-special-blocks"), "warn");
      return false;
    }
  }
  return mergeSelection();
}

/** How the fold commands (GH #525 / #696 one level, GH #794 set depth) see the
 *  tree under their targets: which rows answer through a local surface (embed
 *  rows, query result groups), what counts as folded, and each block's visible
 *  children. GH #792: a query block's children are its result rows (and any
 *  outline children); the rows fold through their group's local surface, never
 *  the file. */
interface FoldWalk {
  /** The command targets (empty when the command falls back to the page's roots). */
  parents: string[];
  pageRoots: string[] | null;
  isCollapsed: (id: string) => boolean;
  surfaceOf: (id: string) => CommandCollapseSurface | undefined;
  children: (ids: string[]) => string[];
}

function foldWalk(pageFallback: boolean): FoldWalk | null {
  let parents: string[] = hasSelection() ? topSelected() : commandTargetIds();
  let pageRoots: string[] | null = null;
  if (!parents.length) {
    if (!pageFallback) return null;
    const route = paneRouter(focusedPaneId()).route();
    pageRoots = route.kind === "page" ? [...(pageByName(route.name)?.roots ?? [])] : null;
    if (!pageRoots) return null;
    parents = [];
  }
  const row = parents.length ? collapseCommandRow(parents[0]) : undefined;
  const surface = row?.surface;
  // A selected whole-block embed host folds its visible root row (D-20261030-EMB).
  if (row) parents = parents.map((id, i) => i === 0 ? row.id : id);
  // Result rows answer to the group that shows them, and so do their descendants.
  const rowSurface = new Map<string, CommandCollapseSurface>();
  const surfaceOf = (id: string): CommandCollapseSurface | undefined => rowSurface.get(id) ?? surface;
  const isCollapsed = (id: string) => {
    const stored = storedFold(id); // `collapsed::`, else a query's authored `:collapsed? true`
    return surfaceOf(id)?.collapsed(id, stored) ?? stored;
  };
  const children = (ids: string[]) => {
    const out = new Set<string>();
    for (const id of ids) {
      const own = rowSurface.get(id);
      for (const child of docNode(id)?.children ?? []) {
        if (own && !rowSurface.has(child)) rowSurface.set(child, own);
        out.add(child);
      }
      for (const group of queryResultFolds(id)) for (const row of group.roots()) {
        if (!docNode(row)) continue;
        if (!rowSurface.has(row)) rowSurface.set(row, group.surface);
        out.add(row);
      }
    }
    return [...out];
  };
  return { parents, pageRoots, isCollapsed, surfaceOf, children };
}

/** Apply fold changes as ONE undo step through the existing per-block door.
 *  Occurrence toggles own their persistence: an embed root saves its HOST
 *  property (with its own undo); below it a block embed saves the source's
 *  folds (D-20261030-EMB) and other surfaces change local presentation only.
 *  A source-page undo snapshot would suppress and then lose that host undo. */
function commitFolds(walk: FoldWalk, changes: [id: string, collapsed: boolean][], tag: string): void {
  const apply = () => {
    for (const [id, value] of changes) {
      if (!blockHasFoldableBody(id) || walk.isCollapsed(id) === value) continue;
      const own = walk.surfaceOf(id);
      if (own) own.toggle(id, walk.isCollapsed(id));
      else setCollapsed(id, value);
    }
  };
  const written = [...new Set(changes.filter(([id]) => !walk.surfaceOf(id)).map(([id]) => docNode(id)?.page).filter((p): p is string => !!p))];
  if (!written.length) apply();
  else withUndoUnit(tag, written, apply);
}

/** GH #525 / #696: collapse immediate children; expand the first level with
 *  folded non-leaves, starting at the immediate children. The target opens so
 *  the result shows. Without a target, start at the page's top-level blocks. */
export function setTargetChildrenCollapsed(collapsed: boolean): boolean {
  const walk = foldWalk(true);
  if (!walk) return false;
  const { parents, pageRoots, isCollapsed, children } = walk;
  const folded = (id: string) => {
    const n = docNode(id);
    return !!n && blockHasFoldableBody(id) && isCollapsed(id);
  };
  // GH #696: one level includes the targets themselves. Collapse folds the
  // children first and the targets once no child is left open; it never opens
  // a target. Expand opens a folded target alone before reaching its children.
  let opened: string[] = [];
  let level: string[];
  if (pageRoots) level = pageRoots;
  else if (collapsed) {
    const open = parents.filter((id) => blockHasFoldableBody(id) && !isCollapsed(id));
    const kids = children(open);
    level = kids.some((id) => blockHasFoldableBody(id) && !isCollapsed(id)) ? kids : open;
  } else {
    opened = parents.filter(folded);
    level = opened.length ? [] : children(parents);
  }
  if (!collapsed && !opened.length) {
    // Breadth first: one press changes one depth across every target. Leaves
    // never stop the search; all changes still use the existing undo/save path.
    while (level.length && !level.some(folded)) level = children(level);
  }
  const touched = [...opened, ...level];
  if (!touched.some((id) => docNode(id)?.page)) return false;
  commitFolds(walk, [...opened.map((id): [string, boolean] => [id, false]), ...level.map((id): [string, boolean] => [id, collapsed])],
    collapsed ? "collapse-children" : "expand-children");
  return true;
}

/** GH #794: show each target down to `depth`. The target is level 0; levels
 *  0 to depth-1 are opened and the blocks at level `depth` are folded, so depth
 *  0 folds the target and depth 2 shows children with their grandchildren folded.
 *  Levels below `depth` are hidden by the fold above them and are left alone,
 *  expanded or not. Only blocks whose state differs are written, in one undo
 *  step through the same door as the one-level commands (#696). Result rows of
 *  a query and block embeds fold through their own surfaces, as there. A
 *  descendant that is not loaded has no children to walk and is skipped.
 *  Returns whether anything changed; nothing selected or edited is a no-op. */
export function setTargetDepth(depth: number): boolean {
  const walk = foldWalk(false);
  if (!walk) return false;
  const { parents, isCollapsed, children } = walk;
  // A selection holds a block and its visible descendants: depth is relative to
  // the topmost selected blocks, so a nested selected block is not a second root.
  const set = new Set(parents);
  let level = parents.filter((id) => !ancestorsIn(id, set));
  const changes: [id: string, collapsed: boolean][] = [];
  const visit = (ids: string[], collapsed: boolean) => {
    for (const id of ids) {
      if (docNode(id) && blockHasFoldableBody(id) && isCollapsed(id) !== collapsed) changes.push([id, collapsed]);
    }
  };
  for (let n = 0; n < depth && level.length; n++) {
    visit(level, false);
    level = children(level);
  }
  visit(level, true);
  if (!changes.length) return false;
  commitFolds(walk, changes, "set-depth");
  return true;
}

/** The Set depth commands' door: with no block selected or edited, say what they need. */
export function runSetDepth(depth: number): boolean {
  if (!commandTargetIds().length) {
    pushToast(msg("block-ref-actions-select-blocks-or-edit-one-to-set-depth"), "warn");
    return false;
  }
  return setTargetDepth(depth);
}

/** The text a "search for selection" command should look up: the focused block
 *  editor's selection, else the rendered text selection. */
export function selectedTextForSearch(): string {
  const active = activeElement();
  if (isElementTag(active, "textarea") || isElementTag(active, "input")) {
    const el = active as HTMLTextAreaElement;
    return el.value.slice(el.selectionStart ?? 0, el.selectionEnd ?? 0).trim();
  }
  return selectionOf()?.toString().trim() ?? "";
}

/** GH #633: open the search switcher with the selected text filled in (plain
 *  search when nothing is selected). */
export function searchSelectedText(): void {
  const text = selectedTextForSearch().replace(/\s+/g, " ");
  openSwitcher({ prefill: text });
}

const BLOCK_REF_RE = /^\(\([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\)\)$/i;

/** GH #448, "Paste as link": the edit that makes the clipboard text a link. A
 *  URL (http(s)/mailto) or a block ref `((uuid))` wraps a non-empty selection as
 *  `[selection](target)` (Org: `[[target][selection]]`), so a block ref becomes
 *  `[selection](((uuid)))`; with no selection the target is inserted bare.
 *  Returns null when the clipboard holds neither, so the caller can say so. */
export function pasteAsLinkEdit(
  text: string,
  start: number,
  end: number,
  clipboard: string,
  format: "md" | "org",
): Edit | null {
  const target = clipboard.trim();
  if (!isPasteableUrl(target) && !BLOCK_REF_RE.test(target)) return null;
  if (start !== end) return wrapLink(text, start, end, target, format);
  const next = text.slice(0, start) + target + text.slice(end);
  const caret = start + target.length;
  return { text: next, start: caret, end: caret };
}
