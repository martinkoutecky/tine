import { createRoot, createSignal } from "solid-js";
import { doc, clearSelection, selectBlock, prevVisible, nextVisible, blockIsGridView, withUndoUnit, blockPageReadOnly } from "../store";
import { endEdit, startEditing } from "../editorController";
import { isSheetCellHidden, splitProps } from "../editor/properties";
import {
  registerEditingStartListener,
  registerModeResetListener,
  registerOutlineSelectionListener,
} from "../modeHooks";
import { buildMatrix, type MatrixCell, type SheetMatrix } from "./matrix";
import type { SheetCellCtx } from "./context";
import {
  copySheetSelection,
  cutSheetSelection,
  deleteColumn,
  deleteRow,
  fillSheetSelection,
  insertColumn,
  insertRow,
  materializeCell,
  moveSheetSelection,
  pasteStructuralSheetSelection,
  pasteTextIntoSheetSelection,
  rectForSheetSelection,
  type SheetMoveDirection,
  type SheetPoint,
  type SheetRect,
} from "./mutations";

export const SEAM_STEPPING = true;

export interface CellSel extends SheetCellCtx {
  kind: "cell";
}

export interface RangeSel {
  kind: "range";
  gridId: string;
  anchor: SheetPoint;
  focus: SheetPoint;
}

export interface RowSeamSel {
  kind: "row-seam";
  gridId: string;
  anchor: SheetPoint;
  col: number;
  at: number;
}

export interface ColSeamSel {
  kind: "col-seam";
  gridId: string;
  anchor: SheetPoint;
  row: number;
  at: number;
}

export type SheetSel = CellSel | RangeSel | RowSeamSel | ColSeamSel;
type CellSelInput = SheetCellCtx | CellSel;
type SheetSelInput = SheetSel | CellSelInput;

export type { SheetPoint, SheetRect };

interface CellSelectionHooks {
  clearOutlineSelection: () => void;
  endActiveEdit: () => void;
}

export interface SheetViewAdapter {
  bounds: () => { rows: number; cols: number };
  blockIdAt?: (row: number, col: number) => string | null;
  cellForBlock?: (blockId: string) => CellSel | null;
  startEditing?: (sel: CellSelInput, offset?: number) => boolean;
  activate?: (sel: CellSel) => boolean;
  overtype?: (sel: CellSel, text: string) => boolean;
  moveWithMod?: (sel: CellSel, dir: CellDirection) => boolean;
}

const [activeCellSel, writeCellSel] = createRoot(() => createSignal<SheetSel | null>(null));
const [aggregateFooterPins, writeAggregateFooterPins] = createRoot(() => createSignal<ReadonlySet<string>>(new Set<string>()));
const [emptyTagColumns, writeEmptyTagColumns] = createRoot(() =>
  createSignal<ReadonlyMap<string, ReadonlySet<string>>>(new Map())
);
const lastByGrid = new Map<string, { row: number; col: number }>();
const adapters = new Map<string, SheetViewAdapter>();

let hooks: CellSelectionHooks = {
  clearOutlineSelection: clearSelection,
  endActiveEdit: () => endEdit("select-block"),
};

export function installCellSelectionHooks(next: Partial<CellSelectionHooks>): () => void {
  const prev = hooks;
  hooks = { ...hooks, ...next };
  return () => {
    hooks = prev;
  };
}

export function registerSheetViewAdapter(gridId: string, adapter: SheetViewAdapter): () => void {
  adapters.set(gridId, adapter);
  return () => {
    if (adapters.get(gridId) === adapter) adapters.delete(gridId);
  };
}

function adapterFor(gridId: string): SheetViewAdapter | null {
  return adapters.get(gridId) ?? null;
}

function clearCellSelectionOnly(): void {
  writeCellSel(null);
}

export function resetCellSelectionForTests(): void {
  clearCellSelectionOnly();
  lastByGrid.clear();
  writeAggregateFooterPins(new Set<string>());
  writeEmptyTagColumns(new Map());
}

