import type { OutlineSelectionTarget } from "./document";

type ReadOutline = () => OutlineSelectionTarget | undefined;
const outlines = new WeakMap<Element, ReadOutline>();
const empty: ReadOutline = () => undefined;

/** Bind a rendered outline boundary. An empty or cold outline still owns the
 * gesture. O(1) registration, with a disposer for a temporary binding on an
 * existing shell. The element owns its weak entry. */
export function bindOutlineSelection(element: Element, read = empty): () => void {
  const previous = outlines.get(element);
  outlines.set(element, read);
  return () => {
    if (outlines.get(element) !== read) return;
    if (previous) outlines.set(element, previous);
    else outlines.delete(element);
  };
}

/** Find the nearest outline in O(DOM depth), without reading any rows. */
export function outlineSelectionAt(element: Element | undefined): ReadOutline | undefined {
  for (let at = element; at; at = at.parentElement ?? undefined) {
    const read = outlines.get(at);
    if (read) return read;
  }
  return undefined;
}
