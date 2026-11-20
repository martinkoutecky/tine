import { onCleanup, onMount } from "solid-js";
import type { OutlineSelectionTarget } from "./document";

interface OutlineBoundary {
  surface: string;
  page?: string;
  read: () => Omit<OutlineSelectionTarget, "surface"> | undefined;
}
const outlines = new WeakMap<Element, { boundary: OutlineBoundary | null; live?: OutlineBoundary }>();

/** Own an outline hit for the lifetime of this shell. O(1) registration and
 * cleanup. A routed page also owns hits on its scroller until it unmounts. */
export function bindOutlineSelection(element: Element, boundary: OutlineBoundary | null = null, routed = false): void {
  if (routed) onMount(() => {
    const scroller = element.closest(".main-content");
    if (scroller) bindOutlineSelection(scroller, boundary);
  });
  const owner = { boundary };
  outlines.set(element, owner);
  onCleanup(() => { if (outlines.get(element) === owner) outlines.delete(element); });
}

/** Publish the live scope on its existing shell in O(1), for this binding's
 * lifetime. A cold or retired binding leaves the shell's ownership intact. */
export function bindLiveOutlineSelection(element: Element, boundary: OutlineBoundary): void {
  const owner = outlines.get(element);
  if (!owner) return;
  owner.live = boundary;
  onCleanup(() => { if (owner.live === boundary) owner.live = undefined; });
}

export const selectAllDialogs = "dialog, [role='dialog'], [role='alertdialog']";
export const selectAllFields = 'input, textarea, select, [contenteditable=""], [contenteditable="true"], [contenteditable="plaintext-only"]';
const interactive = `${selectAllFields}, ${selectAllDialogs}, [data-sheet-grid-id], a, button, label, [role='button'], .block-controls, .block-content-wrapper, .reference-page, .references-header`;

/** Resolve blank outline space at pointer-down in O(DOM depth). Row padding
 * and child indentation belong to the enclosing outline. Embedded outlines
 * are reached before the parent's text wrapper. A cold outline owns its hit
 * without loading its source. A stored hit reads the shell's current scope
 * and expires when the owner or its page leaves. */
export function outlineSelectionAt(element: Element) {
  for (let at: Element | null = element; at; at = at.parentElement) {
    if (at.matches(interactive)) return;
    const binding = outlines.get(at);
    if (!binding) continue;
    const registered = new WeakRef(binding);
    const page = binding.boundary?.page;
    const host = new WeakRef(at);
    return {
      get boundary() {
        const owner = registered.deref();
        return owner?.live ?? owner?.boundary;
      },
      page,
      ownedBy(container: Element) {
        const owner = host.deref();
        return !!owner && container.contains(owner);
      },
      current(focused?: Element) {
        const owner = host.deref();
        const binding = registered.deref();
        return !!owner?.isConnected && !!binding && outlines.get(owner) === binding
          && binding.boundary?.page === page && (!focused || focused.contains(owner) || owner.contains(focused));
      },
    };
  }
}