export function aggregateFooterPinned(ownerId: string): boolean {
  return aggregateFooterPins().has(ownerId);
}

export function setAggregateFooterPinned(ownerId: string, pinned: boolean): void {
  writeAggregateFooterPins((cur) => {
    const next = new Set(cur);
    if (pinned) next.add(ownerId);
    else next.delete(ownerId);
    return next;
  });
}

export function toggleAggregateFooterPinned(ownerId: string): void {
  setAggregateFooterPinned(ownerId, !aggregateFooterPinned(ownerId));
}

export function emptyTagColumnsForBoard(ownerId: string): readonly string[] {
  return [...(emptyTagColumns().get(ownerId) ?? [])];
}

export function addEmptyTagColumn(ownerId: string, tag: string): void {
  const clean = tag.trim();
  if (!clean) return;
  writeEmptyTagColumns((cur) => {
    const next = new Map(cur);
    const tags = new Set(next.get(ownerId) ?? []);
    tags.add(clean);
    next.set(ownerId, tags);
    return next;
  });
}

export function pruneEmptyTagColumns(ownerId: string, persisted: ReadonlySet<string>): void {
  const current = emptyTagColumns().get(ownerId);
  if (!current) return;
  const nextTags = [...current].filter((tag) => !persisted.has(tag));
  if (nextTags.length === current.size) return;
  writeEmptyTagColumns((cur) => {
    const next = new Map(cur);
    if (nextTags.length) next.set(ownerId, new Set(nextTags));
    else next.delete(ownerId);
    return next;
  });
}

export function clearEmptyTagColumns(ownerId?: string): void {
  if (!ownerId) {
    writeEmptyTagColumns(new Map());
    return;
  }
  if (!emptyTagColumns().has(ownerId)) return;
  writeEmptyTagColumns((cur) => {
    const next = new Map(cur);
    next.delete(ownerId);
    return next;
  });
}

export function isSheetCellOwner(owner: string | null): boolean {
  return owner?.startsWith("sheet:") ?? false;
}

export function cellOwner(sel: SheetCellCtx): string {
  return `sheet:${sel.gridId}:${sel.row}:${sel.col}`;
}

export function cellSurfaceKey(gridId: string): string {
  return `sheet:${gridId}`;
}

function isRangeSel(sel: SheetSel | null): sel is RangeSel {
  return sel?.kind === "range";
}

function isSeamSel(sel: SheetSel | null): sel is RowSeamSel | ColSeamSel {
  return sel?.kind === "row-seam" || sel?.kind === "col-seam";
}

export function rowSeamSel(gridId: string, at: number, col: number): RowSeamSel {
  return { kind: "row-seam", gridId, at, col, anchor: { row: at, col } };
}

export function colSeamSel(gridId: string, at: number, row: number): ColSeamSel {
  return { kind: "col-seam", gridId, at, row, anchor: { row, col: at } };
}

function normalizeSel(sel: SheetSelInput): SheetSel {
  if ("kind" in sel && sel.kind) {
    if (sel.kind === "row-seam") {
      const s = sel as RowSeamSel & { anchor?: SheetPoint };
      return rowSeamSel(s.gridId, s.at, s.anchor?.col ?? s.col);
    }
    if (sel.kind === "col-seam") {
      const s = sel as ColSeamSel & { anchor?: SheetPoint };
      return colSeamSel(s.gridId, s.at, s.anchor?.row ?? s.row);
    }
    return { ...sel } as SheetSel;
  }
  return { kind: "cell", gridId: sel.gridId, row: sel.row, col: sel.col };
}

function rememberSelection(sel: SheetSel): void {
  if (sel.kind === "cell") lastByGrid.set(sel.gridId, { row: sel.row, col: sel.col });
  else if (sel.kind === "range") lastByGrid.set(sel.gridId, { ...sel.focus });
}

export function cellSel(): SheetSel | null {
  return activeCellSel();
}

export function setCellSel(sel: SheetSelInput | null): void {
  if (sel) {
    const normalized = normalizeSel(sel);
    rememberSelection(normalized);
    hooks.clearOutlineSelection();
    hooks.endActiveEdit();
    writeCellSel(normalized);
    return;
  }
  writeCellSel(null);
}

