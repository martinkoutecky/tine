import { newResizeObserver } from "../windowRealm";

/** Hold existing viewport content through late journal-day/reference layout.
 * Like editor/scrollAnchor, a gesture or another focus/scroll owner releases
 * it. ResizeObserver restores displacement before paint; release also runs
 * when its pane/graph retires. One observer and four listeners per live anchor. */
export function captureJournalViewport(root: HTMLElement) {
  const scroller = root.closest<HTMLElement>(".main-content");
  if (!scroller) return;
  const focused = root.ownerDocument.activeElement;
  const top = scroller.getBoundingClientRect().top;
  const anchor = focused && root.contains(focused) ? focused
    : [...root.querySelectorAll<HTMLElement>("[data-block-id], h1.journal-title")].find(el => el.getBoundingClientRect().bottom > top);
  if (!anchor) return;
  const before = anchor.getBoundingClientRect().top;
  let scroll = scroller.scrollTop;
  let released = false;
  const gestures = ["wheel", "touchstart", "pointerdown", "keydown"] as const;
  const release = () => {
    released = true;
    observer?.disconnect();
    for (const event of gestures) scroller.removeEventListener(event, release, true);
  };
  const restore = () => {
    if (released) return;
    if (!anchor.isConnected || scroller.scrollTop !== scroll || (anchor === focused && anchor.ownerDocument.activeElement !== focused)) { release(); return; }
    const displacement = anchor.getBoundingClientRect().top - before;
    if (Math.abs(displacement) > 0.5) scroller.scrollTop += displacement;
    scroll = scroller.scrollTop;
  };
  const observer = newResizeObserver(root, restore);
  observer?.observe(root);
  for (const event of gestures) scroller.addEventListener(event, release, { passive: true, capture: true });
  return { restore, release };
}
