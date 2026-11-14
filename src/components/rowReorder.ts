// Pointer-based row reorder (port of master GH #211 rowReorder.ts, used by the
// favorites tree). HTML5 DnD is unreliable in WebKitGTK, so: pointerdown arms,
// a 4px move starts the drag, the drop row comes from elementFromPoint via
// `data-row-index`, and the click that ends a drag is swallowed. While a drag
// runs the document is unselectable (WebKit otherwise smears a text selection
// across every row the pointer crosses) — see dragSelectionGuard.ts, which owns
// the mechanism this and the outline bullet drag (GH #424) both use.
import { dropSelection, setDragSelectionSuppressed } from "../dragSelectionGuard";
import { documentOf } from "../windowRealm";
import { LONG_PRESS_DELAY, LONG_PRESS_MOVE_TOLERANCE, dispatchLongPressContextMenu, isLongPressContextMenu } from "../render/longPress";
import { closeContextMenu, contextMenu } from "../ui";

const DRAG_THRESHOLD_PX = 4;
let suppressClick = false;
/** The one armed gesture: it belongs to the pointer that started it. */
let armed: { pointerId: number; cancel: () => void } | null = null;

/** True for the click that ends a reorder drag; row click handlers bail. */
export const rowReorderClickSuppressed = () => suppressClick;

export interface RowDropTarget {
  index: number;
  before: boolean;
  /** Pointer inline movement relative to where the drag STARTED (not the row's edge), so
   *  where the row was grabbed never decides a nesting depth. */
  dx: number;
  /** The row element the pointer is over, for a caller whose rows live in several lists. */
  row: HTMLElement;
}

export interface RowReorderOptions {
  /** Touch and pen start the drag only after a still hold (LONG_PRESS_DELAY),
   *  so a finger moving at once still scrolls the list; once held, the page
   *  does not scroll. The hold opens the row's own context menu, as a
   *  right-click does (the platform's is swallowed); moving the held finger
   *  past LONG_PRESS_MOVE_TOLERANCE closes it and drags (GH #921). Mouse drags
   *  are unchanged. Opt-in: the sidebar Tabs and Favorites lists (GH #704). */
  touchHold?: boolean;
  /** A pointer beside the rows (left or right of every row, e.g. past the
   *  list's edge while dragging left to un-nest) still targets the row at its
   *  height, instead of cancelling the drop. Opt-in: the sidebar Tabs list. */
  bandHit?: boolean;
}

/** Attach a reorder drag to a row's pointerdown. `onTarget` reports the live
 *  drop target (or null); `commit` receives the final one. The returned cancel
 *  releases listeners and selection suppression if the owning view unmounts. */
export function beginRowReorderDrag(
  event: PointerEvent,
  rowSelector: string,
  onTarget: (target: RowDropTarget | null) => void,
  commit: (target: RowDropTarget) => void,
  opts: RowReorderOptions = {},
): (() => void) | undefined {
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
  const indentSign = doc.documentElement.dir === "rtl" ? -1 : 1;
  const pressed = event.target as Element | null;
  let dragging = false;
  let finished = false;
  /** The row menu this hold opened, while the drag has not yet taken over. */
  let heldMenu: ReturnType<typeof contextMenu> = null;
  let target: RowDropTarget | null = null;
  const hold = !!opts.touchHold && (event.pointerType === "touch" || event.pointerType === "pen");
  let holdTimer: ReturnType<typeof setTimeout> | undefined;
  const blockScroll = (ev: Event) => { if (ev.cancelable) ev.preventDefault(); };
  // The platform's own hold menu never opens (a capture listener on the
  // document runs first and keeps it from the row); the row's menu opens from
  // Tine's own hold event instead (GH #921).
  const swallowMenu = (ev: Event) => {
    if (isLongPressContextMenu(ev)) return;
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
      // GH #921: the still hold also opens the row's menu, as on iOS and
      // Android launchers: the menu shows on the hold, and moving the held
      // finger lifts the row into the drag and dismisses it.
      if (pressed?.isConnected && dispatchLongPressContextMenu(pressed, startX, startY).defaultPrevented) heldMenu = contextMenu();
    }, LONG_PRESS_DELAY);
  }
  const closeHeldMenu = () => {
    if (heldMenu && contextMenu() === heldMenu) closeContextMenu();
    heldMenu = null;
  };
  // A released hold that left its menu open owns the release's compatibility
  // click (it could land on the menu, under the finger) and any late platform
  // menu, until that click, the next gesture, or keyboard input.
  const releaseMenuOwnership = () => {
    doc.removeEventListener("contextmenu", swallowMenu, true);
    doc.removeEventListener("click", swallowReleaseClick, true);
    doc.removeEventListener("pointerdown", nextGesture, true);
    doc.removeEventListener("keydown", releaseMenuOwnership, true);
  };
  const swallowReleaseClick = (ev: MouseEvent) => {
    if (ev.detail === 0) return; // keyboard activation, not the hold's click
    ev.preventDefault();
    ev.stopImmediatePropagation();
    releaseMenuOwnership();
  };
  const nextGesture = (ev: PointerEvent) => { if (ev.isPrimary !== false) releaseMenuOwnership(); };
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
    if (heldMenu) {
      // The hold's menu stays open under a still finger; beyond the tolerance the drag takes over.
      if (Math.hypot(ev.clientX - startX, ev.clientY - startY) <= LONG_PRESS_MOVE_TOLERANCE) return;
      closeHeldMenu();
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
      target = { index: Number(row.dataset.rowIndex), before: ev.clientY < rect.top + rect.height / 2, dx: (ev.clientX - startX) * indentSign, row };
    } else target = null;
    onTarget(target);
  };
  function cleanup() {
    if (finished) return;
    finished = true;
    if (armed?.cancel === cleanup) armed = null;
    clearTimeout(holdTimer);
    holdTimer = undefined;
    doc.removeEventListener("touchmove", blockScroll);
    if (heldMenu && contextMenu() === heldMenu) {
      doc.addEventListener("click", swallowReleaseClick, true);
      doc.addEventListener("pointerdown", nextGesture, true);
      doc.addEventListener("keydown", releaseMenuOwnership, true);
    } else if (hold) {
      // The hold's own (native) contextmenu can arrive just after release.
      setTimeout(() => doc.removeEventListener("contextmenu", swallowMenu, true), 0);
    }
    heldMenu = null;
    doc.removeEventListener("pointermove", onMove);
    doc.removeEventListener("pointerup", onUp);
    doc.removeEventListener("pointercancel", onCancel);
    setDragSelectionSuppressed(false);
    onTarget(null);
  }
  const onCancel = (ev: PointerEvent) => { if (mine(ev)) cleanup(); };
  const onUp = (ev: PointerEvent) => {
    if (!mine(ev) || finished) return;
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
  return cleanup;
}
