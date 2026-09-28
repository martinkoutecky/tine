// Pointer-based row reorder (port of master GH #211 rowReorder.ts, used by the
// favorites tree). HTML5 DnD is unreliable in WebKitGTK, so: pointerdown arms,
// a 4px move starts the drag, the drop row comes from elementFromPoint via
// `data-row-index`, and the click that ends a drag is swallowed. While a drag
// runs the document is unselectable (WebKit otherwise smears a text selection
// across every row the pointer crosses).
const DRAG_THRESHOLD_PX = 4;
const SELECTION_CLASS = "drag-selection-suppressed";
let suppressClick = false;

/** True for the click that ends a reorder drag; row click handlers bail. */
export const rowReorderClickSuppressed = () => suppressClick;

export interface RowDropTarget {
  index: number;
  before: boolean;
  /** Pointer x relative to where the drag STARTED (not the row's edge), so
   *  where the row was grabbed never decides a nesting depth. */
  dx: number;
}

function dropSelection(): void {
  const selection = document.getSelection?.();
  if (selection && selection.rangeCount > 0) selection.removeAllRanges();
}
function suppressSelection(on: boolean): void {
  document.documentElement.classList.toggle(SELECTION_CLASS, on);
  if (on) dropSelection();
}

/** Attach a reorder drag to a row's pointerdown. `onTarget` reports the live
 *  drop target (or null); `commit` receives the final one. */
export function beginRowReorderDrag(
  event: PointerEvent,
  rowSelector: string,
  onTarget: (target: RowDropTarget | null) => void,
  commit: (target: RowDropTarget) => void,
): void {
  if (event.button !== 0) return;
  const startX = event.clientX;
  const startY = event.clientY;
  let dragging = false;
  let target: RowDropTarget | null = null;
  const onMove = (ev: PointerEvent) => {
    if (!dragging) {
      if (Math.hypot(ev.clientX - startX, ev.clientY - startY) < DRAG_THRESHOLD_PX) return;
      dragging = true;
      suppressSelection(true);
    }
    dropSelection(); // WebKit can re-anchor a selection mid-drag
    const row = document.elementFromPoint(ev.clientX, ev.clientY)?.closest<HTMLElement>(rowSelector);
    if (row?.dataset.rowIndex !== undefined) {
      const rect = row.getBoundingClientRect();
      target = { index: Number(row.dataset.rowIndex), before: ev.clientY < rect.top + rect.height / 2, dx: ev.clientX - startX };
    } else target = null;
    onTarget(target);
  };
  const cleanup = () => {
    document.removeEventListener("pointermove", onMove);
    document.removeEventListener("pointerup", onUp);
    document.removeEventListener("pointercancel", cleanup);
    suppressSelection(false);
    onTarget(null);
  };
  const onUp = () => {
    cleanup();
    if (!dragging) return;
    suppressClick = true;
    setTimeout(() => { suppressClick = false; }, 0);
    if (target) commit(target);
  };
  document.addEventListener("pointermove", onMove);
  document.addEventListener("pointerup", onUp);
  document.addEventListener("pointercancel", cleanup);
}
