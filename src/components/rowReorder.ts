// Pointer-based row reorder (port of master GH #211 rowReorder.ts, used by the
// favorites tree). HTML5 DnD is unreliable in WebKitGTK, so: pointerdown arms,
// a 4px move starts the drag, the drop row comes from elementFromPoint via
// `data-row-index`, and the click that ends a drag is swallowed. While a drag
// runs the document is unselectable (WebKit otherwise smears a text selection
// across every row the pointer crosses) — see dragSelectionGuard.ts, which owns
// the mechanism this and the outline bullet drag (GH #424) both use.
import { dropSelection, setDragSelectionSuppressed } from "../dragSelectionGuard";
import { documentOf } from "../windowRealm";
import { LONG_PRESS_DELAY, LONG_PRESS_MOVE_TOLERANCE } from "../render/longPress";

const DRAG_THRESHOLD_PX = 4;
let suppressClick = false;
/** The one armed gesture: it belongs to the pointer that started it. */
let armed: { pointerId: number; cancel: () => void } | null = null;

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

export interface RowReorderOptions {
  /** Touch and pen start the drag only after a still hold (LONG_PRESS_DELAY),
   *  so a finger moving at once still scrolls the list; once held, the page
   *  does not scroll and the hold's own context menu is swallowed. Mouse drags
   *  are unchanged. Opt-in: the sidebar Tabs list (GH #704). */
  touchHold?: boolean;
  /** A pointer beside the rows (left or right of every row, e.g. past the
   *  list's edge while dragging left to un-nest) still targets the row at its
   *  height, instead of cancelling the drop. Opt-in: the sidebar Tabs list. */
  bandHit?: boolean;
}

/** Attach a reorder drag to a row's pointerdown. `onTarget` reports the live
 *  drop target (or null); `commit` receives the final one. */
export function beginRowReorderDrag(
  event: PointerEvent,
  rowSelector: string,
  onTarget: (target: RowDropTarget | null) => void,
  commit: (target: RowDropTarget) => void,
  opts: RowReorderOptions = {},
): void {
  if (event.button !== 0) return;
  if (armed) {
    // A second finger (or pen) never starts or steers a drag while one is
    // armed. A primary pointer going down again means the old gesture lost its
    // pointerup (e.g. released outside the window): drop it and start afresh.
    if (event.isPrimary === false && event.pointerId !== armed.pointerId) return;
    armed.cancel();
  }
  const pointerId = event.pointerId;
  const mine = (ev: PointerEvent) => ev.pointerId === pointerId;
  const startX = event.clientX;
  const startY = event.clientY;
  // The drag's own window: listeners and hit tests follow the pointer's document.
  const doc = documentOf(event);
  let dragging = false;
  let target: RowDropTarget | null = null;
  const hold = !!opts.touchHold && (event.pointerType === "touch" || event.pointerType === "pen");
  let holdTimer: ReturnType<typeof setTimeout> | undefined;
  const blockScroll = (ev: Event) => { if (ev.cancelable) ev.preventDefault(); };
  // The hold's own menu: neither the platform's nor the row's (a capture
  // listener on the document runs first and keeps it from the row).
  const swallowMenu = (ev: Event) => {
    ev.preventDefault();
    ev.stopImmediatePropagation();
  };
  const startDragging = () => {
    dragging = true;
    setDragSelectionSuppressed(true, doc);
    if (hold) doc.addEventListener("touchmove", blockScroll, { passive: false });
  };
  if (hold) {
    // The platform's own hold menu (Android fires contextmenu near the same
    // moment) must not open over the drag.
    doc.addEventListener("contextmenu", swallowMenu, true);
    holdTimer = setTimeout(() => {
      holdTimer = undefined;
      startDragging();
      dropSelection(doc);
    }, LONG_PRESS_DELAY);
  }
  const onMove = (ev: PointerEvent) => {
    if (!mine(ev)) return;
    if (!dragging) {
      const moved = Math.hypot(ev.clientX - startX, ev.clientY - startY);
      if (hold) {
        // Moving before the hold completes is a scroll: give the gesture back.
        if (moved > LONG_PRESS_MOVE_TOLERANCE) cleanup();
        return;
      }
      if (moved < DRAG_THRESHOLD_PX) return;
      startDragging();
    }
    dropSelection(doc); // WebKit can re-anchor a selection mid-drag
    let row = doc.elementFromPoint(ev.clientX, ev.clientY)?.closest<HTMLElement>(rowSelector) ?? null;
    if (!row && opts.bandHit) {
      row = [...doc.querySelectorAll<HTMLElement>(rowSelector)].find((r) => {
        const rect = r.getBoundingClientRect();
        return ev.clientY >= rect.top && ev.clientY < rect.bottom;
      }) ?? null;
    }
    if (row?.dataset.rowIndex !== undefined) {
      const rect = row.getBoundingClientRect();
      target = { index: Number(row.dataset.rowIndex), before: ev.clientY < rect.top + rect.height / 2, dx: ev.clientX - startX, row };
    } else target = null;
    onTarget(target);
  };
  function cleanup() {
    if (armed?.cancel === cleanup) armed = null;
    clearTimeout(holdTimer);
    holdTimer = undefined;
    doc.removeEventListener("touchmove", blockScroll);
    // The hold's own (native) contextmenu can arrive just after release.
    if (hold) setTimeout(() => doc.removeEventListener("contextmenu", swallowMenu, true), 0);
    doc.removeEventListener("pointermove", onMove);
    doc.removeEventListener("pointerup", onUp);
    doc.removeEventListener("pointercancel", onCancel);
    setDragSelectionSuppressed(false);
    onTarget(null);
  }
  const onCancel = (ev: PointerEvent) => { if (mine(ev)) cleanup(); };
  const onUp = (ev: PointerEvent) => {
    if (!mine(ev)) return;
    cleanup();
    if (!dragging) return;
    suppressClick = true;
    setTimeout(() => { suppressClick = false; }, 0);
    if (target) commit(target);
  };
  doc.addEventListener("pointermove", onMove);
  doc.addEventListener("pointerup", onUp);
  doc.addEventListener("pointercancel", onCancel);
  armed = { pointerId, cancel: cleanup };
}
