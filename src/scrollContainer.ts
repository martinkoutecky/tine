import { elementFromPointIn, newResizeObserver, windowOf } from "./windowRealm";
import { batch } from "solid-js";

/** Resolve on mount, after insertion, in the element's own realm. Using the
 * nearest vertical scroller covers panes, sidebars, drawers and previews without
 * binding virtualization to any particular layout's class names. */
export function scrollContainer(el: Element): Element | null {
  const win = windowOf(el);
  for (let parent = el.parentElement; parent; parent = parent.parentElement) {
    if (/^(auto|scroll|overlay)$/.test(win.getComputedStyle(parent).overflowY)) return parent;
  }
  return null;
}

const adjusting = new WeakSet<Element>();
const roots = new WeakMap<Element, Element>();

/** Lazy hosts stay within their pane. Reuse ancestor resolutions, but reject a
 * cached root after moving a host to another pane. No repeated style walk per row. */
export function cachedScrollContainer(el: Element): Element | null {
  const path: Element[] = [];
  const win = windowOf(el);
  for (let parent = el.parentElement; parent; parent = parent.parentElement) {
    const cached = roots.get(parent);
    if (cached?.contains(el)) {
      for (const node of path) roots.set(node, cached);
      return cached;
    }
    path.push(parent);
    if (/^(auto|scroll|overlay)$/.test(win.getComputedStyle(parent).overflowY)) {
      for (const node of path) roots.set(node, parent);
      return parent;
    }
  }
  return null;
}

type Restore = () => void;
let transaction: Map<Element, Restore> | undefined;
const deferredRestores = new WeakMap<Element, Restore>();
const anchoredRoots = new WeakMap<Element, { retain: (section: HTMLElement) => () => void }>();

/** Keep a visible reference occurrence anchored through asynchronous filtering
 * and width reflow. One observer/anchor per scroller, shared by sections. Scroll
 * intent invalidates the old snapshot, so late layout cannot undo a user jump.
 * This supplements the before/after transactions used for window hydration. */
export function trackViewportAnchor(host: HTMLElement): () => void {
  const root = cachedScrollContainer(host);
  if (!root) return () => {};
  const existing = anchoredRoots.get(root);
  if (existing) return existing.retain(host);
  const realm = windowOf(root) as Window & typeof globalThis;
  let owners = 0;
  let anchor: { group: Element; element: Element; block?: string; key?: string; top: number; scroll: number } | undefined;
  const capture = () => {
    const view = root.getBoundingClientRect();
    const hit = elementFromPointIn(root, view.left + view.width * .35, view.top + 2);
    const group = hit?.closest(".reference-group");
    if (!group || !root.contains(group)) { anchor = undefined; return; }
    const element = hit?.closest(".page-property-reference-row, [data-block-id], .reference-excerpt-row") ?? group;
    anchor = { group, element, top: element.getBoundingClientRect().top - view.top, scroll: root.scrollTop,
      block: element.getAttribute("data-block-id") ?? undefined,
      key: element.matches(".page-property-reference-row") ? element.querySelector(".prop-key")?.textContent ?? undefined : undefined };
  };
  const restore = () => {
    if (anchor && anchor.scroll === root.scrollTop && anchor.group.isConnected) {
      let element: Element | null = anchor.element.isConnected ? anchor.element : null;
      if (!element && anchor.block) element = anchor.group.querySelector(`[data-block-id="${realm.CSS.escape(anchor.block)}"]`);
      if (!element && anchor.key) element = [...anchor.group.querySelectorAll(".page-property-reference-row")]
        .find(row => row.querySelector(".prop-key")?.textContent === anchor!.key) ?? null;
      if (element) {
        const displacement = element.getBoundingClientRect().top - root.getBoundingClientRect().top - anchor.top;
        if (Math.abs(displacement) > .5) root.scrollTop += displacement;
      }
    }
    capture();
  };
  // Only the reference sections' own subtrees: typing in the page outline above
  // must not force a layout read per keystroke. Shifts from content above a
  // section are handled by preserveAboveViewport and the root's ResizeObserver.
  const mutation = new realm.MutationObserver(restore);
  const resize = newResizeObserver(root, restore);
  resize?.observe(root);
  root.addEventListener("scroll", capture, { passive: true });
  capture();
  const shared = { retain: (section: HTMLElement) => {
    owners++;
    mutation.observe(section, { childList: true, subtree: true, characterData: true });
    return () => {
      if (--owners) return;
      mutation.disconnect(); resize?.disconnect(); root.removeEventListener("scroll", capture);
      anchoredRoots.delete(root);
    };
  } };
  anchoredRoots.set(root, shared);
  return shared.retain(host);
}

/** Solid defers DOM writes until every entry has supplied its before geometry.
 * Only one anchor per root is needed, then one restore after the DOM commit. */
export function batchViewportChanges(change: () => void): void {
  if (transaction) { change(); return; }
  const restores = new Map<Element, Restore>();
  transaction = restores;
  try { batch(change); }
  finally {
    transaction = undefined;
    for (const restore of restores.values()) restore();
  }
}

/** Rows that host a query occurrence: an authored block's row, or a Journals
 * default-query or agenda wrapper. */
