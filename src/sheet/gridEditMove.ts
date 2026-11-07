// Edit-mode caret moves in a grid (GH #874): one routing module for both block editors
// (CM6 and the textarea), so the two never drift. Table title cells and board cards keep
// today's sheet keys; only cells of a `tine.view:: grid` edit like an outline.
import { blockIsGridView } from "../document";
import { startEditing, type CaretPos } from "../editorController";
import type { SheetCellCtx } from "./context";
import { gridNeighbour, type GridMove, type GridTarget } from "./gridNeighbour";
import { cellOwner, setCellSel } from "./selection";

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
