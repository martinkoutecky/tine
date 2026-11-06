import { elementFromPointIn, requestFrame } from "./windowRealm";

/** The row first in view when a route was left, and how far below the scroller's
 * top it stood. Session memory only, beside the pixel offset (GH #848). */
export interface ScrollAnchor { id: string; offset: number }

const RESTORE_SCROLL_DEADLINE_MS = 10_000;
const RESTORE_SCROLL_QUIET_FRAMES = 30;

/** Bounded hit tests near the top edge, never a scan of the rows. Null when no block is there. */
export function captureScrollAnchor(scroller: HTMLElement): ScrollAnchor | null {
  if (typeof document === "undefined") return null; // no-DOM (unit tests)
  const box = scroller.getBoundingClientRect();
  for (const inset of [8, 48, 120]) {
    const row = elementFromPointIn(scroller, box.left + box.width / 2, box.top + inset)?.closest<HTMLElement>("[data-block-id]");
    const id = row?.getAttribute("data-block-id");
    if (row && id && scroller.contains(row)) return { id, offset: row.getBoundingClientRect().top - box.top };
  }
  return null;
}

/** The scrollTop that puts the anchored row back, or null while it is not rendered. One lookup. */
function scrollTopForAnchor(scroller: HTMLElement, anchor: ScrollAnchor): number | null {
  const row = scroller.querySelector(`[data-block-id="${anchor.id.replace(/["\\]/g, "\\$&")}"]`);
  if (!row) return null;
  return scroller.scrollTop + row.getBoundingClientRect().top - scroller.getBoundingClientRect().top - anchor.offset;
}

const GESTURES = ["wheel", "touchstart", "pointerdown", "keydown"] as const;

/** Go to a saved position once the content has rendered. A deep position keeps being pursued while the page grows:
 *  blocks and linked references render and measure asynchronously, and a slow reference read can answer well after
 *  the page shows (GH #848). The row that was first in view wins over the pixel offset once it exists, since rows
 *  above it may have changed height. Ends when position and height have held for half a second, at the deadline, on
 *  a user gesture or when `current` turns false. If the offset is deeper than the loaded content (a journal-feed
 *  position in days not loaded yet), `extend` pulls in more until it is reachable or the feed is exhausted. One
 *  selector lookup and one layout read per frame. */
export function pursueScroll(options: {
  target: number;
  anchor: ScrollAnchor | undefined;
  scroller: () => HTMLElement | null;
  current: () => boolean;
  extend: () => Promise<boolean>;
}): void {
  const { target, anchor, scroller, current, extend } = options;
  const deadline = performance.now() + RESTORE_SCROLL_DEADLINE_MS;
  let tries = 0;
  let extending = false;
  let quiet = 0;
  let lastHeight = -1;
  let released = false;
  let listening = false;
  const doc = scroller()?.ownerDocument;
  const release = () => { released = true; finish(); };
  const finish = () => { if (listening) for (const gesture of GESTURES) doc?.removeEventListener(gesture, release, true); };
  // Deferred to a frame so the page's content is laid out first; otherwise a synchronous reset races the content swap.
  const tick = () => {
    if (released) return;
    if (!current()) { finish(); return; }
    // From the first frame on, not before: the key press that asked for this navigation is still being dispatched.
    if (!listening && target > 0) {
      listening = true;
      for (const gesture of GESTURES) doc?.addEventListener(gesture, release, { capture: true, passive: true });
    }
    const el = scroller();
    if (!el) { finish(); return; }
    const max = Math.max(0, el.scrollHeight - el.clientHeight);
    const anchored = anchor ? scrollTopForAnchor(el, anchor) : null;
    const want = Math.min(anchored ?? target, max);
    const moved = Math.abs(el.scrollTop - want) >= 1;
    if (moved) el.scrollTop = want;
    if (target <= 0) return;
    quiet = !moved && el.scrollHeight === lastHeight ? quiet + 1 : 0;
    lastHeight = el.scrollHeight;
    const reached = anchored !== null || el.scrollTop >= target - 1;
    if ((reached && quiet >= RESTORE_SCROLL_QUIET_FRAMES) || performance.now() > deadline) { finish(); return; }
    // Out of reach. Content genuinely too short (deeper feed days not loaded): grow it. Otherwise it is async layout
    // catching up, so wait a frame.
    if (max < target - 1 && !extending && tries < 60) {
      extending = true;
      void extend().then((grew) => {
        extending = false;
        if (!grew) tries++;
        if (current() && !released) requestFrame(scroller(), tick);
        else finish();
      });
      return;
    }
    requestFrame(scroller(), tick);
  };
  requestFrame(scroller(), tick);
}
