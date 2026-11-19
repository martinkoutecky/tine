import { onCleanup, onMount } from "solid-js";
import type { OutlineSelectionTarget } from "./document";

interface OutlineBoundary {
  surface: string;
  page?: string;
  read: () => Omit<OutlineSelectionTarget, "surface"> | undefined;
}
const outlines = new WeakMap<Element, OutlineBoundary | null>();

/** Bind a rendered outline boundary. An empty or cold outline still owns the
 * gesture. O(1) registration, with a disposer for an existing shell. A routed
 * page also binds its ancestor scroller until this view unmounts. */
export function bindOutlineSelection(element: Element, boundary: OutlineBoundary | null = null, routed = false): () => void {
  if (routed) onMount(() => {
    const scroller = element.closest(".main-content");
    if (scroller) onCleanup(bindOutlineSelection(scroller, boundary));
  });
  const previous = outlines.get(element);
  outlines.set(element, boundary);
  return () => {
    if (outlines.get(element) !== boundary) return;
    if (previous !== undefined) outlines.set(element, previous);
    else outlines.delete(element);
  };
}

/** Resolve a boundary in O(DOM depth). Blank gestures stop at a row, unless
 * a more specific nested outline already claimed the click. Null is a cold
 * outline, which owns the gesture without loading its source. */
export function outlineSelectionAt(element: Element | undefined, blank = false): OutlineBoundary | null | undefined {
  for (let at = element; at; at = at.parentElement ?? undefined) {
    const boundary = outlines.get(at);
    if (boundary !== undefined) return boundary;
    if (blank && at.classList.contains("ls-block")) return undefined;
  }
  return undefined;
}
