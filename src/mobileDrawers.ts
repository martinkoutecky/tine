import { createSignal } from "solid-js";
import { topTransientLayer } from "./transientLayers";

export type DrawerSide = "left" | "right";
export type DrawerDismissReason = "explicit" | "scrim" | "escape" | "back" | "navigation";

export function isMobileDrawerViewport(width: number) {
  return width < 640;
}
export function mobileDrawerMatches(media = typeof matchMedia === "function" && matchMedia("(max-width: 639px)").matches) {
  return media;
}

const [mobileDrawerMode, setMobileDrawerMode] = createSignal(mobileDrawerMatches());
export { mobileDrawerMode };

/** Installs the sole reactive classifier.  It deliberately has no CSS twin. */
export function installMobileDrawerMode(): () => void {
  if (typeof matchMedia !== "function") return () => {};
  const query = matchMedia("(max-width: 639px)");
  const update = () => setMobileDrawerMode(mobileDrawerMatches(query.matches));
  update();
  query.addEventListener?.("change", update);
  return () => query.removeEventListener?.("change", update);
}

let opener: HTMLElement | null = null;
/** The open sidebar signals in ui.ts are the sole active-drawer source.  This
 * module deliberately owns only transient focus restoration, never visibility. */
export function captureDrawerOpener(trigger?: HTMLElement | null) {
  opener = trigger?.isConnected ? trigger : null;
}
export function takeDrawerOpener() { const value = opener; opener = null; return value; }
export function clearDrawerOpener() { opener = null; }
export function focusMainContent() {
  const main = document.querySelector<HTMLElement>(".pane-focused .main-content, .main-content");
  main?.focus?.();
}
export function restoreDrawerFocus(reason: DrawerDismissReason) {
  const candidate = takeDrawerOpener();
  if ((reason === "explicit" || reason === "escape") && candidate?.isConnected && !candidate.inert) {
    candidate.focus();
  } else {
    focusMainContent();
  }
}
export function drawerFocusables(root: HTMLElement): HTMLElement[] {
  return [...root.querySelectorAll<HTMLElement>('button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])')]
    .filter((el) => {
      for (let node: HTMLElement | null = el; node && node !== root.parentElement; node = node.parentElement) {
        if (node.hidden || node.inert || node.getAttribute("aria-hidden") === "true") return false;
        const style = typeof getComputedStyle === "function" ? getComputedStyle(node) : null;
        if (style?.display === "none" || style?.visibility === "hidden") return false;
      }
      return true;
    });
}

function hasInertAncestor(element: HTMLElement): boolean {
  for (let node: HTMLElement | null = element; node; node = node.parentElement) {
    if (node.inert || node.hasAttribute("inert")) return true;
  }
  return false;
}

/** Only a live layer above the inert background suspends the drawer.  Local
 * transients such as in-page Find can remain registered while their shell
 * region is inert; treating those as higher would strand focus in background. */
function higherTransientOwnsFocus(): boolean {
  const layer = topTransientLayer();
  if (!layer) return false;
  const root = layer.root?.();
  return !root || !hasInertAncestor(root);
}

export function trapDrawerTab(event: KeyboardEvent, root: HTMLElement) {
  // A registered transient is above the drawer. Its own focus model must win;
  // otherwise a Tab in Settings/a menu would be pulled back into the drawer.
  if (higherTransientOwnsFocus()) return;
  const items = drawerFocusables(root);
  if (!items.length) { event.preventDefault(); root.focus(); return; }
  const first = items[0], last = items[items.length - 1];
  const active = document.activeElement;
  if (event.shiftKey && (active === first || active === root || !(active instanceof Node) || !root.contains(active))) {
    event.preventDefault(); last.focus();
  } else if (!event.shiftKey && (active === last || active === root || !(active instanceof Node) || !root.contains(active))) {
    event.preventDefault(); first.focus();
  }
}

/** Focus can move by pointer, script, or assistive technology without a Tab
 * key. Keep it in the drawer unless a higher transient owns it. */
export function containDrawerFocus(root: HTMLElement) {
  if (higherTransientOwnsFocus()) return;
  const focused = document.activeElement;
  if (focused instanceof Node && root.contains(focused)) return;
  (drawerFocusables(root)[0] ?? root).focus();
}

/** Focus the active drawer after it is mounted, unless a higher transient owns
 * focus already.  The active side is passed in so this module stays independent
 * of ui.ts's visibility store. */
export function focusDrawer(side: DrawerSide) {
  if (higherTransientOwnsFocus()) return;
  const root = document.querySelector<HTMLElement>(side === "left" ? ".left-sidebar" : ".right-sidebar");
  if (root) containDrawerFocus(root);
}
