import { newResizeObserver, onEachWindow } from "../windowRealm";
import { routeScrollSettled } from "../routeScrollAnchor";

/** Hold existing viewport content through changes to a reading surface.
 * Like editor/scrollAnchor, a gesture or another focus/scroll owner releases
 * it. ResizeObserver restores displacement before paint; release also runs
 * when its pane/graph retires. One observer per live anchor. */
export const readingScrollerSelector = ".main-content, .right-sidebar";
const owners = new WeakMap<HTMLElement, () => void>();
const touchScrolling = new WeakSet<HTMLElement>();

// A fling may already be running when capture is requested. Observe its
// lifetime before capture, in every workspace window. scrollend is definitive;
// older WebKit falls back to a quiet period after the last momentum scroll.
onEachWindow(win => {
  const doc = win.document;
  if (!doc?.addEventListener) return;
  let scroller: HTMLElement | null = null;
  let held = false;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const clear = () => {
    clearTimeout(timer);
    if (scroller) touchScrolling.delete(scroller);
    scroller = null;
    held = false;
  };
  const settle = () => {
    clearTimeout(timer);
    if (!held) timer = setTimeout(clear, 200);
  };
  const start = (event: Event) => {
    clear();
    scroller = (event.target as Element)?.closest?.<HTMLElement>(readingScrollerSelector) ?? null;
    if (scroller) { held = true; touchScrolling.add(scroller); }
  };
  const end = () => { held = false; settle(); };
  const scroll = (event: Event) => { if (event.target === scroller) settle(); };
  const scrollend = (event: Event) => { if (event.target === scroller && !held) clear(); };
  const listeners = { touchstart: start, touchend: end, touchcancel: end, scroll, scrollend };
  for (const [type, listener] of Object.entries(listeners)) doc.addEventListener(type, listener, { passive: true, capture: true });
  return () => {
    clear();
    for (const [type, listener] of Object.entries(listeners)) doc.removeEventListener(type, listener, true);
  };
});

