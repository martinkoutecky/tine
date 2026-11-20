import { createContext, onCleanup, onMount, useContext } from "solid-js";
import type { OutlineSelectionTarget } from "./document";

interface OutlineBoundary extends OutlineSelectionTarget {
  page?: string;
}
interface OutlineOwner {
  boundary?: OutlineBoundary;
  live?: OutlineSelectionTarget;
}
const zones = new WeakMap<Element, { owner: OutlineOwner; binding: boolean }>();
export const OutlineBlankOwner = createContext<OutlineOwner>();

/** Give each rendered outline its own identity, including while its roots are cold. O(1). */
export function createOutlineOwner(boundary?: OutlineBoundary): OutlineOwner {
  return { boundary };
}

function register(element: Element, owner: OutlineOwner, binding: boolean): void {
  const zone = { owner, binding };
  zones.set(element, zone);
  onCleanup(() => { if (zones.get(element) === zone) zones.delete(element); });
}

/** Register the outline's shell for its lifetime. Routed bindings also own
 * their pane's scroller and inner padding. Each registration and cleanup costs O(1). */
export function bindOutlineSelection(element: Element, owner: OutlineOwner, routed = false): void {
  register(element, owner, true);
  if (routed) onMount(() => {
    const inner = element.closest(".main-content-inner");
    if (!inner?.parentElement?.matches(".main-content")) return;
    register(inner.parentElement, owner, true);
    register(inner, owner, false);
  });
}

/** Register this element's blank layout for its render lifetime, in O(1).
 * Shared row renderers outside a blank outline have no owner to register. */
export function registerOutlineBlankZone(element: Element, owner = useContext(OutlineBlankOwner)): void {
  if (owner) register(element, owner, false);
}

/** Publish the live scope on its existing shell in O(1), for this binding's
 * lifetime. A cold or retired binding leaves the shell's ownership intact. */
export function bindLiveOutlineSelection(owner: OutlineOwner, boundary: OutlineSelectionTarget): void {
  owner.live = boundary;
  onCleanup(() => { if (owner.live === boundary) owner.live = undefined; });
}

export const selectAllDialogs = "dialog, [role='dialog'], [role='alertdialog']";
export const selectAllFields = 'input, textarea, select, [contenteditable=""], [contenteditable="true"], [contenteditable="plaintext-only"]';

/** Resolve blank outline space at click in O(DOM depth). Row padding
 * and child indentation belong to the enclosing outline. Embedded outlines
 * are reached before the parent's text wrapper. A cold outline owns its hit
 * without loading its source. A stored hit reads the shell's current scope
 * and expires when the owner or its page leaves. */
export function outlineSelectionAt(element: Element) {
  const owner = zones.get(element)?.owner;
  if (!owner) return;
  for (let at: Element | null = element; at; at = at.parentElement) {
    const zone = zones.get(at);
    if (zone?.owner !== owner) return;
    if (!zone.binding) continue;
    if (at.closest(".right-sidebar, .peek-popup:not(.comment-thread-popup)")) return;
    const registered = new WeakRef(zone);
    const page = owner.boundary?.page;
    const host = new WeakRef(at);
    return () => {
      const element = host.deref();
      const zone = registered.deref();
      if (element?.isConnected && zone && zones.get(element) === zone && zone.owner.boundary?.page === page) {
        return { host: element, outline: zone.owner.live ?? zone.owner.boundary };
      }
    };
  }
}
