import { clearOnBindingInvalidated } from "./binding";
import { newIntersectionObserver, onEachWindow, windowOf } from "./windowRealm";
import { batchViewportChanges, cachedScrollContainer } from "./scrollContainer";

// Shared "near the viewport" lazy-mount primitive.
// =================================================
// One IntersectionObserver per scroll container — both LiveRefGroup
// (query/backlink groups) and AstBody (block bodies) register a one-shot "I'm
// within ~1.2 screens of the viewport" callback. A broad query set or a large
// page would otherwise spin up O(n) observers. Resolving the root once at mount
// lets the margin extend inside the scroller instead of being clipped by it.
//
// `observeNear(el, cb)` fires `cb` once when `el` first intersects the viewport
// (expanded by rootMargin), then unobserves it.
//
// Block bodies reach only ABOVE the view (`"above"`): mounting there before it is
// seen is what keeps scrolling up steady (GH #810), while a body below the view
// grows only content further down, so mounting it early buys no stability and
// doubled a large page's cold parses (bigLoad, 12 to 24, Beta 0.9.0-beta.5).
// Groups and trackNear keep the symmetric margin (`"around"`). `unobserveNear(el)` cancels a
// still-pending registration (the element unmounted before it ever came near).

//
// Observers still belong to their root's window (OG-MULTIWINDOW P1). Empty
// observers are released so closed panes cannot accumulate in the window map.

const nearCbs = new WeakMap<Element, (entry?: IntersectionObserverEntry) => void>();
const nearTrackers = new WeakMap<Element, (near: boolean) => void>();
interface NearObserver { io: IntersectionObserver; count: number; release: () => void }
export type NearReach = "around" | "above";
const NEAR_MARGIN: Record<NearReach, string> = { around: "1200px 0px", above: "1200px 0px 0px 0px" };
type RootObservers = Map<Element | null, Map<NearReach, NearObserver>>;
const nearObservers = new Map<Window, RootObservers>();
const observing = new WeakMap<Element, NearObserver>();
let windowHookInstalled = false;

function observerFor(el: Element, reach: NearReach = "around"): NearObserver | null {
  const win = windowOf(el);
  // Checked at call time, as before workspace windows: a realm without the
  // constructor (jsdom, a test that removed its stub) renders eagerly.
  if (typeof (win as Window & { IntersectionObserver?: unknown }).IntersectionObserver !== "function") return null;
  const root = cachedScrollContainer(el);
  const observers: RootObservers = nearObservers.get(win) ?? new Map();
  const byReach = observers.get(root) ?? new Map<NearReach, NearObserver>();
  const existing = byReach.get(reach);
  if (existing) return existing;
  const io = newIntersectionObserver(win, (entries) => batchViewportChanges(() => {
    for (const e of entries) {
      nearTrackers.get(e.target)?.(e.isIntersecting);
      if (!e.isIntersecting) continue;
      const fn = nearCbs.get(e.target);
      if (fn) {
        unobserveNear(e.target);
        fn(e);
      }
    }
  }), { root, rootMargin: NEAR_MARGIN[reach] });
  if (!io) return null;
  const observer: NearObserver = { io, count: 0, release: () => {
    io.disconnect();
    byReach.delete(reach);
    if (!byReach.size) observers.delete(root);
    if (!observers.size) nearObservers.delete(win);
  } };
  byReach.set(reach, observer);
  observers.set(root, byReach);
  nearObservers.set(win, observers);
  if (!windowHookInstalled) {
    windowHookInstalled = true;
    onEachWindow((candidate) => () => {
      nearObservers.get(candidate)?.forEach(byReach => byReach.forEach(observer => observer.io.disconnect()));
      nearObservers.delete(candidate);
    });
  }
  return observer;
}

export function observeNear(el: Element, cb: (entry?: IntersectionObserverEntry) => void, reach: NearReach = "around") {
  // jsdom / SSR / any non-browser path has no IntersectionObserver. There, lazy
  // gating is meaningless (no layout, no scrolling), so fire the callback
  // synchronously — every consumer renders immediately, exactly today's behavior.
  // This keeps the jsdom render-test suite green. Checked at call time (not cached)
  // so a late polyfill / a test stub is honored.
  unobserveNear(el);
  const observer = observerFor(el, reach);
  if (!observer) {
    cb();
    return;
  }
  nearCbs.set(el, cb);
  observing.set(el, observer);
  observer.count++;
  observer.io.observe(el);
}

export function unobserveNear(el: Element) {
  if (!nearCbs.has(el)) return;
  nearCbs.delete(el);
  const observer = observing.get(el);
  observer?.io.unobserve(el);
  observing.delete(el);
  if (observer && --observer.count === 0) observer.release();
}

/** Report every crossing of the same near margin until the returned stop runs,
 * sharing this root's one observer. O(1) per host. `el` must already be attached.
 * Without IntersectionObserver nothing is reported: the caller's initial answer
 * stands, as a realm without layout has no viewport to leave. */
export function trackNear(el: Element, report: (near: boolean) => void): () => void {
  const observer = observerFor(el);
  if (!observer) return () => {};
  nearTrackers.set(el, report);
  observer.count++;
  observer.io.observe(el);
  return () => {
    nearTrackers.delete(el);
    if (!nearCbs.has(el)) observer.io.unobserve(el);
    if (--observer.count === 0) observer.release();
  };
}

// "This block id has rendered its body at least once." A block's body is parsed
// and rendered the first time it comes near the viewport; thereafter it stays
// rendered (render-once-keep — see AstBody and docs/adr). So a remount — edit→blur,
// the same block shown in a second surface, a route revisit — renders eagerly with
// no placeholder frame and no scroll-height churn. Module-level so it is shared
// across surfaces and survives component unmount; keyed by the stable block id.
// Graph-scoped: a store reset drops every visited id (I-21).
export const renderedBlocks = new Set<string>();
clearOnBindingInvalidated(() => renderedBlocks.clear());

export function resetNearObserverForTests() {
  for (const observers of nearObservers.values()) for (const byReach of observers.values()) for (const observer of byReach.values()) observer.io.disconnect();
  nearObservers.clear();
  renderedBlocks.clear();
}