export function captureReadingViewport(root: HTMLElement, scroller = root.closest<HTMLElement>(readingScrollerSelector)) {
  if (!scroller) return;
  owners.get(scroller)?.();
  if (touchScrolling.has(scroller)) return;
  const focused = root.ownerDocument.activeElement;
  const viewport = scroller.getBoundingClientRect();
  // Prefer a block starting in view: the tail of a clipped code card can
  // intersect the viewport while its stationary top is offscreen. Holding
  // that top lets its own reflow push the visible reading row away. Outline
  // parents include children; descend the intersecting branch as a fallback
  // when no block starts in view (for example, a tall card fills the pane).
  let reading: HTMLElement | undefined;
  for (const el of root.querySelectorAll<HTMLElement>("[data-block-id], h1.journal-title")) {
    const rect = el.getBoundingClientRect();
    if (rect.bottom <= viewport.top) continue;
    // Document order: once a block starts below the viewport, none later starts in it.
    if (reading && rect.top >= viewport.bottom) break;
    if (!reading || reading.contains(el)) reading = el;
    if (rect.top >= viewport.top && rect.top < viewport.bottom) { reading = el; break; }
  }
  const anchor = focused && focused !== root && root.contains(focused) ? focused : reading;
  if (!anchor) return;
  let before = anchor.getBoundingClientRect().top;
  let scroll = scroller.scrollTop;
  let previousMaximum = Math.max(0, scroller.scrollHeight - scroller.clientHeight);
  let released = false;
  const gestures = ["wheel", "touchstart", "touchmove", "pointerdown", "keydown"] as const;
  const continuingDrag = (event: PointerEvent) => { if (event.buttons) release(); };
  const release = () => {
    released = true;
    observer?.disconnect();
    for (const event of gestures) scroller.removeEventListener(event, release, true);
    root.ownerDocument.removeEventListener("pointermove", continuingDrag, true);
    root.ownerDocument.removeEventListener("focusin", release, true);
    if (owners.get(scroller) === release) owners.delete(scroller);
  };
  const restore = () => {
    if (released) return;
    const maximum = Math.max(0, scroller.scrollHeight - scroller.clientHeight);
    // A held feed editor answers for this scroller: keep this anchor's record
    // current, so after the hold ends it holds what is shown then.
    if (feedHoldHandles(scroller)) {
      if (!root.isConnected || !anchor.isConnected || anchor.ownerDocument.activeElement !== focused) { release(); return; }
      before = anchor.getBoundingClientRect().top;
      scroll = scroller.scrollTop;
      previousMaximum = maximum;
      return;
    }
    // Shrinking wrapped content can make the previous scrollTop exceed the
    // new range. Browsers clamp it during layout, before we can compensate;
    // that clamp is part of this reflow, not another scroll owner. Gestures
    // still release immediately, and all other scroll changes retire us.
    const clamped = isLayoutClamp(scroll, previousMaximum, maximum, scroller.scrollTop);
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
  root.ownerDocument.addEventListener("pointermove", continuingDrag, { passive: true, capture: true });
  root.ownerDocument.addEventListener("focusin", release, true);
  owners.set(scroller, release);
  return { restore, release };
}

/** Shrinking content made the previous offset `last` exceed the new range and
 * the browser clamped the offset `now` during layout: part of that reflow, not
 * another writer. O(1). */
export function isLayoutClamp(last: number, previousMaximum: number, maximum: number, now: number): boolean {
  return maximum < previousMaximum && last > maximum && Math.abs(now - maximum) <= 0.5;
}

/** GH #987. An editor the user opened with a mouse or pen click in the Journals
 * feed keeps its top still while content around it changes size, from its caret
 * watch's settle until the first other interaction (any pointer, wheel, touch,
 * drop, or a key other than text typed into it), focus leaving it (seen at
 * its next check; no blur listener, so a window losing focus keeps it), its
 * teardown, Find
 * opening, a pane resize, or a write by another owner that moves it. While it
 * lives, every reflow restore on its scroller hands over to `check`, whatever
 * it captured; after it ends, everything is as before and it never re-arms on
 * its own. At most one, as only one editor is focused: the feed lives in the
 * main window. */
type HoldResult = "held" | "wrote" | "ended";
interface Hold { editor: HTMLElement; scroller: HTMLElement; check: () => HoldResult; end: () => void }
/** The editor's top within this many CSS px counts as kept: WebKit truncates
 * an assigned `scrollTop` to whole pixels. */
const HOLD_PX = 1;
let opening: string | HTMLElement | undefined;
let hold: Hold | undefined;
let vetoed = () => false;

/** In-page Find reports whether it is open; a hold never arms beside it. */
export function installFeedHoldVeto(veto: () => boolean): void { vetoed = veto; }

/** End the live hold, or only the hold of `editor`. O(1). */
export function endFeedHold(editor?: HTMLElement): void {
  if (hold && (!editor || hold.editor === editor)) hold.end();
}

/** First statement of every reflow restore: true when the live hold of this
 * scroller answered (its check ran and did not end the hold), so the caller
 * writes nothing; false runs the caller's own restore. One layout read. */
export function feedHoldHandles(scroller: Element): boolean {
  return hold?.scroller === scroller && hold.check() !== "ended";
}

const textKey = (event: KeyboardEvent) => (event.key.length === 1 || event.key === "Backspace" || event.key === "Delete")
  && !event.ctrlKey && !event.metaKey && !event.altKey && !event.isComposing;

onEachWindow(win => {
  const doc = win.document;
  if (!doc?.addEventListener) return;
  const interrupt = () => { opening = undefined; endFeedHold(); };
  const pointerdown = (event: Event) => {
    interrupt();
    const { isTrusted, button, pointerType, target } = event as PointerEvent;
    if (!isTrusted || button !== 0 || (pointerType !== "mouse" && pointerType !== "pen")) return;
    const block = (target as Element | null)?.closest?.(".journal-feed-day [data-block-id]");
    opening = block?.getAttribute("data-block-id") ?? undefined;
  };
  const keydown = (event: Event) => {
    opening = undefined;
    if (hold && !(event.target === hold.editor && textKey(event as KeyboardEvent))) endFeedHold();
  };
  // Images above that finish late: answer before the caret watch's next frame looks.
  const resource = (event: Event) => { if (hold?.scroller.contains(event.target as Node)) hold.check(); };
  const listeners = { pointerdown, keydown, wheel: interrupt, touchstart: interrupt, drop: interrupt, load: resource, error: resource };
  for (const [type, listener] of Object.entries(listeners)) doc.addEventListener(type, listener, { passive: true, capture: true });
  return () => { for (const [type, listener] of Object.entries(listeners)) doc.removeEventListener(type, listener, true); };
});

/** The caret watch of `editor` started: it is the click's editor only if it
 * opened the clicked block. */
export function noteEditorOpening(editor: HTMLElement): void {
  if (opening === editor) return;
  opening = opening !== undefined && opening === editor.closest("[data-block-id]")?.getAttribute("data-block-id") ? editor : undefined;
}

/** At the caret watch's settle: hold `editor` if a click opened it in the
 * Journals feed of `scroller`, the route owns that scroller's position and Find
 * is closed. `fits(shift)` answers whether the caret would still be shown after
 * scrolling down by `shift` CSS px. 1 layout read and 2 observed boxes (the day
 * list and the editor's day body, which a reserved day's wrapper hides). */
export function armFeedHold(editor: HTMLElement, scroller: HTMLElement, fits: (shift: number) => boolean): void {
  if (opening !== editor) return;
  opening = undefined;
  const doc = editor.ownerDocument;
  const day = editor.closest(".journal-feed-day");
  const root = day?.parentElement;
  const body = day?.firstElementChild;
  if (!root || !body || doc.activeElement !== editor || !scroller.matches(".main-content") || !scroller.contains(root)
    || vetoed() || !routeScrollSettled(scroller)) return;
  endFeedHold();
  const top = () => editor.getBoundingClientRect().top - scroller.getBoundingClientRect().top;
  const range = () => Math.max(0, scroller.scrollHeight - scroller.clientHeight);
  const baseline = top();
  const viewW = scroller.clientWidth, viewH = scroller.clientHeight;
  let last = scroller.scrollTop;
  let previousMaximum = range();
  const self: Hold = { editor, scroller, end: () => {
    observer?.disconnect();
    if (hold === self) hold = undefined;
  }, check: () => {
    if (hold !== self || !editor.isConnected || !root.isConnected || doc.activeElement !== editor
      || scroller.clientWidth !== viewW || scroller.clientHeight !== viewH) { self.end(); return "ended"; }
    const t = top(), s = scroller.scrollTop, max = range();
    const kept = Math.abs(t - baseline) <= HOLD_PX;
    // Another writer moved the editor (not a rounding, not the range clamping).
    if (!kept && Math.abs(s - last) > 0.5 && !isLayoutClamp(last, previousMaximum, max, s)) { self.end(); return "ended"; }
    if (!kept) {
      const want = s + t - baseline;
      const offset = Math.min(Math.max(want, 0), max);
      if (!fits(offset - s)) { self.end(); return "ended"; }
      scroller.scrollTop = offset;
      const applied = scroller.scrollTop;
      if (Math.abs(want - applied) > HOLD_PX) { self.end(); return "wrote"; }
      last = applied;
    } else last = s;
    previousMaximum = max;
    return "held";
  } };
  const observer = newResizeObserver(root, () => { self.check(); });
  observer?.observe(root);
  observer?.observe(body);
  hold = self;
}