const QUERY_HOST = ".block-main, .default-journal-query, .agenda-block";

/** Lazy replacement can correct an arbitrarily bad estimate (embeds, tables,
 * wrapped text). A box wholly above the view displaces the readers below it;
 * a deferred answer (a query rerun) may also straddle the top or sit inside the
 * view. Compare a visible reader row (or, for a box wholly above, its own
 * edge) across the replacement, so native anchoring already performed by
 * Chromium is not applied twice.
 * IO callers supply observer geometry and batch all entries, so below-view
 * mounts read no layout and above-view mounts share one root snapshot. */
export function preserveAboveViewport(el: Element, change: () => void, root?: Element | null, deferred = false,
  entry?: IntersectionObserverEntry, margin = 0): void {
  // rootBounds includes the symmetric vertical rootMargin, not just the view.
  // IO geometry costs no layout and is valid for the entire callback batch.
  if (entry?.rootBounds && entry.boundingClientRect.bottom > entry.rootBounds.top + margin) { change(); return; }
  root ??= cachedScrollContainer(el);
  if (!root || adjusting.has(root)) { change(); return; }
  const pending = deferred ? deferredRestores : transaction;
  if (pending?.has(root)) { change(); return; }
  const box = el.getBoundingClientRect();
  const bottom = box.bottom;
  const viewport = root.getBoundingClientRect();
  const above = bottom <= viewport.top;
  // Only replies above or inside the viewport can move the reader during
  // deferred publication.
  if (box.top >= viewport.bottom || (!deferred && !above)) { change(); return; }
  // A containing spacer can absorb a nested group's growth. Its own bottom
  // moving then says nothing about the reader. Hit-test a stable visible row
  // instead; a bounded number of probes avoids scanning mounted/page rows.
  // GH #944: a probe that lands on something which cannot hold the reader
  // continues just below it rather than at a fixed inset: below this box when
  // it straddles the top (its own rows change), below a pinned ancestor header
  // (it never moves with the content; three pins cover 90 px), and below
  // another query (it may rerun in the same turn, and this root shares one
  // snapshot). A row of another query is kept only as a fallback. The raw hit
  // is classified before promotion to its row: a table, board or page result
  // has no row of its own and would promote to the row hosting its query.
  // A query occurrence ends at the bottom of the authored row hosting it: the
  // query box's 2 px margin, and any chrome below it, sit on that row, whose
  // top does not move with the query's change. A probe landing on a host row
  // (of this box or of any query) continues below the row; another query's
  // host row is kept only as a fallback.
  // Only misses spend the budget; each skip passes a whole obstruction, and
  // the walk ends at the bottom of the view, whose last pixel row is probed.
  let anchor: Element | null = null, fallback: Element | null = null;
  const x = viewport.left + viewport.width / 2;
  for (let misses = 0, y = viewport.top + 2; misses < 8 && !(y >= viewport.bottom);) {
    let next = y + (y < viewport.top + 32 ? 30 : 32);
    if (next >= viewport.bottom && y < viewport.bottom - 1) next = viewport.bottom - 1;
    if (y >= box.top && y < box.bottom) { y = box.bottom + 1; continue; }
    const hit = elementFromPointIn(root, x, y);
    const blocker: Element | null = !hit || !root.contains(hit) ? null : el.contains(hit) ? el : hit.closest(".sticky-ancestor, .query-block");
    const candidate = hit?.closest(".block-main, h1, .query-header") ?? hit;
    if (blocker && blocker !== root && root.contains(blocker)) {
      if (blocker !== el && !blocker.matches(".sticky-ancestor")) fallback ??= candidate && blocker.contains(candidate) ? candidate : hit;
      const edge = blocker === el ? bottom : blocker.getBoundingClientRect().bottom;
      if (edge >= y) { y = edge + 1; continue; }
    } else if (candidate && candidate !== root && root.contains(candidate)) {
      const own = candidate.contains(el);
      if (candidate.matches(QUERY_HOST) && (own || candidate.querySelector(".query-block"))) {
        if (!own) fallback ??= candidate;
        const edge = candidate.getBoundingClientRect().bottom;
        if (edge >= y) { y = edge + 1; continue; }
      } else if (!own) { anchor = candidate; break; }
    }
    misses++;
    y = next;
  }
  anchor ??= fallback;
  const anchorTop = anchor?.getBoundingClientRect().top;
  adjusting.add(root);
  try {
    const restore = () => {
      if (!el.isConnected || !root.isConnected) return;
      if (anchor && !anchor.isConnected) return;
      if (!anchor && !above) return;
      const displacement = anchor ? anchor.getBoundingClientRect().top - anchorTop!
        : el.getBoundingClientRect().bottom - bottom;
      if (Math.abs(displacement) > 0.5) root.scrollTop += displacement;
    };
    if (pending) pending.set(root, restore);
    change();
    // Resource setters run inside Solid's batch. Its DOM update completes when
    // the batch exits; this microtask restores before the next painted frame.
    if (deferred) queueMicrotask(() => {
      deferredRestores.delete(root);
      restore();
    });
    else if (!transaction) restore();
  } finally { adjusting.delete(root); }
}
