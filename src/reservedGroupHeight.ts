import { createComputed, createSignal, onCleanup, onMount, untrack } from "solid-js";
import { clearOnBindingInvalidated } from "./binding";
import { newResizeObserver, windowOf } from "./windowRealm";
import { batchViewportChanges, preserveAboveViewport } from "./scrollContainer";

const heights = new Map<string, { width: number; height: number }>();
clearOnBindingInvalidated(() => heights.clear());
const observers = new WeakMap<Window, { observer: ResizeObserver; targets: Map<Element, (entry: ResizeObserverEntry) => void> }>();

function trackSize(el: Element, receive: (entry: ResizeObserverEntry) => void) {
  const win = windowOf(el);
  let shared = observers.get(win);
  if (!shared) {
    const targets = new Map<Element, (entry: ResizeObserverEntry) => void>();
    const observer = newResizeObserver(el, entries => batchViewportChanges(() => {
      for (const entry of entries) targets.get(entry.target)?.(entry);
    }));
    if (!observer) return () => {};
    shared = { observer, targets };
    observers.set(win, shared);
  }
  shared.targets.set(el, receive);
  shared.observer.observe(el, { box: "border-box" });
  return () => {
    shared.targets.delete(el);
    shared.observer.unobserve(el);
    if (!shared.targets.size) {
      shared.observer.disconnect();
      observers.delete(win);
    }
  };
}

/** A bounded geometry cache; identity comes from the group owner, never rows.
 * Geometry comes from layout's ResizeObserver delivery, never from cleanup:
 * unmounting N groups adds zero synchronous layout reads. */
export function reservedGroupHeight(keyOf: () => string | undefined, element: () => HTMLElement | undefined, mounted: () => boolean) {
  const [reserved, setReserved] = createSignal(0);
  let key: string | undefined;
  let measured: { width: number; height: number } | undefined;
  let stop = () => {};
  createComputed(() => { key = keyOf(); });
  onMount(() => {
    const el = element();
    const remembered = key === undefined ? undefined : heights.get(key);
    // Retain the last box through the commit. Width validation belongs to the
    // next layout delivery, avoiding a forced clientWidth read per remount.
    if (remembered) setReserved(remembered.height);
    if (el) stop = trackSize(el, entry => {
      const width = el.clientWidth;
      if (remembered && remembered.width !== width && !untrack(mounted)) {
        preserveAboveViewport(el, () => setReserved(0));
      }
      if (!untrack(mounted)) return;
      const height = entry.borderBoxSize[0]?.blockSize ?? entry.contentRect.height;
      if (height > 0) measured = { width, height };
    });
  });
  onCleanup(() => {
    stop();
    if (key === undefined || !measured || !untrack(mounted)) return;
    heights.delete(key);
    heights.set(key, measured);
    if (heights.size > 512) heights.delete(heights.keys().next().value!);
  });
  return reserved;
}