export function lastCellFor(gridId: string): { row: number; col: number } | null {
  const last = lastByGrid.get(gridId);
  return last ? { ...last } : null;
}

function rowsForGrid(gridId: string): { id: string; cellIds: readonly string[] }[] {
  return (doc.byId[gridId]?.children ?? []).map((id) => ({
    id,
    cellIds: doc.byId[id]?.children ?? [],
  }));
}

export function matrixForGrid(gridId: string): SheetMatrix {
  return buildMatrix(rowsForGrid(gridId));
}

function boundsForGrid(gridId: string): { rows: number; cols: number } {
  const adapter = adapterFor(gridId);
  if (adapter) return adapter.bounds();
  const matrix = matrixForGrid(gridId);
  return { rows: matrix.rows, cols: matrix.rows === 0 ? 0 : matrix.cols };
}

export function cellAt(sel: CellSelInput): MatrixCell | null {
  const matrix = matrixForGrid(sel.gridId);
  return matrix.cells.find((cell) => cell.row === sel.row && cell.col === sel.col) ?? null;
}

export function cellBlockId(sel: CellSelInput): string | null {
  const adapter = adapterFor(sel.gridId);
  if (adapter?.blockIdAt) return adapter.blockIdAt(sel.row, sel.col);
  return cellAt(sel)?.blockId ?? null;
}

export function cellForBlockId(blockId: string): CellSel | null {
  const rowId = doc.byId[blockId]?.parent ?? null;
  const gridId = rowId ? doc.byId[rowId]?.parent ?? null : null;
  if (gridId && blockIsGridView(gridId)) {
    const cell = matrixForGrid(gridId).cells.find((c) => c.blockId === blockId);
    if (cell) return { kind: "cell", gridId, row: cell.row, col: cell.col };
  }
  for (const [adapterGridId, adapter] of adapters) {
    const cell = adapter.cellForBlock?.(blockId);
    if (cell && cell.gridId === adapterGridId) return cell;
  }
  return null;
}

function enclosingCellForGrid(gridId: string): CellSel | null {
  return cellForBlockId(gridId);
}

export function focusCell(sel: CellSel | RangeSel): CellSel {
  if (sel.kind === "cell") return sel;
  return { kind: "cell", gridId: sel.gridId, row: sel.focus.row, col: sel.focus.col };
}

export function sheetSelectionRect(sel: SheetSel | null): SheetRect | null {
  if (!sel || isSeamSel(sel)) return null;
  return rectForSheetSelection(sel);
}

export function sheetSelectionRectForGrid(gridId: string): SheetRect | null {
  const sel = cellSel();
  if (!sel || sel.gridId !== gridId || isSeamSel(sel)) return null;
  return rectForSheetSelection(sel);
}

export function cellIsInRange(gridId: string, row: number, col: number): boolean {
  const rect = sheetSelectionRectForGrid(gridId);
  return !!rect && row >= rect.top && row <= rect.bottom && col >= rect.left && col <= rect.right;
}

function clampCell(gridId: string, wanted: { row: number; col: number }): CellSel | null {
  const bounds = boundsForGrid(gridId);
  if (bounds.rows <= 0 || bounds.cols <= 0) return null;
  return {
    kind: "cell",
    gridId,
    row: Math.max(0, Math.min(wanted.row, bounds.rows - 1)),
    col: Math.max(0, Math.min(wanted.col, bounds.cols - 1)),
  };
}

function hasAdapter(gridId: string): boolean {
  return adapters.has(gridId);
}

function clampPoint(gridId: string, wanted: SheetPoint): SheetPoint | null {
  const cell = clampCell(gridId, wanted);
  return cell ? { row: cell.row, col: cell.col } : null;
}

