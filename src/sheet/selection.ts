import { createRoot, createSignal } from "solid-js";
import { doc, clearSelection, selectBlock, prevVisible, nextVisible, blockIsGridView, withUndoUnit, blockPageReadOnly, formatForBlock } from "../store";
import { endEdit, startEditing } from "../editorController";
import { isSheetCellHidden, splitProps } from "../editor/properties";
import {
  registerEditingStartListener,
  registerModeResetListener,
  registerOutlineSelectionListener,
} from "../modeHooks";
import { decodeNavIntent, navDirectionForKey, type NavDirection } from "../navProtocol";
import { buildMatrix, type MatrixCell, type SheetMatrix } from "./matrix";
import type { SheetCellCtx } from "./context";
import {
  clearSheetSelection,
  copySheetSelection,
  cutSheetSelection,
  deleteColumn,
  deleteColumns,
  deleteRow,
  deleteRows,
  fillSheetSelection,
  insertColumn,
  insertRow,
  materializeCell,
  moveSheetSelection,
  pasteTextIntoSheetSelection,
  rectForSheetSelection,
  splatStructuralSheetSelection,
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
  surfaceId?: string;
  anchor: SheetPoint;
  focus: SheetPoint;
  anchorRowId?: string;
  focusRowId?: string;
  anchorColumnId?: string;
  focusColumnId?: string;
}

export interface RowSeamSel {
  kind: "row-seam";
  gridId: string;
  surfaceId?: string;
  anchor: SheetPoint;
  col: number;
  at: number;
}

export interface ColSeamSel {
  kind: "col-seam";
  gridId: string;
  surfaceId?: string;
  anchor: SheetPoint;
  row: number;
  at: number;
}

export type SheetSel = CellSel | RangeSel | RowSeamSel | ColSeamSel;
type CellSelInput = SheetCellCtx | CellSel;
type SheetSelInput = SheetSel | CellSelInput;

function withCellMeta<T extends object>(value: T, surfaceId?: string, rowId?: string, columnId?: string): T & { surfaceId?: string; rowId?: string; columnId?: string } {
  const tagged = value as T & { surfaceId?: string; rowId?: string; columnId?: string };
  delete tagged.surfaceId;
  delete tagged.rowId;
  delete tagged.columnId;
  if (surfaceId !== undefined) Object.defineProperty(tagged, "surfaceId", { value: surfaceId, enumerable: false, configurable: true });
  if (rowId !== undefined) Object.defineProperty(tagged, "rowId", { value: rowId, enumerable: false, configurable: true });
  if (columnId !== undefined) Object.defineProperty(tagged, "columnId", { value: columnId, enumerable: false, configurable: true });
  return tagged;
}

function withRangeRows(
  value: RangeSel,
  anchorRowId?: string,
  focusRowId?: string,
  anchorColumnId?: string,
  focusColumnId?: string,
): RangeSel {
  delete value.anchorRowId;
  delete value.focusRowId;
  delete value.anchorColumnId;
  delete value.focusColumnId;
  if (anchorRowId !== undefined) Object.defineProperty(value, "anchorRowId", { value: anchorRowId, enumerable: false, configurable: true });
  if (focusRowId !== undefined) Object.defineProperty(value, "focusRowId", { value: focusRowId, enumerable: false, configurable: true });
  if (anchorColumnId !== undefined) Object.defineProperty(value, "anchorColumnId", { value: anchorColumnId, enumerable: false, configurable: true });
  if (focusColumnId !== undefined) Object.defineProperty(value, "focusColumnId", { value: focusColumnId, enumerable: false, configurable: true });
  return value;
}

