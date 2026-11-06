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

/** Lazy replacement can correct an arbitrarily bad estimate (embeds, tables,
 * wrapped text). Only a box wholly above the view can displace existing readers
 * below it. Compare its viewport edge across the synchronous replacement, so
 * native anchoring already performed by Chromium is not applied twice.
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
  const bottom = el.getBoundingClientRect().bottom;
  const viewport = root.getBoundingClientRect();
  const above = bottom <= viewport.top;
  if (!deferred && bottom > viewport.top) { change(); return; }
  // A containing spacer can absorb a nested group's growth. Its own bottom
  // moving then says nothing about the reader. Hit-test a stable visible row
  // instead; a bounded number of probes avoids scanning mounted/page rows.
  let anchor: Element | null = null;
  for (const inset of [2, 32, 64]) {
    const hit = elementFromPointIn(root, viewport.left + viewport.width / 2, viewport.top + inset);
    const candidate = hit?.closest(".block-main, h1, .query-header") ?? hit;
    if (candidate && root.contains(candidate) && !candidate.contains(el) && !el.contains(candidate)) {
      anchor = candidate;
      break;
    }
  }
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
