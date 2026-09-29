// Shared "near the viewport" lazy-mount primitive.
// =================================================
// One module-level IntersectionObserver for the whole app — both LiveRefGroup
// (query/backlink groups) and AstBody (block bodies) register a one-shot "I'm
// within ~1.2 screens of the viewport" callback. A broad query set or a large
// page would otherwise spin up O(n) observers; one shared observer with a WeakMap
// of callbacks keeps it to a single observer.
//
// `observeNear(el, cb)` fires `cb` once when `el` first intersects the viewport
// (expanded by rootMargin), then unobserves it. `unobserveNear(el)` cancels a
// still-pending registration (the element unmounted before it ever came near).

const nearCbs = new WeakMap<Element, () => void>();
let sharedNearIO: IntersectionObserver | null = null;

export function observeNear(el: Element, cb: () => void) {
  // jsdom / SSR / any non-browser path has no IntersectionObserver. There, lazy
  // gating is meaningless (no layout, no scrolling), so fire the callback
  // synchronously — every consumer renders immediately, exactly today's behavior.
  // This keeps the jsdom render-test suite green. Checked at call time (not cached)
  // so a late polyfill / a test stub is honored.
  if (typeof IntersectionObserver === "undefined") {
    cb();
    return;
  }
  if (!sharedNearIO) {
    sharedNearIO = new IntersectionObserver(
      (entries) => {
        for (const e of entries) {
          if (!e.isIntersecting) continue;
          const fn = nearCbs.get(e.target);
          if (fn) {
            nearCbs.delete(e.target);
            sharedNearIO!.unobserve(e.target);
            fn();
          }
        }
      },
      { rootMargin: "1200px 0px" }
    );
  }
  nearCbs.set(el, cb);
  sharedNearIO.observe(el);
}

export function unobserveNear(el: Element) {
  if (sharedNearIO && nearCbs.has(el)) {
    nearCbs.delete(el);
    sharedNearIO.unobserve(el);
  }
}

// "This block id has rendered its body at least once." A block's body is parsed
// and rendered the first time it comes near the viewport; thereafter it stays
// rendered (render-once-keep — see AstBody and docs/adr). So a remount — edit→blur,
// the same block shown in a second surface, a route revisit — renders eagerly with
// no placeholder frame and no scroll-height churn. Module-level so it is shared
// across surfaces and survives component unmount; keyed by the stable block id.
// Bounded in practice by the set of blocks ever brought near the viewport (the
// working set), so it does not grow without use.
export const renderedBlocks = new Set<string>();

export function resetNearObserverForTests() {
  sharedNearIO?.disconnect();
  sharedNearIO = null;
  renderedBlocks.clear();
}
