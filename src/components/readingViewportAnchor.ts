import { newResizeObserver } from "../windowRealm";

/** Hold existing viewport content through changes to a reading surface.
 * Like editor/scrollAnchor, a gesture or another focus/scroll owner releases
 * it. ResizeObserver restores displacement before paint; release also runs
 * when its pane/graph retires. One observer per live anchor. */
export const readingScrollerSelector = ".main-content, .right-sidebar";
const owners = new WeakMap<HTMLElement, () => void>();

export function captureReadingViewport(root: HTMLElement, scroller = root.closest<HTMLElement>(readingScrollerSelector)) {
  if (!scroller) return;
  owners.get(scroller)?.();
  const focused = root.ownerDocument.activeElement;
  const top = scroller.getBoundingClientRect().top;
  const anchor = focused && focused !== root && root.contains(focused) ? focused
    : [...root.querySelectorAll<HTMLElement>("[data-block-id], h1.journal-title")].find(el => el.getBoundingClientRect().bottom > top);
  if (!anchor) return;
  const before = anchor.getBoundingClientRect().top;
  let scroll = scroller.scrollTop;
  let previousMaximum = Math.max(0, scroller.scrollHeight - scroller.clientHeight);
  let released = false;
  const gestures = ["wheel", "touchstart", "pointerdown", "keydown"] as const;
  const release = () => {
    released = true;
    observer?.disconnect();
    for (const event of gestures) scroller.removeEventListener(event, release, true);
    root.ownerDocument.removeEventListener("focusin", release, true);
    if (owners.get(scroller) === release) owners.delete(scroller);
  };
  const restore = () => {
    if (released) return;
    // Shrinking wrapped content can make the previous scrollTop exceed the
    // new range. Browsers clamp it during layout, before we can compensate;
    // that clamp is part of this reflow, not another scroll owner. Gestures
    // still release immediately, and all other scroll changes retire us.
    const maximum = Math.max(0, scroller.scrollHeight - scroller.clientHeight);
    const clamped = maximum < previousMaximum && scroll > maximum && Math.abs(scroller.scrollTop - maximum) <= 0.5;
    if (!root.isConnected || !anchor.isConnected || (scroller.scrollTop !== scroll && !clamped) || anchor.ownerDocument.activeElement !== focused) { release(); return; }
    const displacement = anchor.getBoundingClientRect().top - before;
    if (Math.abs(displacement) > 0.5) scroller.scrollTop += displacement;
    scroll = scroller.scrollTop;
    previousMaximum = maximum;
  };
  const observer = newResizeObserver(root, restore);
  observer?.observe(root);
  // A scroller's border box stays fixed while its content grows. Observe its
  // content boxes as well when it is itself the reading root (wrap preference).
  if (root === scroller) for (const child of root.children) observer?.observe(child);
  for (const event of gestures) scroller.addEventListener(event, release, { passive: true, capture: true });
  root.ownerDocument.addEventListener("focusin", release, true);
  owners.set(scroller, release);
  return { restore, release };
}