function setRangeOrCell(gridId: string, anchor: SheetPoint, focus: SheetPoint): boolean {
  const a = clampPoint(gridId, anchor);
  const f = clampPoint(gridId, focus);
  if (!a || !f) return false;
  if (a.row === f.row && a.col === f.col) setCellSel({ kind: "cell", gridId, row: f.row, col: f.col });
  else setCellSel({ kind: "range", gridId, anchor: a, focus: f });
  return true;
}

export function setCellRangeSelection(gridId: string, anchor: SheetPoint, focus: SheetPoint): boolean {
  return setRangeOrCell(gridId, anchor, focus);
}

export function extendCellSelectionTo(gridId: string, focus: SheetPoint): boolean {
  const sel = cellSel();
  const anchor =
    sel && sel.gridId === gridId && !isSeamSel(sel)
      ? sel.kind === "range"
        ? sel.anchor
        : { row: sel.row, col: sel.col }
      : focus;
  return setRangeOrCell(gridId, anchor, focus);
}

export function selectTopRowSeam(gridId: string, col = 0): boolean {
  const bounds = boundsForGrid(gridId);
  if (bounds.rows <= 0 || bounds.cols <= 0) return false;
  setCellSel(rowSeamSel(gridId, 0, Math.max(0, Math.min(col, bounds.cols - 1))));
  return true;
}

export function selectTopRowSeamAfterEdit(gridId: string, col = 0): boolean {
  endEdit("select-block");
  return selectTopRowSeam(gridId, col);
}

export function enterGridSelection(gridId: string): boolean {
  if (!blockIsGridView(gridId)) return false;
  const target = clampCell(gridId, lastCellFor(gridId) ?? { row: 0, col: 0 });
  if (!target) return false;
  setCellSel(target);
  return true;
}

export function startCellEditing(sel: CellSelInput, offset?: number): boolean {
  const adapter = adapterFor(sel.gridId);
  if (adapter?.startEditing) return adapter.startEditing(sel, offset);
  const blockId = cellBlockId(sel);
  if (!blockId) return false;
  if (blockPageReadOnly(blockId)) return false; // org round-trip gate (review finding)
  const node = doc.byId[blockId];
  if (!node) return false;
  const visibleLen = splitProps(node.raw, isSheetCellHidden).visible.length;
  setCellSel({ kind: "cell", gridId: sel.gridId, row: sel.row, col: sel.col });
  startEditing(blockId, offset ?? visibleLen, cellOwner(sel));
  return true;
}

export function selectCellAfterEdit(sel: SheetCellCtx): void {
  endEdit("select-block");
  setCellSel(sel);
}

type CellDirection = "up" | "down" | "left" | "right";

function flowOutVertical(sel: { gridId: string }, dir: "up" | "down"): boolean {
  clearCellSelectionOnly();
  const target = dir === "up" ? prevVisible(sel.gridId) : nextVisible(sel.gridId);
  selectBlock(target ?? sel.gridId);
  return true;
}

function exitLeft(sel: { gridId: string }): boolean {
  clearCellSelectionOnly();
  selectBlock(sel.gridId);
  return true;
}

function setClampedCell(gridId: string, row: number, col: number): boolean {
  const next = clampCell(gridId, { row, col });
  if (!next) return false;
  setCellSel(next);
  return true;
}

function moveFromRowSeam(sel: RowSeamSel, dir: CellDirection): boolean {
  const bounds = boundsForGrid(sel.gridId);
  if (bounds.rows <= 0 || bounds.cols <= 0) return false;

  if (dir === "up") {
    if (sel.at <= 0) return true;
    return setClampedCell(sel.gridId, sel.at - 1, sel.anchor.col);
  }
  if (dir === "down") {
    if (sel.at >= bounds.rows) return true;
    return setClampedCell(sel.gridId, sel.at, sel.anchor.col);
  }
  if (dir === "left") {
    setCellSel(rowSeamSel(sel.gridId, sel.at, Math.max(0, sel.anchor.col - 1)));
    return true;
  }
  setCellSel(rowSeamSel(sel.gridId, sel.at, Math.min(bounds.cols - 1, sel.anchor.col + 1)));
  return true;
}

