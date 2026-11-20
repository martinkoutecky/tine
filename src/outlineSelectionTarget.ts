import { onCleanup, onMount } from "solid-js";
import type { OutlineSelectionTarget } from "./document";

interface OutlineBoundary {
  surface: string;
  page?: string;
  read: () => Omit<OutlineSelectionTarget, "surface"> | undefined;
}
const outlines = new WeakMap<Element, { boundary: OutlineBoundary | null }>();

/** Bind a rendered outline boundary. An empty or cold outline still owns the
 * gesture. O(1) registration, with a disposer for an existing shell. A routed
 * page also binds its ancestor scroller until this view unmounts. */
export function bindOutlineSelection(element: Element, boundary: OutlineBoundary | null = null, routed = false): () => void {
  if (routed) onMount(() => {
    const scroller = element.closest(".main-content");
    if (scroller) onCleanup(bindOutlineSelection(scroller, boundary));
  });
  const previous = outlines.get(element);
  const binding = { boundary };
  outlines.set(element, binding);
  return () => {
    if (outlines.get(element) !== binding) return;
    if (previous !== undefined) outlines.set(element, previous);
    else outlines.delete(element);
  };
}

export const selectAllDialogs = "dialog, [role='dialog'], [role='alertdialog']";
export const selectAllFields = `input, textarea, [contenteditable=""], [contenteditable="true"], [contenteditable="plaintext-only"], ${selectAllDialogs}`;
const interactive = `${selectAllFields}, [data-sheet-grid-id], a, button, select, label, [role='button'], .block-controls, .block-content-wrapper, .reference-page, .references-header`;

/** Resolve blank outline space at pointer-down in O(DOM depth). Row padding
 * and child indentation belong to the enclosing outline. Embedded outlines
 * are reached before the parent's text wrapper. A cold outline owns its hit
 * without loading its source. A stored hit expires with its binding or node. */
export function outlineSelectionAt(element: Element) {
  for (let at: Element | null = element; at; at = at.parentElement) {
    if (at.matches(interactive)) return;
    const binding = outlines.get(at);
    if (!binding) continue;
    const registered = new WeakRef(binding);
    const page = binding.boundary?.page;
    const host = new WeakRef(at);
    const hit = new WeakRef(element);
    return {
      get boundary() { return registered.deref()?.boundary; },
      page,
      current(focused?: Element) {
        const node = hit.deref();
        const owner = host.deref();
        const binding = registered.deref();
        return !!node?.isConnected && !!owner && !!binding && outlines.get(owner) === binding
          && owner.contains(node) && binding.boundary?.page === page && (!focused || focused.contains(node));
      },
    };
  }
}
