// Pointer-based row reorder (port of master GH #211 rowReorder.ts, used by the
// favorites tree). HTML5 DnD is unreliable in WebKitGTK, so: pointerdown arms,
// a 4px move starts the drag, the drop row comes from elementFromPoint via
// `data-row-index`, and the click that ends a drag is swallowed. While a drag
// runs the document is unselectable (WebKit otherwise smears a text selection
// across every row the pointer crosses) — see dragSelectionGuard.ts, which owns
// the mechanism this and the outline bullet drag (GH #424) both use.
import { dropSelection, setDragSelectionSuppressed } from "../dragSelectionGuard";
import { documentOf } from "../windowRealm";

const DRAG_THRESHOLD_PX = 4;
let suppressClick = false;

/** True for the click that ends a reorder drag; row click handlers bail. */
export const rowReorderClickSuppressed = () => suppressClick;

export interface RowDropTarget {
  index: number;
  before: boolean;
  /** Pointer x relative to where the drag STARTED (not the row's edge), so
   *  where the row was grabbed never decides a nesting depth. */
  dx: number;
  /** The row element the pointer is over, for a caller whose rows live in several lists. */
  row: HTMLElement;
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
  // The drag's own window: listeners and hit tests follow the pointer's document.
  const doc = documentOf(event);
  let dragging = false;
  let target: RowDropTarget | null = null;
  const onMove = (ev: PointerEvent) => {
    if (!dragging) {
      if (Math.hypot(ev.clientX - startX, ev.clientY - startY) < DRAG_THRESHOLD_PX) return;
      dragging = true;
      setDragSelectionSuppressed(true, doc);
    }
    dropSelection(doc); // WebKit can re-anchor a selection mid-drag
    const row = doc.elementFromPoint(ev.clientX, ev.clientY)?.closest<HTMLElement>(rowSelector);
    if (row?.dataset.rowIndex !== undefined) {
      const rect = row.getBoundingClientRect();
      target = { index: Number(row.dataset.rowIndex), before: ev.clientY < rect.top + rect.height / 2, dx: ev.clientX - startX, row };
    } else target = null;
    onTarget(target);
  };
  const cleanup = () => {
    doc.removeEventListener("pointermove", onMove);
    doc.removeEventListener("pointerup", onUp);
    doc.removeEventListener("pointercancel", cleanup);
    setDragSelectionSuppressed(false);
    onTarget(null);
  };
  const onUp = () => {
    cleanup();
    if (!dragging) return;
    suppressClick = true;
    setTimeout(() => { suppressClick = false; }, 0);
    if (target) commit(target);
  };
  doc.addEventListener("pointermove", onMove);
  doc.addEventListener("pointerup", onUp);
  doc.addEventListener("pointercancel", cleanup);
}
