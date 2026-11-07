// Reading-order caret moves through a grid in Edit mode (GH #874, D-20261105-GE).
//
// A grid is read row by row and, within a row, cell by cell; inside a cell, its own text
// and then its child outline from top to bottom. A cell (or a block nested in one) that is
// itself a grid holds a sub-grid, read right after its own text. Table and board views
// inside a cell are opaque here, as they are in the outline's visible order.
//
// - Left/Right walk reading order: Right at a row's end continues on the next row, so
//   Left and Right are inverses everywhere.
// - Up/Down walk the cell's outline; at the cell's edge they go to the cell above or
//   below in the same column, keeping the caret's visual column (as between outline
//   blocks). They enter a sub-grid at its first column and leave it the same way.
// - Tab/Shift+Tab, on a cell's own text, go to the next/previous cell of the same grid.
// - Leaving the outermost grid lands on its host block's text (up/left) or on the
//   block after the grid (down/right), as ordinary outline moves.
//
// The cell block never moves: this module only answers "where does the caret go".
// Cells are positional (grid -> row children -> cell children), as the grid adapter's
// `blockIdAt` reads them. Every grid nested in a grid renders on its outer grid's surface,
// so a move keeps the surface it started on.
import { blockIsGridView, blockIsOpaqueSheetView, nextVisible, node } from "../document";
import type { SheetCellCtx } from "./context";

export type GridMove = "left" | "right" | "up" | "down" | "tab-forward" | "tab-back";

/** Where the caret lands: the text's start or end, or a visual column on its first/last row. */
export type GridCaret = "start" | "end" | { col: number; edge: "first" | "last" };

/** `cell` is null when the target is an ordinary outline block outside every grid. */
export interface GridTarget {
  blockId: string;
  cell: SheetCellCtx | null;
  caret: GridCaret;
}

interface Pos {
  blockId: string;
  cell: SheetCellCtx | null;
}

const children = (id: string): readonly string[] => node(id)?.children ?? [];
const parentOf = (id: string): string | null => node(id)?.parent ?? null;

function cellCtx(gridId: string, row: number, col: number, surfaceId?: string): SheetCellCtx {
  const ctx: SheetCellCtx = { gridId, row, col };
  if (surfaceId !== undefined) ctx.surfaceId = surfaceId;
  return ctx;
}

function blockAt(gridId: string, row: number, col: number): string | null {
  const rowId = children(gridId)[row];
  return rowId ? children(rowId)[col] ?? null : null;
}

function cellBlock(cell: SheetCellCtx): string | null {
  return blockAt(cell.gridId, cell.row, cell.col);
}

/**
 * The grid cell whose content holds `blockId` (the block itself or a block nested in a
 * cell). `null`: the block is an ordinary outline block. `undefined`: it sits inside a
 * table or board, which this module does not walk.
 */
export function containingGridCell(blockId: string, surfaceId?: string): SheetCellCtx | null | undefined {
  let cur = blockId;
  for (;;) {
    const row = parentOf(cur);
    const grid = row ? parentOf(row) : null;
    if (row && grid && blockIsGridView(grid)) {
      const r = children(grid).indexOf(row), c = children(row).indexOf(cur);
      return r >= 0 && c >= 0 ? cellCtx(grid, r, c, surfaceId) : undefined;
    }
    if (!row) return null;
    // A table or board host between the block and its cell: not walked here.
    if (blockIsOpaqueSheetView(row)) return undefined;
    cur = row;
  }
}

/** First / last cell of a grid in reading order, or the first cell of the top / bottom row. */
function edgeCell(gridId: string, end: "first" | "last", column: "reading" | "first", surfaceId?: string): SheetCellCtx | null {
  const rows = children(gridId);
  const order = end === "first" ? rows.map((_, i) => i) : rows.map((_, i) => rows.length - 1 - i);
  for (const r of order) {
    const n = children(rows[r]).length;
    if (n === 0) continue;
    const c = end === "last" && column === "reading" ? n - 1 : 0;
    return cellCtx(gridId, r, c, surfaceId);
  }
  return null;
}

/** The next / previous cell of the same grid in reading order (skips empty rows). */
function siblingCell(cell: SheetCellCtx, step: 1 | -1): SheetCellCtx | null {
  const rows = children(cell.gridId);
  const inRow = children(rows[cell.row] ?? "").length;
  if (step === 1 && cell.col + 1 < inRow) return cellCtx(cell.gridId, cell.row, cell.col + 1, cell.surfaceId);
  if (step === -1 && cell.col > 0) return cellCtx(cell.gridId, cell.row, cell.col - 1, cell.surfaceId);
  for (let r = cell.row + step; r >= 0 && r < rows.length; r += step) {
    const n = children(rows[r]).length;
    if (n > 0) return cellCtx(cell.gridId, r, step === 1 ? 0 : n - 1, cell.surfaceId);
  }
  return null;
}

/** The cell above / below in the same column; rows without that column are skipped. */
function verticalCell(cell: SheetCellCtx, step: 1 | -1): SheetCellCtx | null {
  const rows = children(cell.gridId);
  for (let r = cell.row + step; r >= 0 && r < rows.length; r += step) {
    if (children(rows[r]).length > cell.col) return cellCtx(cell.gridId, r, cell.col, cell.surfaceId);
  }
  return null;
}

