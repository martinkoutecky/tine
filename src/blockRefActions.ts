import { msg } from "./i18n";
import { collapseCommandSurface } from "./collapseCommandSurface";
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
  insertEmptyChildBlock,
  insertOutlineAfter,
  insertOutlineBefore,
  pageByName,
  pasteClipboardPayload,
  selectedIds,
  setCollapsed,
  withUndoUnit,
  node as docNode,
} from "./document";
import { openInNewTab } from "./router";
import { openBlockInSidebar, openSwitcher } from "./ui";
import { focusedPaneId, openRouteInOtherPane, paneRouter } from "./panes";
import { editingId, editingSurface, startEditing } from "./editorController";
import { graphMeta } from "./graphSession";
import { pushToast } from "./toasts";
import { copyBlockLink } from "./components/blockLinkCopy";
import { type ClipboardPayloadSlot } from "./clipboard";
import { isPasteableUrl, wrapLink, type Edit } from "./editor/format";
import { activeElement, isElementTag, selectionOf } from "./windowRealm";

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

// ---------------------------------------------------------------------------
// Keyboard block commands (R2 B1). Each acts on the block selection when there
// is one (visible order), else on the block being edited, so a chord works from
// either mode. None has a default binding (OG binds none of them); each is a
// palette command and rebindable in Settings > Keyboard shortcuts.
// ---------------------------------------------------------------------------

/** The blocks a keyboard block command acts on: the selection (visible order),
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
  const ids = commandTargetIds();
  const set = new Set(ids);
  const roots = ids.filter((id) => !ancestorsIn(id, set));
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
  const ids = commandTargetIds();
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

/** GH #525 / #696: collapse immediate children; expand the first level with
 *  folded non-leaves, starting at the immediate children. The target opens so
 *  the result shows. Without a target, start at the page's top-level blocks. */
export function setTargetChildrenCollapsed(collapsed: boolean): boolean {
  let parents: string[] = commandTargetIds();
  let pageRoots: string[] | null = null;
  if (!parents.length) {
    const route = paneRouter(focusedPaneId()).route();
    pageRoots = route.kind === "page" ? [...(pageByName(route.name)?.roots ?? [])] : null;
    if (!pageRoots) return false;
    parents = [];
  }
  const surface = parents.length ? collapseCommandSurface(parents[0]) : undefined;
  const isCollapsed = (id: string) => {
    const stored = docNode(id)?.collapsed ?? false;
    return surface?.collapsed(id, stored) ?? stored;
  };
  const folded = (id: string) => {
    const n = docNode(id);
    return !!n && blockHasFoldableBody(id) && isCollapsed(id);
  };
  const children = (ids: string[]) => ids.flatMap((id) => docNode(id)?.children ?? []);
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
  const pages = [...new Set([...opened, ...level].map((id) => docNode(id)?.page).filter((p): p is string => !!p))];
  if (!pages.length) return false;
  const apply = () => {
    const setFold = (id: string, value: boolean) => {
      if (!blockHasFoldableBody(id) || isCollapsed(id) === value) return;
      if (surface) surface.toggle(id, isCollapsed(id));
      else setCollapsed(id, value);
    };
    for (const id of opened) setFold(id, false);
    for (const id of level) setFold(id, collapsed);
  };
  // Occurrence toggles own their persistence: an embed root saves its HOST
  // property (with its own undo), descendants only change local presentation.
  // A source-page undo snapshot would suppress and then lose that host undo.
  if (surface) apply();
  else withUndoUnit(collapsed ? "collapse-children" : "expand-children", pages, apply);
  return true;
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
