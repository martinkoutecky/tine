// The hover preview's trigger and close rules (GH #40, #735), shared by every
// preview entry point: page links, block references and margin-comment
// highlights (D-20261108-MC7). Only a real mouse arms the dwell timer; touch
// opens through `hold`. Leaving the anchor or the popup closes after a grace
// period unless the popup's editor holds it (D-20261023-VQ).
import { createEffect, createSignal, onCleanup } from "solid-js";
import { hoverPreviewDelayMs } from "../hoverPreviewSettings";

// OG 0.10.9 page-preview-trigger / block-ref tippy: `:delay [1000, 100]` (ms open, close).
// GH #735: a quicker open puts the popup over the reference you meant to click.
const PEEK_CLOSE_MS = 100;

export function createPeekBridge(disabled: () => boolean) {
  const [shown, setOpen] = createSignal(false);
  const open = () => shown() && !disabled();
  let touchOpen = false;
  let openT: ReturnType<typeof setTimeout> | undefined;
  let closeT: ReturnType<typeof setTimeout> | undefined;
  // The popup's editor holds the caret (D-20261023-VQ): no hover timer closes it.
  let held = false;
  const setHeld = (value: boolean) => { held = value; };
  const closeLater = () => { closeT = setTimeout(() => { if (!held) setOpen(false); }, PEEK_CLOSE_MS); };
  const clearOpen = () => {
    if (openT) {
      clearTimeout(openT);
      openT = undefined;
    }
  };
  const clearClose = () => {
    if (closeT) {
      clearTimeout(closeT);
      closeT = undefined;
    }
  };
  // `opening` runs when the dwell elapses, just before the popup opens: an entry
  // point with several anchors (a block's comment highlights) picks the one that
  // armed it.
  const anchorEnter = (event: PointerEvent, opening?: () => void) => {
    // Touch WebViews synthesize mouse hover around a hold. Only a real mouse
    // may arm a hover preview; hybrid devices keep their mouse behavior.
    if (event.pointerType !== "mouse") { dismiss(); return; }
    if (disabled()) return;
    clearClose();
    clearOpen();
    const delay = hoverPreviewDelayMs();
    if (delay !== null) openT = setTimeout(() => { opening?.(); setOpen(true); }, delay);
  };
  const anchorLeave = () => {
    if (touchOpen) return;
    clearOpen();
    if (disabled()) return;
    clearClose();
    closeLater();
  };
  const popupEnter = () => {
    if (disabled()) return;
    clearClose();
  };
  const popupLeave = () => {
    if (touchOpen) return;
    if (disabled()) return;
    clearClose();
    closeLater();
  };
  const dismiss = () => {
    touchOpen = false;
    clearOpen();
    clearClose();
    setOpen(false);
  };
  // Disabling retires the preview, its hold and its timers, so enabling again opens nothing by itself.
  createEffect(() => { if (disabled()) dismiss(); });
  onCleanup(() => {
    clearOpen();
    clearClose();
  });
  /** Open the preview for a deliberate touch hold; false when this entry point
   *  does not preview, so the caller gives the hold its context menu. */
  const hold = () => {
    if (disabled()) return false;
    clearOpen();
    clearClose();
    touchOpen = true;
    setOpen(true);
    return true;
  };
  return { open, anchorEnter, anchorLeave, popupEnter, popupLeave, dismiss, hold, setHeld };
}