function moveFromColSeam(sel: ColSeamSel, dir: CellDirection): boolean {
  const bounds = boundsForGrid(sel.gridId);
  if (bounds.rows <= 0 || bounds.cols <= 0) return false;

  if (dir === "left") {
    if (sel.at <= 0) return true;
    return setClampedCell(sel.gridId, sel.anchor.row, sel.at - 1);
  }
  if (dir === "right") {
    if (sel.at >= bounds.cols) return true;
    return setClampedCell(sel.gridId, sel.anchor.row, sel.at);
  }
  if (dir === "up") {
    setCellSel(colSeamSel(sel.gridId, sel.at, Math.max(0, sel.anchor.row - 1)));
    return true;
  }
  setCellSel(colSeamSel(sel.gridId, sel.at, Math.min(bounds.rows - 1, sel.anchor.row + 1)));
  return true;
}

export function moveCellSelectionFrom(sel: SheetSel, dir: CellDirection): boolean {
  if (sel.kind === "row-seam") return moveFromRowSeam(sel, dir);
  if (sel.kind === "col-seam") return moveFromColSeam(sel, dir);
  if (sel.kind === "range") return moveCellSelectionFrom(focusCell(sel), dir);

  const bounds = boundsForGrid(sel.gridId);
  if (bounds.rows <= 0 || bounds.cols <= 0) return false;
  if (hasAdapter(sel.gridId)) {
    if (dir === "up") {
      if (sel.row <= 0) return flowOutVertical(sel, "up");
      return setClampedCell(sel.gridId, sel.row - 1, sel.col);
    }
    if (dir === "down") {
      if (sel.row >= bounds.rows - 1) return flowOutVertical(sel, "down");
      return setClampedCell(sel.gridId, sel.row + 1, sel.col);
    }
    if (dir === "left") {
      if (sel.col <= 0) return exitLeft(sel);
      return setClampedCell(sel.gridId, sel.row, sel.col - 1);
    }
    if (sel.col >= bounds.cols - 1) return true;
    return setClampedCell(sel.gridId, sel.row, sel.col + 1);
  }

  if (dir === "up") {
    if (SEAM_STEPPING) {
      setCellSel(rowSeamSel(sel.gridId, sel.row, sel.col));
      return true;
    }
    if (sel.row <= 0) return flowOutVertical(sel, "up");
    setCellSel({ ...sel, row: sel.row - 1 });
    return true;
  }
  if (dir === "down") {
    if (SEAM_STEPPING) {
      setCellSel(rowSeamSel(sel.gridId, sel.row + 1, sel.col));
      return true;
    }
    if (sel.row >= bounds.rows - 1) return flowOutVertical(sel, "down");
    setCellSel({ ...sel, row: sel.row + 1 });
    return true;
  }
  if (dir === "left") {
    if (SEAM_STEPPING) {
      setCellSel(colSeamSel(sel.gridId, sel.col, sel.row));
      return true;
    }
    if (sel.col <= 0) return exitLeft(sel);
    setCellSel({ ...sel, col: sel.col - 1 });
    return true;
  }
  if (SEAM_STEPPING) {
    setCellSel(colSeamSel(sel.gridId, sel.col + 1, sel.row));
    return true;
  }
  if (sel.col >= bounds.cols - 1) return true;
  setCellSel({ ...sel, col: sel.col + 1 });
  return true;
}

function moveCellTab(sel: SheetSel, dir: 1 | -1): boolean {
  if (isSeamSel(sel)) return true;
  const cell = focusCell(sel);
  const bounds = boundsForGrid(cell.gridId);
  if (bounds.rows <= 0 || bounds.cols <= 0) return false;
  const next = cell.row * bounds.cols + cell.col + dir;
  if (next < 0 || next >= bounds.rows * bounds.cols) return true;
  setCellSel({
    kind: "cell",
    gridId: cell.gridId,
    row: Math.floor(next / bounds.cols),
    col: next % bounds.cols,
  });
  return true;
}

