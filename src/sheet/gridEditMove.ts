// Edit-mode caret moves in a grid (GH #874): one routing module for both block editors
// (CM6 and the textarea), so the two never drift. Table title cells and board cards keep
// today's sheet keys; only cells of a `tine.view:: grid` edit like an outline.
import { batch } from "solid-js";
import { blockIsGridView, insertEmptyChildBlock, node, setRaw, withUndoUnit, type OutlineScope } from "../document";
import { editingId, editingOwner, startEditing, takeCaretFor, type CaretPos } from "../editorController";
import { editorCommandFor, isPermittedTabGesture } from "../keybindings";
import type { SheetCellCtx } from "./context";
import { gridEntry, gridNeighbour, type GridMove, type GridTarget } from "./gridNeighbour";
import { cellBlockId, cellOwner, selectCellAfterEdit, setCellSel } from "./selection";
import { wrapCompactGridCell } from "./mutations";

/** True for a cell of a grid view, the only sheet surface that edits like an outline. */
export function editsLikeOutline(cell: SheetCellCtx): boolean {
  return blockIsGridView(cell.gridId);
}

/** Opens the editor at a resolved target: a grid cell (selected, then edited) or an outline block. */
export function openGridTarget(target: GridTarget, surfaceId?: string): void {
  const caret: CaretPos = target.caret === "start" ? 0 : target.caret === "end" ? Number.MAX_SAFE_INTEGER : target.caret;
  if (target.cell) {
    // Selecting first mounts a cell outside the grid's render window (visibility hook)
    // and keeps the cell selection on the edited cell, as a click does.
    setCellSel(target.cell);
    startEditing(target.blockId, caret, cellOwner(target.cell));
  } else {
    startEditing(target.blockId, caret, null, surfaceId ?? null);
  }
}

/**
 * Moves the caret from `blockId` (edited in grid cell `cell`) in reading order. The caller
 * has already committed the editor's text. Returns false when the move has nowhere to go;
 * the key is then still the grid's (the caret stays put).
 */
export function moveInGridEdit(blockId: string, cell: SheetCellCtx, move: GridMove, col = 0): boolean {
  const target = gridNeighbour(blockId, cell, move, col);
  if (!target) return false;
  openGridTarget(target, cell.surfaceId);
  return true;
}

/** A cell's child outline is a complete selection/merge scope, excluding the fixed cell text. */
export function gridOutlineScope(cell: SheetCellCtx): OutlineScope {
  return { get roots() { return [...(node(cellBlockId(cell) ?? "")?.children ?? [])]; }, forceExpandedRoot: cellBlockId(cell) ?? undefined };
}

const structureCommands = new Set([
  "editor/comment", "editor/indent", "editor/outdent", "editor/move-block-up", "editor/move-block-down",
  "editor/select-block-up", "editor/select-block-down", "editor/select-all", "editor/new-sibling-after-subtree",
]);

/** Bounds the outline verbs before either editor dispatches them. */
export function gridBlocksCommand(id: string, cell: SheetCellCtx, command: string): boolean {
  const host = cellBlockId(cell);
  if (id === host) return structureCommands.has(command);
  if (command === "editor/move-block-up" || command === "editor/move-block-down") {
    // The outline mover can cross a child-list edge into a parent's sibling.
    // At the cell's own child-list boundary that would move into another cell.
    if (node(id)?.parent !== host) return false;
    const siblings = node(host!)?.children ?? [];
    return command.endsWith("up") ? siblings[0] === id : siblings.at(-1) === id;
  }
  return false;
}

/** Document outline commands publish their focus with ordinary outline ownership.
 * Reattach that focus to this cell in the same reactive batch, before either editor
 * remounts. Selection commands end editing and retain their cell-bounded scope. */
export function runGridOutline<T>(cell: SheetCellCtx | null | undefined, action: () => T): T {
  if (!cell || !editsLikeOutline(cell)) return action();
  return batch(() => {
    const result = action();
    const id = editingId();
    if (id && editingOwner() === null) {
      let parent: string | null = id;
      const host = cellBlockId(cell);
      while (parent && parent !== host) parent = node(parent)?.parent ?? null;
      if (parent === host) {
        const caret = takeCaretFor(id) ?? 0;
        setCellSel(cell);
        startEditing(id, caret, cellOwner(cell));
      }
    }
    return result;
  });
}

export function gridNestedBlock(id: string, cell: SheetCellCtx | null | undefined): boolean {
  return !!cell && editsLikeOutline(cell) && id !== cellBlockId(cell);
}

/** Shared grid key policy. Editors supply their own visual-edge measurements;
 * unhandled keys continue through the ordinary outline editor, within its cell scope. */
export function gridEditKey(e: KeyboardEvent, ctx: {
  id: string; cell: SheetCellCtx; raw: string; start: number; end: number;
  commit(raw: string): void; softNewline(): void; edge: GridMove | null; col: number;
}): boolean {
  const { id, cell, raw, start, end, commit } = ctx;
  const host = cellBlockId(cell), nested = id !== host;
  const plain = !e.ctrlKey && !e.metaKey && !e.altKey;
  if (plain && e.key === "Escape") {
    commit(raw);
    selectCellAfterEdit(cell);
    return true;
  }
  if (isPermittedTabGesture(e) && !nested) {
    commit(raw);
    moveInGridEdit(id, cell, e.shiftKey ? "tab-back" : "tab-forward");
    return true;
  }
  if (!nested && e.key === "Enter" && !e.ctrlKey && !e.metaKey) {
    if (e.shiftKey) { ctx.softNewline(); return true; }
    // Alt+Enter is an alias. The compact-grid wrapper keeps its rows intact;
    // the new text child precedes that hosted grid, as it does any other child.
    batch(() => withUndoUnit(`grid:enter:${id}`, [node(id).page], () => {
      if (blockIsGridView(id) && !wrapCompactGridCell(id)) return;
      const child = insertEmptyChildBlock(id, 0);
      if (!child) return;
      commit(raw.slice(0, start));
      setRaw(child, raw.slice(end));
      startEditing(child, 0, cellOwner(cell));
    }));
    return true;
  }
  if (plain && start === end) {
    if (e.key === "Backspace" && start === 0
      && (!nested || node(id)?.parent === host && node(host!)?.children[0] === id)) return true;
    // The fixed cell text must also survive forward deletion; nested Delete is
    // allowed only while its merge target remains in the cell's child scope.
    if (e.key === "Delete" && end === raw.length && !nested) return true;
    if (!e.shiftKey && ctx.edge) {
      commit(raw);
      moveInGridEdit(id, cell, ctx.edge, ctx.col);
      return true;
    }
  }
  const command = editorCommandFor(e);
  return !!command && gridBlocksCommand(id, cell, command);
}

/** Caret-only outline entry; block selection continues to see an opaque grid. */
export function enterGridEdit(gridId: string, surfaceId?: string): boolean {
  const target = gridEntry(gridId, "above", 0, surfaceId);
  if (!target) return false;
  openGridTarget(target, surfaceId);
  return true;
}
