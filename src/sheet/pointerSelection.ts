import type { SheetCellCtx } from "./context";
import { extendCellSelectionTo, setCellRangeSelection, setCellSel } from "./selection";

const DRAG_THRESHOLD_PX = 4;
const INTERACTIVE_SELECTOR = "textarea, input, select, button, a, [contenteditable='true']";

export function sheetGridIdFromEventTarget(target: EventTarget | null): string | null {
  const el = target instanceof Element ? target.closest("[data-sheet-grid-id]") as HTMLElement | null : null;
  return el?.dataset.sheetGridId ?? null;
}

export function isSheetPointerInteractive(target: EventTarget | null): boolean {
  return target instanceof Element && !!target.closest(INTERACTIVE_SELECTOR);
}

export function sheetCellFromEventTarget(target: EventTarget | null, gridId: string): SheetCellCtx | null {
  if (!(target instanceof Element)) return null;
  const cell = target.closest(".sheet-cell[data-sheet-grid-id][data-row][data-col]") as HTMLElement | null;
  if (!cell || cell.dataset.sheetGridId !== gridId) return null;
  const row = Number(cell.dataset.row);
  const col = Number(cell.dataset.col);
  if (!Number.isInteger(row) || !Number.isInteger(col)) return null;
  return {
    gridId,
    surfaceId: cell.dataset.sheetSurfaceId,
    rowId: cell.dataset.sheetRowId,
    columnId: cell.dataset.sheetColumnId,
    row,
    col,
  };
}

export function beginCellPointerSelection(e: PointerEvent, gridId: string): boolean {
  if (e.button !== 0 || e.ctrlKey || e.metaKey || e.altKey) return false;
  if (isSheetPointerInteractive(e.target)) return false;
  const anchor = sheetCellFromEventTarget(e.target, gridId);
  if (!anchor) return false;

  e.preventDefault();
  e.stopPropagation();

  if (e.shiftKey) {
    extendCellSelectionTo(gridId, { row: anchor.row, col: anchor.col }, anchor.surfaceId);
    return true;
  }

  setCellSel(anchor);

  const startX = e.clientX;
  const startY = e.clientY;
  let moved = false;
  let lastRow = anchor.row;
  let lastCol = anchor.col;

  const focusAt = (ev: PointerEvent): SheetCellCtx | null => {
    const target = document.elementFromPoint(ev.clientX, ev.clientY);
    return sheetCellFromEventTarget(target, gridId);
  };
  const updateFocus = (focus: SheetCellCtx): void => {
    if (focus.row === lastRow && focus.col === lastCol) return;
    lastRow = focus.row;
    lastCol = focus.col;
    setCellRangeSelection(gridId, { row: anchor.row, col: anchor.col }, { row: focus.row, col: focus.col }, anchor.surfaceId);
  };
  const removeListeners = () => {
    window.removeEventListener("pointermove", onMove, true);
    window.removeEventListener("pointerup", onUp, true);
    window.removeEventListener("pointercancel", onCancel, true);
  };
  const onMove = (ev: PointerEvent) => {
    if (!moved && Math.hypot(ev.clientX - startX, ev.clientY - startY) < DRAG_THRESHOLD_PX) return;
    moved = true;
    const focus = focusAt(ev);
    if (focus) updateFocus(focus);
    window.getSelection()?.removeAllRanges();
    ev.preventDefault();
  };
  const onUp = (ev: PointerEvent) => {
    removeListeners();
    if (moved) ev.preventDefault();
  };
  const onCancel = () => {
    removeListeners();
  };

  window.addEventListener("pointermove", onMove, true);
  window.addEventListener("pointerup", onUp, true);
  window.addEventListener("pointercancel", onCancel, true);
  return true;
}
