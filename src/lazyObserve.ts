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
// (expanded by rootMargin), then unobserves it. `unobserveNear(el)` cancels a
// still-pending registration (the element unmounted before it ever came near).

//
// Observers still belong to their root's window (OG-MULTIWINDOW P1). Empty
// observers are released so closed panes cannot accumulate in the window map.

const nearCbs = new WeakMap<Element, (entry?: IntersectionObserverEntry) => void>();
interface NearObserver { io: IntersectionObserver; count: number; release: () => void }
const nearObservers = new Map<Window, Map<Element | null, NearObserver>>();
const observing = new WeakMap<Element, NearObserver>();
let windowHookInstalled = false;

function observerFor(el: Element): NearObserver | null {
  const win = windowOf(el);
  // Checked at call time, as before workspace windows: a realm without the
  // constructor (jsdom, a test that removed its stub) renders eagerly.
  if (typeof (win as Window & { IntersectionObserver?: unknown }).IntersectionObserver !== "function") return null;
  const root = cachedScrollContainer(el);
  const observers = nearObservers.get(win) ?? new Map<Element | null, NearObserver>();
  const existing = observers.get(root);
  if (existing) return existing;
  const io = newIntersectionObserver(win, (entries) => batchViewportChanges(() => {
    for (const e of entries) {
      if (!e.isIntersecting) continue;
      const fn = nearCbs.get(e.target);
      if (fn) {
        unobserveNear(e.target);
        fn(e);
      }
    }
  }), { root, rootMargin: "1200px 0px" });
  if (!io) return null;
  const observer: NearObserver = { io, count: 0, release: () => {
    io.disconnect();
    observers.delete(root);
    if (!observers.size) nearObservers.delete(win);
  } };
  observers.set(root, observer);
  nearObservers.set(win, observers);
  if (!windowHookInstalled) {
    windowHookInstalled = true;
    onEachWindow((candidate) => () => {
      nearObservers.get(candidate)?.forEach(observer => observer.io.disconnect());
      nearObservers.delete(candidate);
    });
  }
  return observer;
}

export function observeNear(el: Element, cb: (entry?: IntersectionObserverEntry) => void) {
  // jsdom / SSR / any non-browser path has no IntersectionObserver. There, lazy
  // gating is meaningless (no layout, no scrolling), so fire the callback
  // synchronously — every consumer renders immediately, exactly today's behavior.
  // This keeps the jsdom render-test suite green. Checked at call time (not cached)
  // so a late polyfill / a test stub is honored.
  unobserveNear(el);
  const observer = observerFor(el);
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
  for (const observers of nearObservers.values()) for (const observer of observers.values()) observer.io.disconnect();
  nearObservers.clear();
  renderedBlocks.clear();
}
