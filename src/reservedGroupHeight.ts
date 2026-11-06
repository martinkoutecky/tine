import { createComputed, createContext, createSignal, onCleanup, onMount, untrack, useContext } from "solid-js";
import { clearOnBindingInvalidated } from "./binding";
import { newResizeObserver, windowOf } from "./windowRealm";
import { batchViewportChanges, preserveAboveViewport } from "./scrollContainer";

const heights = new Map<string, { width: number; height: number }>();
/** Chunk geometry belongs to the lightweight occurrence, without its text. */
export const ReservedGroupGeometryContext = createContext<Map<string, { width: number; height: number }>>();
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
export function reservedGroupHeight(keyOf: () => string | undefined, element: () => HTMLElement | undefined, mounted: () => boolean, keepMeasured = false) {
  const cache = useContext(ReservedGroupGeometryContext) ?? heights;
  const [reserved, setReserved] = createSignal(0);
  let key: string | undefined;
  let measured: { width: number; height: number } | undefined;
  let stop = () => {};
  const remember = (height: number, width: number) => {
    if (height <= 0) return;
    measured = { width, height };
    setReserved(height);
  };
  createComputed(() => { key = keyOf(); });
  onMount(() => {
    const el = element();
    const remembered = key === undefined ? undefined : cache.get(key);
    // Retain the last box through the commit. Width validation belongs to the
    // next layout delivery, avoiding a forced clientWidth read per remount.
    if (remembered) setReserved(remembered.height);
    if (el) stop = trackSize(el, entry => {
      const width = entry.borderBoxSize[0]?.inlineSize ?? entry.contentRect?.width ?? 0;
      if (!keepMeasured && remembered && remembered.width !== width && !untrack(mounted)) {
        preserveAboveViewport(el, () => setReserved(0));
      }
      if (!untrack(mounted)) return;
      const height = entry.borderBoxSize[0]?.blockSize ?? entry.contentRect.height;
      remember(height, width);
    });
  });
  onCleanup(() => {
    stop();
    if (key === undefined || !measured) return;
    cache.delete(key);
    cache.set(key, measured);
    if (cache === heights && cache.size > 512) cache.delete(cache.keys().next().value!);
  });
  return Object.assign(reserved, { remember });
}