function inferUniqueMountedSurface(gridId: string): string | undefined {
  if (typeof document === "undefined") return undefined;
  const esc = typeof CSS !== "undefined" && CSS.escape ? CSS.escape(gridId) : gridId.replace(/["\\]/g, "\\$&");
  const surfaces = new Set(
    [...document.querySelectorAll<HTMLElement>(`[data-sheet-grid-id="${esc}"][data-sheet-surface-id]`)]
      .map((el) => el.dataset.sheetSurfaceId)
      .filter((value): value is string => !!value)
  );
  return surfaces.size === 1 ? [...surfaces][0] : undefined;
}

export type { SheetPoint, SheetRect };

interface CellSelectionHooks {
  clearOutlineSelection: () => void;
  endActiveEdit: () => void;
}

export interface SheetViewAdapter {
  bounds: () => { rows: number; cols: number };
  blockIdAt?: (row: number, col: number, rowId?: string, columnId?: string) => string | null;
  rowIdAt?: (row: number, col?: number) => string | null;
  columnIdAt?: (row: number, col: number) => string | null;
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
const visibilityHooks = new Map<string, (sel: SheetSel) => void>();
const instanceKey = (gridId: string, surfaceId?: string) => `${surfaceId ?? ""}\0${gridId}`;

let hooks: CellSelectionHooks = {
  clearOutlineSelection: clearSelection,
  endActiveEdit: () => endEdit("select-block"),
};
let outlinedGridId: string | null = null;

export function outlinedGridSelectionId(): string | null {
  return outlinedGridId;
}

export function installCellSelectionHooks(next: Partial<CellSelectionHooks>): () => void {
  const prev = hooks;
  hooks = { ...hooks, ...next };
  return () => {
    hooks = prev;
  };
}

export function registerSheetViewAdapter(gridId: string, adapter: SheetViewAdapter, surfaceId?: string): () => void {
  const key = instanceKey(gridId, surfaceId);
  adapters.set(key, adapter);
  return () => {
    if (adapters.get(key) === adapter) adapters.delete(key);
  };
}

export function registerSheetVisibilityHook(gridId: string, hook: (sel: SheetSel) => void, surfaceId?: string): () => void {
  const key = instanceKey(gridId, surfaceId);
  visibilityHooks.set(key, hook);
  return () => {
    if (visibilityHooks.get(key) === hook) visibilityHooks.delete(key);
  };
}

function adapterFor(gridId: string, surfaceId?: string): SheetViewAdapter | null {
  const exact = adapters.get(instanceKey(gridId, surfaceId)) ?? adapters.get(instanceKey(gridId));
  if (exact) return exact;
  if (surfaceId === undefined) {
    const matches = [...adapters].filter(([key]) => key.endsWith(`\0${gridId}`));
    if (matches.length === 1) return matches[0][1];
  }
  return null;
}

function clearCellSelectionOnly(): void {
  writeCellSel(null);
}

export function resetCellSelectionForTests(): void {
  clearCellSelectionOnly();
  outlinedGridId = null;
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
  return sel.surfaceId
    ? `sheet:${sel.surfaceId}:${sel.gridId}:${sel.rowId ?? sel.row}:${sel.columnId ?? sel.col}`
    : `sheet:${sel.gridId}:${sel.rowId ?? sel.row}:${sel.columnId ?? sel.col}`;
}

export function cellSurfaceKey(gridId: string, surfaceId?: string): string {
  return `sheet:${surfaceId ?? ""}:${gridId}`;
}

function isRangeSel(sel: SheetSel | null): sel is RangeSel {
  return sel?.kind === "range";
}

function isSeamSel(sel: SheetSel | null): sel is RowSeamSel | ColSeamSel {
  return sel?.kind === "row-seam" || sel?.kind === "col-seam";
}

export function rowSeamSel(gridId: string, at: number, col: number, surfaceId?: string): RowSeamSel {
  return withCellMeta({ kind: "row-seam", gridId, at, col, anchor: { row: at, col } } as RowSeamSel, surfaceId);
}

export function colSeamSel(gridId: string, at: number, row: number, surfaceId?: string): ColSeamSel {
  return withCellMeta({ kind: "col-seam", gridId, at, row, anchor: { row, col: at } } as ColSeamSel, surfaceId);
}

function normalizeSel(sel: SheetSelInput): SheetSel {
  if ("kind" in sel && sel.kind) {
    if (sel.kind === "row-seam") {
      const s = sel as RowSeamSel & { anchor?: SheetPoint };
      return rowSeamSel(s.gridId, s.at, s.anchor?.col ?? s.col, s.surfaceId);
    }
    if (sel.kind === "col-seam") {
      const s = sel as ColSeamSel & { anchor?: SheetPoint };
      return colSeamSel(s.gridId, s.at, s.anchor?.row ?? s.row, s.surfaceId);
    }
    const normalized = withCellMeta(
      { ...sel, surfaceId: undefined, rowId: undefined, columnId: undefined } as SheetSel,
      sel.surfaceId ?? inferUniqueMountedSurface(sel.gridId),
      "rowId" in sel ? sel.rowId : undefined,
      "columnId" in sel ? sel.columnId : undefined
    );
    return sel.kind === "range"
      ? withRangeRows(normalized as RangeSel, sel.anchorRowId, sel.focusRowId, sel.anchorColumnId, sel.focusColumnId)
      : normalized;
  }
  return withCellMeta(
    { kind: "cell", gridId: sel.gridId, row: sel.row, col: sel.col },
    sel.surfaceId ?? inferUniqueMountedSurface(sel.gridId),
    sel.rowId,
    sel.columnId
  );
}

function rememberSelection(sel: SheetSel): void {
  const key = instanceKey(sel.gridId, sel.surfaceId);
  if (sel.kind === "cell") lastByGrid.set(key, { row: sel.row, col: sel.col });
  else if (sel.kind === "range") lastByGrid.set(key, { ...sel.focus });
}

export function cellSel(): SheetSel | null {
  return activeCellSel();
}

export function setCellSel(sel: SheetSelInput | null): void {
  if (sel) {
    const normalized = normalizeSel(sel);
    visibilityHooks.get(instanceKey(normalized.gridId, normalized.surfaceId))?.(normalized);
    rememberSelection(normalized);
    hooks.clearOutlineSelection();
    hooks.endActiveEdit();
    writeCellSel(normalized);
    return;
  }
  writeCellSel(null);
}

/** Re-anchor one selected table cell after reactive sorting without ending its edit. */
export function rebaseSelectedCell(
  gridId: string,
  surfaceId: string | undefined,
  rowId: string,
  point: SheetPoint,
  columnId?: string,
): void {
  const sel = activeCellSel();
  if (sel?.kind !== "cell" || sel.gridId !== gridId || sel.surfaceId !== surfaceId || sel.rowId !== rowId ||
      (sel.row === point.row && sel.col === point.col)) return;
  writeCellSel(withCellMeta(
    { ...sel, surfaceId: undefined, rowId: undefined, columnId: undefined, ...point },
    surfaceId,
    rowId,
    columnId ?? sel.columnId
  ));
}

export function rebaseSelectedRange(
  gridId: string,
  surfaceId: string | undefined,
  anchorRowId: string,
  anchor: SheetPoint,
  focusRowId: string,
  focus: SheetPoint,
  anchorColumnId?: string,
  focusColumnId?: string,
): void {
  const sel = activeCellSel();
  if (sel?.kind !== "range" || sel.gridId !== gridId || sel.surfaceId !== surfaceId ||
      sel.anchorRowId !== anchorRowId || sel.focusRowId !== focusRowId ||
      (sel.anchor.row === anchor.row && sel.anchor.col === anchor.col &&
       sel.focus.row === focus.row && sel.focus.col === focus.col)) return;
  writeCellSel(withCellMeta(withRangeRows({
    ...sel,
    anchor,
    focus,
  }, anchorRowId, focusRowId, anchorColumnId ?? sel.anchorColumnId, focusColumnId ?? sel.focusColumnId), surfaceId));
}

export function clearSelectedSheetInstance(gridId: string, surfaceId?: string): void {
  const sel = activeCellSel();
  if (sel?.gridId === gridId && sel.surfaceId === surfaceId) writeCellSel(null);
}

export function lastCellFor(gridId: string, surfaceId?: string): { row: number; col: number } | null {
  const last = lastByGrid.get(instanceKey(gridId, surfaceId));
  return last ? { ...last } : null;
}

function rowsForGrid(gridId: string): { id: string; cellIds: readonly string[] }[] {
  return (doc.byId[gridId]?.children ?? []).map((id) => ({
    id,
    cellIds: doc.byId[id]?.children ?? [],
  }));
}

export function matrixForGrid(gridId: string): SheetMatrix {
  return buildMatrix(rowsForGrid(gridId), { visibleRows: 0, visibleCols: 0 });
}

function boundsForGrid(gridId: string, surfaceId?: string): { rows: number; cols: number } {
  const adapter = adapterFor(gridId, surfaceId);
  if (adapter) return adapter.bounds();
  const matrix = matrixForGrid(gridId);
  return { rows: matrix.rows, cols: matrix.rows === 0 ? 0 : matrix.cols };
}

export function cellAt(sel: CellSelInput): MatrixCell | null {
  const rows = doc.byId[sel.gridId]?.children ?? [];
  if (sel.row < 0 || sel.row >= rows.length || sel.col < 0) return null;
  let cols = 1;
  for (const rowId of rows) cols = Math.max(cols, doc.byId[rowId]?.children.length ?? 0);
  if (sel.col >= cols) return null;
  const blockId = doc.byId[rows[sel.row]]?.children[sel.col] ?? null;
  return { blockId, row: sel.row, col: sel.col, rowSpan: 1, colSpan: 1 };
}

export function cellBlockId(sel: CellSelInput): string | null {
  const adapter = adapterFor(sel.gridId, sel.surfaceId);
  if (adapter?.blockIdAt) return adapter.blockIdAt(sel.row, sel.col, sel.rowId, sel.columnId);
  return cellAt(sel)?.blockId ?? null;
}

export function cellForBlockId(blockId: string, preferredSurfaceId?: string): CellSel | null {
  const rowId = doc.byId[blockId]?.parent ?? null;
  const gridId = rowId ? doc.byId[rowId]?.parent ?? null : null;
  if (gridId && blockIsGridView(gridId)) {
    const row = doc.byId[gridId]?.children.indexOf(rowId!) ?? -1;
    const col = doc.byId[rowId!]?.children.indexOf(blockId) ?? -1;
    if (row >= 0 && col >= 0) return withCellMeta(
      { kind: "cell", gridId, row, col } as CellSel,
      preferredSurfaceId ?? inferUniqueMountedSurface(gridId)
    );
  }
  for (const [key, adapter] of adapters) {
    if (preferredSurfaceId !== undefined && !key.startsWith(`${preferredSurfaceId}\0`)) continue;
    const cell = adapter.cellForBlock?.(blockId);
    if (cell) return cell;
  }
  return null;
}

function enclosingCellForGrid(gridId: string, nestedSurfaceId?: string): CellSel | null {
  const rowId = doc.byId[gridId]?.parent ?? null;
  const outerGridId = rowId ? doc.byId[rowId]?.parent ?? null : null;
  let parentSurfaceId = nestedSurfaceId;
  if (outerGridId && nestedSurfaceId?.startsWith("sheet:")) {
    const suffix = `:${outerGridId}`;
    if (nestedSurfaceId.endsWith(suffix)) {
      parentSurfaceId = nestedSurfaceId.slice("sheet:".length, -suffix.length);
    }
  }
  return cellForBlockId(gridId, parentSurfaceId);
}

export function focusCell(sel: CellSel | RangeSel): CellSel {
  if (sel.kind === "cell") return sel;
  return withCellMeta(
    { kind: "cell", gridId: sel.gridId, row: sel.focus.row, col: sel.focus.col } as CellSel,
    sel.surfaceId,
    sel.focusRowId,
    sel.focusColumnId
  );
}

export function sheetSelectionRect(sel: SheetSel | null): SheetRect | null {
  if (!sel || isSeamSel(sel)) return null;
  return rectForSheetSelection(sel);
}

export function sheetSelectionRectForGrid(gridId: string, surfaceId?: string): SheetRect | null {
  const sel = cellSel();
  if (!sel || sel.gridId !== gridId || sel.surfaceId !== surfaceId || isSeamSel(sel)) return null;
  return rectForSheetSelection(sel);
}

export function cellIsInRange(gridId: string, row: number, col: number, surfaceId?: string): boolean {
  const rect = sheetSelectionRectForGrid(gridId, surfaceId);
  return !!rect && row >= rect.top && row <= rect.bottom && col >= rect.left && col <= rect.right;
}

function clampCell(gridId: string, wanted: { row: number; col: number }, surfaceId?: string): CellSel | null {
  const bounds = boundsForGrid(gridId, surfaceId);
  if (bounds.rows <= 0 || bounds.cols <= 0) return null;
  const row = Math.max(0, Math.min(wanted.row, bounds.rows - 1));
  const col = Math.max(0, Math.min(wanted.col, bounds.cols - 1));
  return withCellMeta({
    kind: "cell",
    gridId,
    row,
    col,
  } as CellSel,
  surfaceId,
  adapterFor(gridId, surfaceId)?.rowIdAt?.(row, col) ?? undefined,
  adapterFor(gridId, surfaceId)?.columnIdAt?.(row, col) ?? undefined);
}

function hasAdapter(gridId: string, surfaceId?: string): boolean {
  return adapterFor(gridId, surfaceId) !== null;
}

function clampPoint(gridId: string, wanted: SheetPoint, surfaceId?: string): SheetPoint | null {
  const cell = clampCell(gridId, wanted, surfaceId);
  return cell ? { row: cell.row, col: cell.col } : null;
}

function setRangeOrCell(gridId: string, anchor: SheetPoint, focus: SheetPoint, surfaceId?: string): boolean {
  const a = clampPoint(gridId, anchor, surfaceId);
  const f = clampPoint(gridId, focus, surfaceId);
  if (!a || !f) return false;
  if (a.row === f.row && a.col === f.col) {
    setCellSel(withCellMeta(
      { kind: "cell", gridId, row: f.row, col: f.col } as CellSel,
      surfaceId,
      adapterFor(gridId, surfaceId)?.rowIdAt?.(f.row, f.col) ?? undefined,
      adapterFor(gridId, surfaceId)?.columnIdAt?.(f.row, f.col) ?? undefined
    ));
  }
  else {
    const adapter = adapterFor(gridId, surfaceId);
    setCellSel(withCellMeta(withRangeRows(
      { kind: "range", gridId, anchor: a, focus: f },
      adapter?.rowIdAt?.(a.row, a.col) ?? undefined,
      adapter?.rowIdAt?.(f.row, f.col) ?? undefined,
      adapter?.columnIdAt?.(a.row, a.col) ?? undefined,
      adapter?.columnIdAt?.(f.row, f.col) ?? undefined,
    ), surfaceId));
  }
  return true;
}

export function setCellRangeSelection(gridId: string, anchor: SheetPoint, focus: SheetPoint, surfaceId?: string): boolean {
  return setRangeOrCell(gridId, anchor, focus, surfaceId);
}

export function extendCellSelectionTo(gridId: string, focus: SheetPoint, surfaceId?: string): boolean {
  const sel = cellSel();
  const anchor =
    sel && sel.gridId === gridId && (sel.surfaceId === surfaceId || sel.surfaceId === undefined) && !isSeamSel(sel)
      ? sel.kind === "range"
        ? sel.anchor
        : { row: sel.row, col: sel.col }
      : focus;
  return setRangeOrCell(gridId, anchor, focus, surfaceId);
}

export function selectTopRowSeam(gridId: string, col = 0, surfaceId?: string): boolean {
  const bounds = boundsForGrid(gridId, surfaceId);
  if (bounds.rows <= 0 || bounds.cols <= 0) return false;
  setCellSel(rowSeamSel(gridId, 0, Math.max(0, Math.min(col, bounds.cols - 1)), surfaceId));
  return true;
}

export function selectTopRowSeamAfterEdit(gridId: string, col = 0, surfaceId?: string): boolean {
  endEdit("select-block");
  return selectTopRowSeam(gridId, col, surfaceId);
}

export function enterGridSelection(gridId: string, surfaceId?: string): boolean {
  if (!blockIsGridView(gridId)) return false;
  const target = clampCell(gridId, lastCellFor(gridId, surfaceId) ?? { row: 0, col: 0 }, surfaceId);
  if (!target) return false;
  setCellSel(target);
  return true;
}

export function startCellEditing(sel: CellSelInput, offset?: number): boolean {
  const scoped = withCellMeta(
    { kind: "cell", gridId: sel.gridId, row: sel.row, col: sel.col } as CellSel,
    sel.surfaceId ?? inferUniqueMountedSurface(sel.gridId),
    sel.rowId,
    sel.columnId
  );
  const adapter = adapterFor(scoped.gridId, scoped.surfaceId);
  if (adapter?.startEditing) return adapter.startEditing(scoped, offset);
  const blockId = cellBlockId(scoped);
  if (!blockId) return false;
  if (blockPageReadOnly(blockId)) return false; // org round-trip gate (review finding)
  const node = doc.byId[blockId];
  if (!node) return false;
  const visibleLen = splitProps(node.raw, isSheetCellHidden, formatForBlock(blockId)).visible.length;
  setCellSel(scoped);
  startEditing(blockId, offset ?? visibleLen, cellOwner(scoped));
  return true;
}

export function selectCellAfterEdit(sel: SheetCellCtx): void {
  endEdit("select-block");
  setCellSel(sel);
}

// The direction vocabulary IS the shared nav protocol's (ADR 0034).
type CellDirection = NavDirection;

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

function setClampedCell(gridId: string, row: number, col: number, surfaceId?: string): boolean {
  const next = clampCell(gridId, { row, col }, surfaceId);
  if (!next) return false;
  setCellSel(next);
  return true;
}

function moveFromRowSeam(sel: RowSeamSel, dir: CellDirection): boolean {
  const bounds = boundsForGrid(sel.gridId, sel.surfaceId);
  if (bounds.rows <= 0 || bounds.cols <= 0) return false;

  if (dir === "up") {
    if (sel.at <= 0) return true;
    return setClampedCell(sel.gridId, sel.at - 1, sel.anchor.col, sel.surfaceId);
  }
  if (dir === "down") {
    if (sel.at >= bounds.rows) return true;
    return setClampedCell(sel.gridId, sel.at, sel.anchor.col, sel.surfaceId);
  }
  if (dir === "left") {
    setCellSel(rowSeamSel(sel.gridId, sel.at, Math.max(0, sel.anchor.col - 1), sel.surfaceId));
    return true;
  }
  setCellSel(rowSeamSel(sel.gridId, sel.at, Math.min(bounds.cols - 1, sel.anchor.col + 1), sel.surfaceId));
  return true;
}

function moveFromColSeam(sel: ColSeamSel, dir: CellDirection): boolean {
  const bounds = boundsForGrid(sel.gridId, sel.surfaceId);
  if (bounds.rows <= 0 || bounds.cols <= 0) return false;

  if (dir === "left") {
    if (sel.at <= 0) return true;
    return setClampedCell(sel.gridId, sel.anchor.row, sel.at - 1, sel.surfaceId);
  }
  if (dir === "right") {
    if (sel.at >= bounds.cols) return true;
    return setClampedCell(sel.gridId, sel.anchor.row, sel.at, sel.surfaceId);
  }
  if (dir === "up") {
    setCellSel(colSeamSel(sel.gridId, sel.at, Math.max(0, sel.anchor.row - 1), sel.surfaceId));
    return true;
  }
  setCellSel(colSeamSel(sel.gridId, sel.at, Math.min(bounds.rows - 1, sel.anchor.row + 1), sel.surfaceId));
  return true;
}

export function moveCellSelectionFrom(sel: SheetSel, dir: CellDirection): boolean {
  if (sel.kind === "row-seam") return moveFromRowSeam(sel, dir);
  if (sel.kind === "col-seam") return moveFromColSeam(sel, dir);
  if (sel.kind === "range") return moveCellSelectionFrom(focusCell(sel), dir);

  const bounds = boundsForGrid(sel.gridId, sel.surfaceId);
  if (bounds.rows <= 0 || bounds.cols <= 0) return false;
  if (hasAdapter(sel.gridId, sel.surfaceId) && !blockIsGridView(sel.gridId)) {
    if (dir === "up") {
      if (sel.row <= 0) return flowOutVertical(sel, "up");
      return setClampedCell(sel.gridId, sel.row - 1, sel.col, sel.surfaceId);
    }
    if (dir === "down") {
      if (sel.row >= bounds.rows - 1) return flowOutVertical(sel, "down");
      return setClampedCell(sel.gridId, sel.row + 1, sel.col, sel.surfaceId);
    }
    if (dir === "left") {
      if (sel.col <= 0) return exitLeft(sel);
      return setClampedCell(sel.gridId, sel.row, sel.col - 1, sel.surfaceId);
    }
    if (sel.col >= bounds.cols - 1) return true;
    return setClampedCell(sel.gridId, sel.row, sel.col + 1, sel.surfaceId);
  }

  if (dir === "up") {
    if (SEAM_STEPPING) {
      setCellSel(rowSeamSel(sel.gridId, sel.row, sel.col, sel.surfaceId));
      return true;
    }
    if (sel.row <= 0) return flowOutVertical(sel, "up");
    setCellSel({ ...sel, row: sel.row - 1 });
    return true;
  }
  if (dir === "down") {
    if (SEAM_STEPPING) {
      setCellSel(rowSeamSel(sel.gridId, sel.row + 1, sel.col, sel.surfaceId));
      return true;
    }
    if (sel.row >= bounds.rows - 1) return flowOutVertical(sel, "down");
    setCellSel({ ...sel, row: sel.row + 1 });
    return true;
  }
  if (dir === "left") {
    if (SEAM_STEPPING) {
      setCellSel(colSeamSel(sel.gridId, sel.col, sel.row, sel.surfaceId));
      return true;
    }
    if (sel.col <= 0) return exitLeft(sel);
    setCellSel({ ...sel, col: sel.col - 1 });
    return true;
  }
  if (SEAM_STEPPING) {
    setCellSel(colSeamSel(sel.gridId, sel.col + 1, sel.row, sel.surfaceId));
    return true;
  }
  if (sel.col >= bounds.cols - 1) return true;
  setCellSel({ ...sel, col: sel.col + 1 });
  return true;
}

function moveCellTab(sel: SheetSel, dir: 1 | -1): boolean {
  if (isSeamSel(sel)) return true;
  const cell = focusCell(sel);
  const bounds = boundsForGrid(cell.gridId, cell.surfaceId);
  if (bounds.rows <= 0 || bounds.cols <= 0) return false;
  const next = cell.row * bounds.cols + cell.col + dir;
  if (next < 0 || next >= bounds.rows * bounds.cols) return true;
  return setClampedCell(cell.gridId, Math.floor(next / bounds.cols), next % bounds.cols, cell.surfaceId);
}

// Shift+arrow while a seam is selected starts a range from the boundary (the
// nit "shift-select works from cells but not edges"). A perpendicular arrow
// grabs the two cells the seam straddles, anchored on the far side (mirroring a
// cell's shift-extend); an arrow along the seam resolves it to the cell on the
// line and extends from there. setRangeOrCell clamps and collapses at edges.
function extendFromSeam(sel: RowSeamSel | ColSeamSel, dir: CellDirection): boolean {
  if (sel.kind === "row-seam") {
    const col = sel.anchor.col;
    if (dir === "up" || dir === "down") {
      const above = { row: sel.at - 1, col };
      const below = { row: sel.at, col };
      return dir === "up"
        ? setRangeOrCell(sel.gridId, below, above, sel.surfaceId)
        : setRangeOrCell(sel.gridId, above, below, sel.surfaceId);
    }
    const row = sel.at;
    return setRangeOrCell(sel.gridId, { row, col }, { row, col: col + (dir === "right" ? 1 : -1) }, sel.surfaceId);
  }
  const row = sel.anchor.row;
  if (dir === "left" || dir === "right") {
    const left = { row, col: sel.at - 1 };
    const right = { row, col: sel.at };
    return dir === "left"
      ? setRangeOrCell(sel.gridId, right, left, sel.surfaceId)
      : setRangeOrCell(sel.gridId, left, right, sel.surfaceId);
  }
  const col = sel.at;
  return setRangeOrCell(sel.gridId, { row, col }, { row: row + (dir === "down" ? 1 : -1), col }, sel.surfaceId);
}

function extendCellRange(sel: SheetSel, dir: CellDirection): boolean {
  if (isSeamSel(sel)) return extendFromSeam(sel, dir);
  const anchor = sel.kind === "range" ? sel.anchor : { row: sel.row, col: sel.col };
  const cur = sel.kind === "range" ? sel.focus : { row: sel.row, col: sel.col };
  const wanted =
    dir === "up" ? { row: cur.row - 1, col: cur.col }
    : dir === "down" ? { row: cur.row + 1, col: cur.col }
    : dir === "left" ? { row: cur.row, col: cur.col - 1 }
    : { row: cur.row, col: cur.col + 1 };
  return setRangeOrCell(sel.gridId, anchor, wanted, sel.surfaceId);
}

function selectRows(sel: SheetSel): boolean {
  if (isSeamSel(sel)) return true;
  const bounds = boundsForGrid(sel.gridId, sel.surfaceId);
  if (bounds.rows <= 0 || bounds.cols <= 0) return false;
  const rect = rectForSheetSelection(sel);
  return setRangeOrCell(
    sel.gridId,
    { row: rect.top, col: 0 },
    { row: rect.bottom, col: bounds.cols - 1 },
    sel.surfaceId
  );
}

function selectColumns(sel: SheetSel): boolean {
  if (isSeamSel(sel)) return true;
  const bounds = boundsForGrid(sel.gridId, sel.surfaceId);
  if (bounds.rows <= 0 || bounds.cols <= 0) return false;
  const rect = rectForSheetSelection(sel);
  return setRangeOrCell(
    sel.gridId,
    { row: 0, col: rect.left },
    { row: bounds.rows - 1, col: rect.right },
    sel.surfaceId
  );
}

function selectAllGrid(gridId: string, surfaceId?: string): boolean {
  const bounds = boundsForGrid(gridId, surfaceId);
  if (bounds.rows <= 0 || bounds.cols <= 0) return false;
  return setRangeOrCell(gridId, { row: 0, col: 0 }, { row: bounds.rows - 1, col: bounds.cols - 1 }, surfaceId);
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
  const surface = sel.surfaceId ? `[data-sheet-surface-id="${esc(sel.surfaceId)}"]` : "";
  return document.querySelector(
    `.sheet-cell[data-sheet-grid-id="${esc(sel.gridId)}"]${surface}[data-row="${sel.row}"][data-col="${sel.col}"]`
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
  const adapter = adapterFor(sel.gridId, sel.surfaceId);
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
      target = withCellMeta({ kind: "cell", gridId: sel.gridId, row: sel.at, col } as CellSel, sel.surfaceId);
      return;
    }
    insertColumn(sel.gridId, sel.at);
    const row = Math.max(0, sel.anchor.row);
    if (!materializeCell(sel.gridId, row, sel.at)) return;
    target = withCellMeta({ kind: "cell", gridId: sel.gridId, row, col: sel.at } as CellSel, sel.surfaceId);
  });
  return target;
}

export function growSheetEdge(gridId: string, edge: "row" | "col", surfaceId?: string): CellSel | null {
  const b = boundsForGrid(gridId, surfaceId);
  const target =
    edge === "col"
      ? seamInsertTarget(colSeamSel(gridId, b.cols, 0, surfaceId))
      : seamInsertTarget(rowSeamSel(gridId, b.rows, 0, surfaceId));
  if (!target) return null;
  setCellSel(target);
  return target;
}

function editInsertedFromSeam(sel: RowSeamSel | ColSeamSel, text: string | null): boolean {
  const target = seamInsertTarget(sel);
  if (!target) return true;
  if (!startCellEditing(target, 0)) return true;
  if (text !== null) replaceThroughMountedEditor(target, text);
  return true;
}

function nearestAfterRowDelete(gridId: string, deletedRow: number, col: number, surfaceId?: string): CellSel | null {
  const bounds = boundsForGrid(gridId, surfaceId);
  if (bounds.rows <= 0 || bounds.cols <= 0) return null;
  return withCellMeta({
    kind: "cell",
    gridId,
    row: Math.max(0, Math.min(deletedRow, bounds.rows - 1)),
    col: Math.max(0, Math.min(col, bounds.cols - 1)),
  } as CellSel, surfaceId);
}

function nearestAfterColumnDelete(gridId: string, row: number, deletedCol: number, surfaceId?: string): CellSel | null {
  const bounds = boundsForGrid(gridId, surfaceId);
  if (bounds.rows <= 0 || bounds.cols <= 0) return null;
  return withCellMeta({
    kind: "cell",
    gridId,
    row: Math.max(0, Math.min(row, bounds.rows - 1)),
    col: Math.max(0, Math.min(deletedCol, bounds.cols - 1)),
  } as CellSel, surfaceId);
}

function reselectAfterRemoval(gridId: string, row: number, col: number, surfaceId?: string): void {
  const next = clampCell(gridId, { row, col }, surfaceId);
  if (next) setCellSel(next);
  else {
    clearCellSelectionOnly();
    selectBlock(gridId);
  }
}

// Backspace/Delete on a cell selection. An explicit range that spans a full
// axis (whole row(s) or column(s)) REMOVES that structure; anything else — a
// lone cell (even in a 1×N grid, hence the range-only gate), or a partial
// block — clears contents. Martin's rule: selection shape disambiguates, so
// there's no separate "delete row" command to reach for.
function removeCellsOrClear(sel: CellSel | RangeSel): boolean {
  const bounds = boundsForGrid(sel.gridId, sel.surfaceId);
  if (bounds.rows <= 0 || bounds.cols <= 0) return false;
  if (sel.kind === "range") {
    const rect = rectForSheetSelection(sel);
    const fullRows = rect.left === 0 && rect.right === bounds.cols - 1;
    const fullCols = rect.top === 0 && rect.bottom === bounds.rows - 1;
    if (fullRows) {
      deleteRows(sel.gridId, rect.top, rect.bottom);
      reselectAfterRemoval(sel.gridId, rect.top, 0, sel.surfaceId);
      return true;
    }
    if (fullCols) {
      deleteColumns(sel.gridId, rect.left, rect.right);
      reselectAfterRemoval(sel.gridId, 0, rect.left, sel.surfaceId);
      return true;
    }
  }
  return clearSheetSelection(sel);
}

function deleteFromSeam(sel: RowSeamSel | ColSeamSel, side: "before" | "after"): boolean {
  if (sel.kind === "row-seam") {
    const row = side === "before" ? sel.at - 1 : sel.at;
    if (row < 0 || row >= rowsForGrid(sel.gridId).length) return true;
    deleteRow(sel.gridId, row);
    const next = nearestAfterRowDelete(sel.gridId, row, sel.anchor.col, sel.surfaceId);
    if (next) setCellSel(next);
    else {
      clearCellSelectionOnly();
      selectBlock(sel.gridId);
    }
    return true;
  }

  const col = side === "before" ? sel.at - 1 : sel.at;
  const bounds = boundsForGrid(sel.gridId, sel.surfaceId);
  if (col < 0 || col >= bounds.cols) return true;
  deleteColumn(sel.gridId, col);
  const next = nearestAfterColumnDelete(sel.gridId, sel.anchor.row, col, sel.surfaceId);
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
  const structural = splatStructuralSheetSelection(sel, text);
  if (structural !== undefined) {
    if (structural) setCellSel(withCellMeta(structural, sel.surfaceId));
    return !!structural;
  }
  const next = pasteTextIntoSheetSelection(sel, text);
  if (next) setCellSel(withCellMeta(next, sel.surfaceId));
  return !!next;
}

export function handleCellSelectionKey(e: KeyboardEvent): boolean {
  const sel = cellSel();
  if (!sel) return false;
  const mod = e.ctrlKey || e.metaKey;
  const key = e.key.toLowerCase();
  const arrowDir = navDirectionForKey(e.key);

  // Sheet-specific chords first — mod/alt combos and Tab are surface commands,
  // not navigation (the shared decoder declines them; ADR 0034).
  if (!mod && !e.altKey && e.shiftKey && (e.key === " " || e.code === "Space")) return selectRows(sel);
  if (mod && !e.altKey && !e.shiftKey && (e.key === " " || e.code === "Space")) return selectColumns(sel);
  if (mod && !e.altKey && !e.shiftKey && key === "a") return selectAllGrid(sel.gridId, sel.surfaceId);
  if (mod && !e.altKey && !e.shiftKey && arrowDir && !isSeamSel(sel)) {
    const cell = focusCell(sel);
    const adapter = adapterFor(cell.gridId, cell.surfaceId);
    if (adapter?.moveWithMod) return adapter.moveWithMod(cell, arrowDir);
    const next = moveSheetSelection(sel, arrowDir as SheetMoveDirection);
    if (next) {
      const rowId = next.kind === "cell"
        ? adapterFor(next.gridId, sel.surfaceId)?.rowIdAt?.(next.row) ?? undefined
        : undefined;
      setCellSel(withCellMeta(next, sel.surfaceId, rowId));
    }
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
  if (!e.ctrlKey && !e.metaKey && !e.altKey && (e.key === "Tab" || e.code === "Tab")) {
    return moveCellTab(sel, e.shiftKey ? -1 : 1);
  }

  const intent = decodeNavIntent(e, { acceptF2: true });
  if (!intent) return false;
  switch (intent.kind) {
    case "dismiss": {
      // Down one rung: range → its anchor cell → the enclosing cell (hosted
      // subgrid ascent) → the grid's outline block.
      if (isRangeSel(sel)) {
        setCellSel(withCellMeta(
          { kind: "cell", gridId: sel.gridId, row: sel.anchor.row, col: sel.anchor.col } as CellSel,
          sel.surfaceId,
          sel.anchorRowId
        ));
        return true;
      }
      const outer = enclosingCellForGrid(sel.gridId, sel.surfaceId);
      if (outer) {
        setCellSel(outer);
        return true;
      }
      clearCellSelectionOnly();
      selectBlock(sel.gridId);
      return true;
    }
    case "extend":
      return extendCellRange(sel, intent.dir);
    case "step":
      return moveCellSelectionFrom(sel, intent.dir);
    case "remove":
      return isSeamSel(sel) ? deleteFromSeam(sel, intent.side) : removeCellsOrClear(sel);
    case "activate":
      if (!isSeamSel(sel)) {
        const cell = focusCell(sel);
        const adapter = adapterFor(cell.gridId, cell.surfaceId);
        if (!adapter?.activate?.(cell)) startCellEditing(cell);
      } else editInsertedFromSeam(sel, null);
      return true;
    case "overtype":
      return isSeamSel(sel) ? editInsertedFromSeam(sel, intent.char) : overtypeCellSelection(sel, intent.char);
  }
}

registerOutlineSelectionListener((id) => {
  outlinedGridId = blockIsGridView(id) ? id : null;
  clearCellSelectionOnly();
});
registerEditingStartListener((_id, owner) => {
  if (!isSheetCellOwner(owner)) clearCellSelectionOnly();
});
registerModeResetListener(() => {
  outlinedGridId = null;
  resetCellSelectionForTests();
});
