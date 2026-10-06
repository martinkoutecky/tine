import { clearOnBindingInvalidated } from "./binding";
import { newIntersectionObserver, onEachWindow, windowOf } from "./windowRealm";

// Shared "near the viewport" lazy-mount primitive.
// =================================================
// One IntersectionObserver per window for the whole app — both LiveRefGroup
// (query/backlink groups) and AstBody (block bodies) register a one-shot "I'm
// within ~1.2 screens of the viewport" callback. A broad query set or a large
// page would otherwise spin up O(n) observers; one shared observer with a WeakMap
// of callbacks keeps it to a single observer.
//
// `observeNear(el, cb)` fires `cb` once when `el` first intersects the viewport
// (expanded by rootMargin), then unobserves it. `unobserveNear(el)` cancels a
// still-pending registration (the element unmounted before it ever came near).

//
// Workspace windows (OG-MULTIWINDOW P1): an IntersectionObserver's implicit
// root is ITS realm's viewport, so a popup's blocks need the popup's observer.
// There is one shared observer per window, created lazily from that window's
// constructor and dropped when the window unregisters.

const nearCbs = new WeakMap<Element, () => void>();
const nearObservers = new Map<Window, IntersectionObserver>();
let windowHookInstalled = false;

function observerFor(el: Element): IntersectionObserver | null {
  const win = windowOf(el);
  // Checked at call time, as before workspace windows: a realm without the
  // constructor (jsdom, a test that removed its stub) renders eagerly.
  if (typeof (win as Window & { IntersectionObserver?: unknown }).IntersectionObserver !== "function") return null;
  const existing = nearObservers.get(win);
  if (existing) return existing;
  const io = newIntersectionObserver(win, (entries) => {
    for (const e of entries) {
      if (!e.isIntersecting) continue;
      const fn = nearCbs.get(e.target);
      if (fn) {
        nearCbs.delete(e.target);
        io!.unobserve(e.target);
        fn();
      }
    }
  }, { rootMargin: "1200px 0px" });
  if (!io) return null;
  nearObservers.set(win, io);
  if (!windowHookInstalled) {
    windowHookInstalled = true;
    onEachWindow((candidate) => () => {
      nearObservers.get(candidate)?.disconnect();
      nearObservers.delete(candidate);
    });
  }
  return io;
}

export function observeNear(el: Element, cb: () => void) {
  // jsdom / SSR / any non-browser path has no IntersectionObserver. There, lazy
  // gating is meaningless (no layout, no scrolling), so fire the callback
  // synchronously — every consumer renders immediately, exactly today's behavior.
  // This keeps the jsdom render-test suite green. Checked at call time (not cached)
  // so a late polyfill / a test stub is honored.
  const io = observerFor(el);
  if (!io) {
    cb();
    return;
  }
  nearCbs.set(el, cb);
  io.observe(el);
}

export function unobserveNear(el: Element) {
  if (!nearCbs.has(el)) return;
  nearCbs.delete(el);
  nearObservers.get(windowOf(el))?.unobserve(el);
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
  for (const io of nearObservers.values()) io.disconnect();
  nearObservers.clear();
  renderedBlocks.clear();
}
