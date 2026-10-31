import { elementFromPointIn, windowOf } from "./windowRealm";
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
