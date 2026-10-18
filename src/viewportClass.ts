// The one place that turns the window's size into a layout class
// (D-20261018-W1): UI depends on available room, never on a device name.
// Mobile drawers (< 640 px wide) own their own classifier in mobileDrawers.ts;
// the questions below build on it or on the same viewport reads.
import { createSignal } from "solid-js";
import { touchPrimary } from "./inputCapability";
import { mobileDrawerMode } from "./mobileDrawers";
import { mainWindow } from "./windowRealm";

/** Tablet-or-larger: the shorter viewport edge has room for two documents
 *  side by side. iPad mini portrait is 744pt across; every iPhone in either
 *  orientation is at most 430pt. Reads the current viewport; O(1). */
export function isTabletViewport(
  width = mainWindow?.innerWidth ?? 0,
  height = mainWindow?.innerHeight ?? 0,
): boolean {
  return Math.min(width, height) >= 700;
}

// A reactive tick for "the viewport changed size", so computations that call
// isSinglePaneShell() follow a rotation or a window resize.
const [viewportTick, bumpViewportTick] = createSignal(0);
if (typeof mainWindow?.addEventListener === "function") {
  mainWindow.addEventListener("resize", () => bumpViewportTick((n) => n + 1));
}

/** Is the layout single-pane (no split panes, PDFs open in place, no restored
 *  multi-pane layout)? A touch-first screen too small to carry two documents.
 *  A mouse or trackpad user picks their own window size and keeps splits at
 *  any size (so a saved layout is never flattened by a narrow desktop
 *  window), and a tablet has the room (GH #446). Reactive; O(1). */
export function isSinglePaneShell(): boolean {
  viewportTick();
  return touchPrimary() && !isTabletViewport();
}

/** Does the top bar have room for the tab strip? The window is wide enough
 *  that the left sidebar is not a drawer (>= 640 px). Below that the sidebar's
 *  Tabs section is the way to switch. Reactive. */
export function tabStripFits(): boolean {
  return !mobileDrawerMode();
}