/** The last position inside `id`'s content (its own text, sub-grid or child outline). */
function lastPos(id: string, cell: SheetCellCtx, column: "reading" | "first"): Pos {
  if (blockIsGridView(id)) {
    const inner = edgeCell(id, "last", column, cell.surfaceId);
    const innerBlock = inner && cellBlock(inner);
    if (inner && innerBlock) return lastPos(innerBlock, inner, column);
    return { blockId: id, cell };
  }
  if (blockIsOpaqueSheetView(id)) return { blockId: id, cell };
  const kids = children(id);
  return kids.length ? lastPos(kids[kids.length - 1], cell, column) : { blockId: id, cell };
}

/** The first position inside `id`'s content after its own text, if any. */
function firstInside(id: string, cell: SheetCellCtx): Pos | null {
  if (blockIsGridView(id)) {
    const inner = edgeCell(id, "first", "first", cell.surfaceId);
    const innerBlock = inner && cellBlock(inner);
    return inner && innerBlock ? { blockId: innerBlock, cell: inner } : null;
  }
  if (blockIsOpaqueSheetView(id)) return null;
  const first = children(id)[0];
  return first ? { blockId: first, cell } : null;
}

/** After `id`'s whole subtree, staying in reading order (`vertical`: the cell below, not the next cell). */
function afterSubtree(id: string, cell: SheetCellCtx, vertical: boolean): Pos | null {
  const host = cellBlock(cell);
  let cur = id;
  while (cur !== host) {
    const parent = parentOf(cur);
    if (!parent) return null;
    const siblings = children(parent), at = siblings.indexOf(cur);
    if (at >= 0 && at + 1 < siblings.length) return { blockId: siblings[at + 1], cell };
    cur = parent;
  }
  const nextCell = vertical ? verticalCell(cell, 1) : siblingCell(cell, 1);
  const nextBlock = nextCell && cellBlock(nextCell);
  if (nextCell && nextBlock) return { blockId: nextBlock, cell: nextCell };
  return afterGrid(cell, vertical);
}

/** Leaving a grid past its end: after the grid host's subtree, in the host's own context. */
function afterGrid(cell: SheetCellCtx, vertical: boolean): Pos | null {
  const outer = containingGridCell(cell.gridId, cell.surfaceId);
  if (outer === undefined) return null;
  if (outer === null) {
    const after = nextVisible(cell.gridId);
    return after ? { blockId: after, cell: null } : null;
  }
  return afterSubtree(cell.gridId, outer, vertical);
}

/** Before `id` in reading order (`vertical`: from a cell's own text, the cell above). */
function before(id: string, cell: SheetCellCtx, vertical: boolean): Pos | null {
  const host = cellBlock(cell);
  if (id !== host) {
    const parent = parentOf(id);
    if (!parent) return null;
    const siblings = children(parent), at = siblings.indexOf(id);
    if (at > 0) return lastPos(siblings[at - 1], cell, vertical ? "first" : "reading");
    return { blockId: parent, cell };
  }
  const prevCell = vertical ? verticalCell(cell, -1) : siblingCell(cell, -1);
  const prevBlock = prevCell && cellBlock(prevCell);
  if (prevCell && prevBlock) return lastPos(prevBlock, prevCell, vertical ? "first" : "reading");
  // Leaving a grid past its start: the grid host's own text, in the host's context.
  const outer = containingGridCell(cell.gridId, cell.surfaceId);
  if (outer === undefined) return null;
  return { blockId: cell.gridId, cell: outer };
}

/**
 * Where a caret move from `blockId` (being edited in grid cell `cell`) lands, or null when
 * the move has nowhere to go (Tab at the grid's ends, a grid at the page's end). `col` is
 * the caret's visual column, kept by Up/Down.
 */
export function gridNeighbour(blockId: string, cell: SheetCellCtx, move: GridMove, col = 0): GridTarget | null {
  const host = cellBlock(cell);
  if (!host || !node(blockId)) return null;
  if (move === "tab-forward" || move === "tab-back") {
    if (blockId !== host) return null;
    const target = siblingCell(cell, move === "tab-forward" ? 1 : -1);
    const targetBlock = target && cellBlock(target);
    return target && targetBlock ? { blockId: targetBlock, cell: target, caret: "start" } : null;
  }
  const forward = move === "right" || move === "down";
  const vertical = move === "up" || move === "down";
  const pos = forward
    ? firstInside(blockId, cell) ?? afterSubtree(blockId, cell, vertical)
    : before(blockId, cell, vertical);
  if (!pos) return null;
  const caret: GridCaret = vertical ? { col, edge: forward ? "first" : "last" } : forward ? "start" : "end";
  return { blockId: pos.blockId, cell: pos.cell, caret };
}

/**
 * Entering a grid from the outline with a vertical move: Down from the grid host's own
 * text lands in the top-left cell; Up from the block below the grid lands on the last
 * block of the bottom-left cell.
 */
export function gridEntry(gridId: string, from: "above" | "below", col = 0, surfaceId?: string): GridTarget | null {
  if (!blockIsGridView(gridId)) return null;
  const cell = edgeCell(gridId, from === "above" ? "first" : "last", "first", surfaceId);
  const block = cell && cellBlock(cell);
  if (!cell || !block) return null;
  if (from === "above") return { blockId: block, cell, caret: { col, edge: "first" } };
  const last = lastPos(block, cell, "first");
  return { blockId: last.blockId, cell: last.cell, caret: { col, edge: "last" } };
}
