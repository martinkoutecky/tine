/** Shared outline/reference window observation. One observer per scroller and
 * margin, one geometry transaction per delivery. No per-group layout polling. */
import { batchViewportChanges, cachedScrollContainer } from "./scrollContainer";
import { newIntersectionObserver, newResizeObserver, windowOf } from "./windowRealm";

type Receive = (entry: IntersectionObserverEntry, margin: number) => void;
interface Watch { observer: IntersectionObserver; targets: Map<Element, Receive>; margin: number; resize?: ResizeObserver | null }
const realms = new WeakMap<Window, Map<Element | null, Map<number | "viewport", Watch>>>();

export function observeViewportWindow(host: HTMLElement, receive: Receive, reach: number | "viewport" = 600): () => void {
  const win = windowOf(host);
  const root = cachedScrollContainer(host);
  let roots = realms.get(win);
  if (!roots) realms.set(win, roots = new Map());
  let margins = roots.get(root);
  if (!margins) roots.set(root, margins = new Map());
  let watch = margins.get(reach);
  if (!watch) {
    let margin = reach === "viewport" ? Math.round((root?.clientHeight || win.innerHeight || 800) * 1.2) : reach;
    const targets = new Map<Element, Receive>();
    const receiveEntries = (entries: IntersectionObserverEntry[]) => batchViewportChanges(() => {
      for (const entry of entries) targets.get(entry.target)?.(entry, margin);
    });
    const observer = newIntersectionObserver(host, receiveEntries, { root, rootMargin: `${margin}px 0px` });
    if (!observer) return () => {};
    watch = { observer, targets, margin }; margins.set(reach, watch);
    if (reach === "viewport" && root) {
      const current = watch;
      current.resize = newResizeObserver(root, () => {
        const next = Math.round(root.clientHeight * 1.2);
        if (!next || next === margin) return;
        margin = next;
        current.observer.disconnect();
        current.observer = newIntersectionObserver(host, receiveEntries, { root, rootMargin: `${margin}px 0px` })!;
        for (const target of targets.keys()) current.observer.observe(target);
      });
      current.resize?.observe(root);
    }
  }
  watch.targets.set(host, receive); watch.observer.observe(host);
  return () => {
    watch.targets.delete(host); watch.observer.unobserve(host);
    if (!watch.targets.size) {
      watch.observer.disconnect(); watch.resize?.disconnect(); margins.delete(reach);
      if (!margins.size) roots.delete(root);
    }
  };
}
