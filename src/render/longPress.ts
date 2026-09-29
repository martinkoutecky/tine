// GH #231: long-press → context menu for touch input. A still hold dispatches
// a SYNTHETIC contextmenu at the held point, so the gesture goes through the
// exact desktop menu path. Only touch/pen arm it: desktop mice already have
// right-click.
//
// What preventDefault() on that synthetic event does NOT do — this header used
// to claim otherwise, and GH #452 is what the claim cost — is cancel the
// platform's own long-press text selection. That gesture belongs to a native
// recognizer; iOS does not fire `contextmenu` for touch at all, so there is no
// default to prevent. Selection is refused in CSS instead
// (`html[data-platform="ios"] :is(.page-ref, .tag)` and the `.ctx-*` rules in
// app.css). Dropping any range the hold already produced, below, covers the
// window between the OS recognizer firing and our menu appearing.

import { dropSelection } from "../dragSelectionGuard";

export const LONG_PRESS_DELAY = 500; // ms — the conventional hold time
export const LONG_PRESS_MOVE_TOLERANCE = 10; // px — beyond it, the hold is a scroll/drag

const ownedContextMenuEvents = new WeakSet<Event>();

/** True only for the synthetic contextmenu produced by Tine's deliberate hold.
 * Native mobile contextmenu events remain owned by text selection. */
export function isLongPressContextMenu(event: Event): boolean {
  return ownedContextMenuEvents.has(event);
}

export interface LongPressHandlers {
  onPointerDown(e: PointerEvent): void;
  onPointerMove(e: PointerEvent): void;
  onPointerUp(e: PointerEvent): void;
  onPointerCancel(e: PointerEvent): void;
  /** Consume the compatibility click emitted when a completed hold releases. */
  consumeClick(event?: MouseEvent): boolean;
  dispose(): void;
}

/** Attach the returned handlers to the anchor/interactive element. When a
 *  primary touch/pen press stays within tolerance for the full delay, a
 *  contextmenu event fires AT THAT ELEMENT with the press coordinates, so a
 *  listening menu handler runs its ordinary desktop behavior (including
 *  preventDefault, which suppresses native selection/callout for this hold).
 *  A quick tap, any larger movement, pointer-cancel, or unmount cancels the
 *  gesture — ordinary scroll, tap, and text selection elsewhere are untouched
 *  because nothing is bound until the press starts, and everything clears the
 *  moment it ends. */
export function createLongPress(target: () => HTMLElement | undefined): LongPressHandlers {
  let armed: { id: number; x: number; y: number } | null = null;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let firedPointer: number | null = null;
  let ownedClickPointer: number | null = null;
  let ownedPointerType: string | null = null;
  let clickDocument: Document | null = null;
  let suppressClick = false;
  const cancel = () => {
    armed = null;
    if (timer !== null) {
      clearTimeout(timer);
      timer = null;
    }
  };
  const clearSuppression = () => {
    suppressClick = false;
    ownedClickPointer = null;
    ownedPointerType = null;
    clickDocument?.removeEventListener("contextmenu", consumeOwnedContextMenu, true);
    clickDocument?.removeEventListener("click", consumeOwnedClick, true);
    clickDocument?.removeEventListener("pointerdown", nextGesture, true);
    clickDocument = null;
  };
  const nextGesture = (event: PointerEvent) => {
    if (!event.isPrimary) return;
    firedPointer = null;
    clearSuppression();
  };
  const consumeOwnedContextMenu = (event: MouseEvent) => {
    if (ownedContextMenuEvents.has(event) || ownedClickPointer === null) return;
    // Android can deliver its native hold menu after our timer opened the
    // overlay. Keyboard/mouse context menus have a different pointer identity.
    if (!(event instanceof PointerEvent) || event.pointerId !== ownedClickPointer ||
        event.pointerType !== ownedPointerType) return;
    event.preventDefault();
    event.stopImmediatePropagation();
    // Keep ownership: the same hold may still emit its compatibility click.
  };
  const consumeOwnedClick = (event: MouseEvent) => {
    if (event.detail === 0 || ownedClickPointer === null) return;
    if (event instanceof PointerEvent && event.pointerId !== ownedClickPointer) return;
    // A newly opened menu can retarget this hold's compatibility click to its
    // overlay. Capture the gesture before that new target can activate/close.
    event.preventDefault();
    event.stopImmediatePropagation();
    firedPointer = null;
    clearSuppression();
  };
  return {
    onPointerDown(e: PointerEvent) {
      if (!e.isPrimary || (e.pointerType !== "touch" && e.pointerType !== "pen")) return;
      cancel();
      const armedNow = { id: e.pointerId, x: e.clientX, y: e.clientY };
      armed = armedNow;
      timer = setTimeout(() => {
        // Firing is terminal for THIS gesture; release later does nothing more.
        cancel();
        firedPointer = armedNow.id;
        const el = target();
        if (!el) return;
        clearSuppression();
        ownedClickPointer = armedNow.id;
        ownedPointerType = e.pointerType;
        clickDocument = el.ownerDocument;
        clickDocument.addEventListener("contextmenu", consumeOwnedContextMenu, true);
        clickDocument.addEventListener("click", consumeOwnedClick, true);
        clickDocument.addEventListener("pointerdown", nextGesture, true);
        const contextMenu = new MouseEvent("contextmenu", {
          bubbles: true,
          cancelable: true,
          clientX: armedNow.x,
          clientY: armedNow.y,
        });
        ownedContextMenuEvents.add(contextMenu);
        // The OS recognizer may have selected the held word first; the menu is
        // about to open over it.
        dropSelection();
        el.dispatchEvent(contextMenu);
      }, LONG_PRESS_DELAY);
    },
    onPointerMove(e: PointerEvent) {
      if (!armed || e.pointerId !== armed.id) return;
      const dx = e.clientX - armed.x;
      const dy = e.clientY - armed.y;
      if (Math.abs(dx) > LONG_PRESS_MOVE_TOLERANCE || Math.abs(dy) > LONG_PRESS_MOVE_TOLERANCE) cancel();
    },
    onPointerUp(e: PointerEvent) {
      if (firedPointer === e.pointerId) {
        firedPointer = null;
        suppressClick = true;
        // Ownership ends at its compatibility click or the next gesture,
        // including when this WebView emits no click. No timing window.
        e.preventDefault();
        e.stopPropagation();
      }
      cancel();
    },
    onPointerCancel(e: PointerEvent) {
      if (firedPointer === e.pointerId) firedPointer = null;
      cancel();
    },
    consumeClick(event?: MouseEvent) {
      if (event?.type === "click" && event.detail === 0) return false;
      // Some touch WebViews synthesize compatibility mouse events before
      // pointerup. Surfaces that activate on mousedown must be able to decline
      // them as soon as the hold has fired, not only after release.
      if (firedPointer !== null) return true;
      if (!suppressClick) return false;
      clearSuppression();
      return true;
    },
    dispose() {
      cancel();
      firedPointer = null;
      clearSuppression();
    },
  };
}