function extendCellRange(sel: SheetSel, dir: CellDirection): boolean {
  if (isSeamSel(sel)) return true;
  const anchor = sel.kind === "range" ? sel.anchor : { row: sel.row, col: sel.col };
  const cur = sel.kind === "range" ? sel.focus : { row: sel.row, col: sel.col };
  const wanted =
    dir === "up" ? { row: cur.row - 1, col: cur.col }
    : dir === "down" ? { row: cur.row + 1, col: cur.col }
    : dir === "left" ? { row: cur.row, col: cur.col - 1 }
    : { row: cur.row, col: cur.col + 1 };
  return setRangeOrCell(sel.gridId, anchor, wanted);
}

function selectRows(sel: SheetSel): boolean {
  if (isSeamSel(sel)) return true;
  const bounds = boundsForGrid(sel.gridId);
  if (bounds.rows <= 0 || bounds.cols <= 0) return false;
  const rect = rectForSheetSelection(sel);
  return setRangeOrCell(
    sel.gridId,
    { row: rect.top, col: 0 },
    { row: rect.bottom, col: bounds.cols - 1 }
  );
}

function selectColumns(sel: SheetSel): boolean {
  if (isSeamSel(sel)) return true;
  const bounds = boundsForGrid(sel.gridId);
  if (bounds.rows <= 0 || bounds.cols <= 0) return false;
  const rect = rectForSheetSelection(sel);
  return setRangeOrCell(
    sel.gridId,
    { row: 0, col: rect.left },
    { row: bounds.rows - 1, col: rect.right }
  );
}

function selectAllGrid(gridId: string): boolean {
  const bounds = boundsForGrid(gridId);
  if (bounds.rows <= 0 || bounds.cols <= 0) return false;
  return setRangeOrCell(gridId, { row: 0, col: 0 }, { row: bounds.rows - 1, col: bounds.cols - 1 });
}

export function moveCellAfterEdit(sel: SheetCellCtx, dir: CellDirection | "tab-forward" | "tab-back"): void {
  endEdit("select-block");
  const cell: CellSel = { kind: "cell", ...sel };
  if (dir === "tab-forward") moveCellTab(cell, 1);
  else if (dir === "tab-back") moveCellTab(cell, -1);
  else moveCellSelectionFrom(cell, dir);
}

