// **A query-sheet popover always fits the viewport (GH #619).** The popovers are `position:absolute` under their
// trigger. A long list (the Task value list in the original report, the field chooser) hung off the bottom of
// the screen, and since the sheet is `position:fixed` nothing could scroll it back: the wheel had nowhere to
// go. The popover now measures the room above and below its trigger and either flips upward, or is clamped to
// the larger side and scrolls inside itself (its `.qs-options` list is the part that shrinks).

const GAP = 4; // the .qs-menu margin between trigger and popover
const EDGE = 8; // breathing room kept against the viewport edge
const MIN_HEIGHT = 120; // never clamp a popover below this; it would be unusable

export interface PopoverPlacement {
  /** Open upward (`bottom:100%`) instead of downward. */
  flip: boolean;
  /** Clamp to this many px and scroll inside; `null` = natural height. */
  maxHeight: number | null;
}

/** Pure placement arithmetic: `needed` is the popover's natural height, `below`/`above` the free px each side. */
export function placePopover(needed: number, below: number, above: number): PopoverPlacement {
  if (needed <= below) return { flip: false, maxHeight: null };
  if (needed <= above) return { flip: true, maxHeight: null };
  const flip = above > below;
  const room = flip ? above : below;
  return { flip, maxHeight: Math.max(MIN_HEIGHT, Math.floor(room)) };
}

/** Where the wide-layout query sheet's top edge goes (GH #619). The sheet is `position:fixed` under its
 *  sentence, so a sentence near the bottom of the window left the sheet's lower half off screen with nothing to
 *  scroll it back. It stays under the sentence when it fits there, else sits above it when it fits there, else is
 *  pushed up to the viewport's bottom edge (covering the sentence rather than leaving controls unreachable). */
export function placeSheetTop(sentenceTop: number, sentenceBottom: number, sheetHeight: number, viewHeight: number): number {
  if (sheetHeight <= 0) return sentenceBottom;
  if (sentenceBottom + sheetHeight <= viewHeight - EDGE) return sentenceBottom;
  if (sentenceTop - sheetHeight >= EDGE) return sentenceTop - sheetHeight;
  return Math.max(EDGE, viewHeight - sheetHeight - EDGE);
}

/** The nearest ancestor that clips and scrolls its content (the bottom sheet on a phone), if any. A popover
 *  inside one cannot flip upward: the part above the scroller's top edge is clipped and unreachable. It opens
 *  downward and the scroller's own scroll reveals it. */
function scrollingAncestor(menu: HTMLElement): HTMLElement | null {
  for (let el = menu.parentElement; el; el = el.parentElement) {
    const overflow = getComputedStyle(el).overflowY;
    if (overflow === "auto" || overflow === "scroll") return el;
  }
  return null;
}

/** Fit `menu` (a `.qs-menu`) to the viewport relative to its positioned parent, now and on every resize of
 *  the popover, scroll or window resize. Returns the cleanup. */
export function fitPopoverToViewport(menu: HTMLElement): () => void {
  let frame = 0;
  let alive = true;
  let revealed = false;
  const apply = () => {
    if (!alive) return;
    const anchor = menu.offsetParent ?? menu.parentElement;
    if (!menu.isConnected || !anchor) {
      frame = requestAnimationFrame(apply);
      return;
    }
    const view = window.visualViewport;
    const height = view?.height ?? window.innerHeight;
    const width = view?.width ?? window.innerWidth;
    const box = anchor.getBoundingClientRect();
    // Measure the NATURAL height: with last frame's clamp still applied the list had already shrunk, which read
    // as "fits" and flipped the popover into an oscillation.
    const held = menu.style.maxHeight;
    if (held) menu.style.maxHeight = "";
    const chrome = menu.offsetHeight - menu.clientHeight;
    const needed = menu.scrollHeight + chrome;
    const below = height - box.bottom - GAP - EDGE;
    const above = box.top - GAP - EDGE;
    const scroller = scrollingAncestor(menu);
    const placement = scroller ? { flip: false, maxHeight: null } : placePopover(needed, below, above);
    const next = placement.maxHeight === null ? "" : `${placement.maxHeight}px`;
    menu.style.maxHeight = next;
    const top = placement.flip ? "auto" : "";
    if (menu.style.top !== top) {
      menu.style.top = top;
      menu.style.bottom = placement.flip ? "100%" : "";
      menu.style.marginTop = placement.flip ? "0" : "";
      menu.style.marginBottom = placement.flip ? `${GAP}px` : "";
    }
    if (scroller && !revealed) {
      revealed = true;
      menu.scrollIntoView?.({ block: "nearest", inline: "nearest" });
    }
    // Horizontal: a popover that would cross the right edge hangs off its trigger's right side instead.
    const wide = Math.min(menu.offsetWidth, width);
    const crosses = box.left + wide > width - EDGE && box.right - wide >= EDGE;
    const left = crosses ? "auto" : "";
    if (menu.style.left !== left) {
      menu.style.left = left;
      menu.style.right = crosses ? "0" : "";
    }
  };
  const schedule = () => {
    if (!alive) return;
    cancelAnimationFrame(frame);
    frame = requestAnimationFrame(apply);
  };
  schedule();
  window.addEventListener("resize", schedule);
  window.addEventListener("scroll", schedule, true);
  const observer = typeof ResizeObserver === "function" ? new ResizeObserver(schedule) : undefined;
  observer?.observe(menu);
  for (const child of Array.from(menu.children)) observer?.observe(child);
  return () => {
    alive = false;
    cancelAnimationFrame(frame);
    window.removeEventListener("resize", schedule);
    window.removeEventListener("scroll", schedule, true);
    observer?.disconnect();
  };
}