function cellElement(sel: CellSelInput): HTMLElement | null {
  if (typeof document === "undefined") return null;
  const esc = (value: string) =>
    typeof CSS !== "undefined" && CSS.escape ? CSS.escape(value) : value.replace(/["\\]/g, "\\$&");
  return document.querySelector(
    `.sheet-cell[data-sheet-grid-id="${esc(sel.gridId)}"][data-row="${sel.row}"][data-col="${sel.col}"]`
  );
}

function replaceThroughMountedEditor(sel: CellSelInput, text: string): void {
  const apply = () => {
    const textarea = cellElement(sel)?.querySelector("textarea.block-editor") as HTMLTextAreaElement | null;
    if (!textarea) return false;
    textarea.value = text;
    textarea.setSelectionRange(text.length, text.length);
    let ev: Event;
    try {
      ev = new InputEvent("input", { bubbles: true, inputType: "insertText", data: text });
    } catch {
      ev = new Event("input", { bubbles: true });
    }
    textarea.dispatchEvent(ev);
    return true;
  };
  queueMicrotask(() => {
    if (apply()) return;
    if (typeof requestAnimationFrame === "function") requestAnimationFrame(() => apply());
    else setTimeout(() => apply(), 0);
  });
}

function overtypeCell(sel: CellSel, text: string): boolean {
  const adapter = adapterFor(sel.gridId);
  if (adapter?.overtype) return adapter.overtype(sel, text);
  if (!cellBlockId(sel)) {
    const made = materializeCell(sel.gridId, sel.row, sel.col);
    if (!made) return true;
  }
  if (!startCellEditing(sel, 0)) return true;
  replaceThroughMountedEditor(sel, text);
  return true;
}

function overtypeCellSelection(sel: CellSel | RangeSel, text: string): boolean {
  return overtypeCell(focusCell(sel), text);
}

function printableKey(e: KeyboardEvent): string | null {
  if (e.ctrlKey || e.metaKey || e.altKey || e.isComposing) return null;
  return e.key.length === 1 ? e.key : null;
}

function pageForGrid(gridId: string): string | null {
  return doc.byId[gridId]?.page ?? null;
}

function seamInsertTarget(sel: RowSeamSel | ColSeamSel): CellSel | null {
  const page = pageForGrid(sel.gridId);
  if (!page) return null;
  let target: CellSel | null = null;
  withUndoUnit("sheet:seam-insert", [page], () => {
    if (sel.kind === "row-seam") {
      const rowId = insertRow(sel.gridId, sel.at);
      if (!rowId) return;
      const col = Math.max(0, sel.anchor.col);
      if (!materializeCell(sel.gridId, sel.at, col)) return;
      target = { kind: "cell", gridId: sel.gridId, row: sel.at, col };
      return;
    }
    insertColumn(sel.gridId, sel.at);
    const row = Math.max(0, sel.anchor.row);
    if (!materializeCell(sel.gridId, row, sel.at)) return;
    target = { kind: "cell", gridId: sel.gridId, row, col: sel.at };
  });
  return target;
}

function editInsertedFromSeam(sel: RowSeamSel | ColSeamSel, text: string | null): boolean {
  const target = seamInsertTarget(sel);
  if (!target) return true;
  if (!startCellEditing(target, 0)) return true;
  if (text !== null) replaceThroughMountedEditor(target, text);
  return true;
}

function nearestAfterRowDelete(gridId: string, deletedRow: number, col: number): CellSel | null {
  const bounds = boundsForGrid(gridId);
  if (bounds.rows <= 0 || bounds.cols <= 0) return null;
  return {
    kind: "cell",
    gridId,
    row: Math.max(0, Math.min(deletedRow, bounds.rows - 1)),
    col: Math.max(0, Math.min(col, bounds.cols - 1)),
  };
}

function nearestAfterColumnDelete(gridId: string, row: number, deletedCol: number): CellSel | null {
  const bounds = boundsForGrid(gridId);
  if (bounds.rows <= 0 || bounds.cols <= 0) return null;
  return {
    kind: "cell",
    gridId,
    row: Math.max(0, Math.min(row, bounds.rows - 1)),
    col: Math.max(0, Math.min(deletedCol, bounds.cols - 1)),
  };
}

function deleteFromSeam(sel: RowSeamSel | ColSeamSel, side: "before" | "after"): boolean {
  if (sel.kind === "row-seam") {
    const row = side === "before" ? sel.at - 1 : sel.at;
    if (row < 0 || row >= rowsForGrid(sel.gridId).length) return true;
    deleteRow(sel.gridId, row);
    const next = nearestAfterRowDelete(sel.gridId, row, sel.anchor.col);
    if (next) setCellSel(next);
    else {
      clearCellSelectionOnly();
      selectBlock(sel.gridId);
    }
    return true;
  }

  const col = side === "before" ? sel.at - 1 : sel.at;
  const bounds = boundsForGrid(sel.gridId);
  if (col < 0 || col >= bounds.cols) return true;
  deleteColumn(sel.gridId, col);
  const next = nearestAfterColumnDelete(sel.gridId, sel.anchor.row, col);
  if (next) setCellSel(next);
  else {
    clearCellSelectionOnly();
    selectBlock(sel.gridId);
  }
  return true;
}

export function handleSheetPasteEvent(e: ClipboardEvent): boolean {
  const sel = cellSel();
  if (!sel || isSeamSel(sel)) return false;
  const text = e.clipboardData?.getData("text/plain") ?? "";
  if (text === "") return false;
  const structural = pasteStructuralSheetSelection(sel, text);
  if (structural !== undefined) {
    if (structural) setCellSel(structural);
    return !!structural;
  }
  const next = pasteTextIntoSheetSelection(sel, text);
  if (next) setCellSel(next);
  return !!next;
}

export function handleCellSelectionKey(e: KeyboardEvent): boolean {
  const sel = cellSel();
  if (!sel) return false;
  const mod = e.ctrlKey || e.metaKey;
  const key = e.key.toLowerCase();
  const plain = !e.ctrlKey && !e.metaKey && !e.altKey && !e.shiftKey;
  const arrowDir = e.key === "ArrowUp" ? "up"
    : e.key === "ArrowDown" ? "down"
    : e.key === "ArrowLeft" ? "left"
    : e.key === "ArrowRight" ? "right"
    : null;

  if (!mod && !e.altKey && e.key === "Escape") {
    if (isRangeSel(sel)) {
      setCellSel({ kind: "cell", gridId: sel.gridId, row: sel.anchor.row, col: sel.anchor.col });
      return true;
    }
    const outer = enclosingCellForGrid(sel.gridId);
    if (outer) {
      setCellSel(outer);
      return true;
    }
    clearCellSelectionOnly();
    selectBlock(sel.gridId);
    return true;
  }
  if (!mod && !e.altKey && e.shiftKey && arrowDir) return extendCellRange(sel, arrowDir);
  if (!mod && !e.altKey && e.shiftKey && (e.key === " " || e.code === "Space")) return selectRows(sel);
  if (mod && !e.altKey && !e.shiftKey && (e.key === " " || e.code === "Space")) return selectColumns(sel);
  if (mod && !e.altKey && !e.shiftKey && key === "a") return selectAllGrid(sel.gridId);
  if (mod && !e.altKey && !e.shiftKey && arrowDir && !isSeamSel(sel)) {
    const cell = focusCell(sel);
    const adapter = adapterFor(cell.gridId);
    if (adapter?.moveWithMod) return adapter.moveWithMod(cell, arrowDir as CellDirection);
    const next = moveSheetSelection(sel, arrowDir as SheetMoveDirection);
    if (next) setCellSel(next);
    return true;
  }
  if (mod && !e.altKey && !e.shiftKey && key === "d" && !isSeamSel(sel)) return fillSheetSelection(sel, "down");
  if (mod && !e.altKey && !e.shiftKey && key === "r" && !isSeamSel(sel)) return fillSheetSelection(sel, "right");
  if (mod && !e.altKey && !e.shiftKey && key === "c" && !isSeamSel(sel)) {
    void copySheetSelection(sel);
    return true;
  }
  if (mod && !e.altKey && !e.shiftKey && key === "x" && !isSeamSel(sel)) {
    cutSheetSelection(sel);
    return true;
  }
  if (plain && e.key === "ArrowUp") return moveCellSelectionFrom(sel, "up");
  if (plain && e.key === "ArrowDown") return moveCellSelectionFrom(sel, "down");
  if (plain && e.key === "ArrowLeft") return moveCellSelectionFrom(sel, "left");
  if (plain && e.key === "ArrowRight") return moveCellSelectionFrom(sel, "right");
  if (plain && isSeamSel(sel) && e.key === "Backspace") return deleteFromSeam(sel, "before");
  if (plain && isSeamSel(sel) && e.key === "Delete") return deleteFromSeam(sel, "after");
  if (plain && (e.key === "Enter" || e.key === "F2")) {
    if (!isSeamSel(sel)) {
      const cell = focusCell(sel);
      const adapter = adapterFor(cell.gridId);
      if (!adapter?.activate?.(cell)) startCellEditing(cell);
    }
    else editInsertedFromSeam(sel, null);
    return true;
  }
  if (!e.ctrlKey && !e.metaKey && !e.altKey && (e.key === "Tab" || e.code === "Tab")) {
    return moveCellTab(sel, e.shiftKey ? -1 : 1);
  }

  const ch = printableKey(e);
  if (ch) return isSeamSel(sel) ? editInsertedFromSeam(sel, ch) : overtypeCellSelection(sel, ch);
  return false;
}

registerOutlineSelectionListener(() => clearCellSelectionOnly());
registerEditingStartListener((_id, owner) => {
  if (!isSheetCellOwner(owner)) clearCellSelectionOnly();
});
registerModeResetListener(() => resetCellSelectionForTests());
