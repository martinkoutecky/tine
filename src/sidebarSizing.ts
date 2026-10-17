import { msg } from "./i18n";
/** Device-local sidebar widths. Reads and changes are O(1), independent of the graph.
 * Keyboard and pointer resizing share resizeSidebar's bounds; commitSidebarWidth
 * persists the current width using the existing localStorage keys. Load/save
 * failures show a toast and keep the in-memory width usable. Callers need no
 * knowledge of storage keys or bounds; live drag updates commit on release.
 */
import { createSignal } from "solid-js";
import { pushToast } from "./toasts";
import { mainWindow, viewportOf } from "./windowRealm";

const SIDEBAR_W_KEY = "logseq-claude.sidebarWidth";
function loadSidebarWidth(): number {
  try {
    const v = Number(localStorage.getItem(SIDEBAR_W_KEY));
    if (v >= 180 && v <= 600) return v;
  } catch {
    if (typeof localStorage !== "undefined") pushToast(msg("sidebar-sizing-could-not-load-sidebar-width"), "error");
  }
  return 246;
}
export const [sidebarWidth, setSidebarWidth] = createSignal(loadSidebarWidth());
export function persistSidebarWidth() {
  try {
    localStorage.setItem(SIDEBAR_W_KEY, String(sidebarWidth()));
  } catch {
    pushToast(msg("sidebar-sizing-could-not-save-sidebar-width"), "error");
  }
}

const RS_W_KEY = "logseq-claude.rightSidebarWidth";
export const RIGHT_MIN = 220;
/** Sanity bound on a stored width (devices differ; the live bound is rightSidebarMaxWidth). */
const RIGHT_STORED_CAP = 4096;
/** GH #521. OG caps its right sidebar at 70% of the window (og-1.0.0
 * components/right_sidebar.cljs `sidebar-resizer`, max-ratio 0.7); Tine takes the
 * same fraction and also keeps the main pane at least this wide. */
export const RIGHT_MAX_WINDOW_FRACTION = 0.7;
export const MAIN_PANE_MIN = 360;

let leftReserve = 0;
/** The left sidebar's occupied width (0 when closed), so the main-pane minimum
 * counts it. Kept current by RightSidebar. */
export function setLeftSidebarReserve(px: number): void {
  leftReserve = px > 0 ? px : 0;
}

/** Widest the right sidebar may be in a window `windowWidth` px wide: at most 70% of
 * it, and never so wide that the main pane (beside the left sidebar, if open) drops
 * below MAIN_PANE_MIN. Never below RIGHT_MIN. Pure; O(1). */
export function rightSidebarMaxWidth(windowWidth: number, reserved = leftReserve): number {
  const byFraction = windowWidth * RIGHT_MAX_WINDOW_FRACTION;
  const byMainPane = windowWidth - reserved - MAIN_PANE_MIN;
  return Math.max(RIGHT_MIN, Math.floor(Math.min(byFraction, byMainPane)));
}
function loadRsWidth(): number {
  try {
    const v = Number(localStorage.getItem(RS_W_KEY));
    if (v >= RIGHT_MIN && v <= RIGHT_STORED_CAP) return v;
  } catch {
    if (typeof localStorage !== "undefined") pushToast(msg("sidebar-sizing-could-not-load-right-sidebar-width"), "error");
  }
  return 360;
}
export const [rightSidebarWidth, setRightSidebarWidth] = createSignal(loadRsWidth());
export function persistRightSidebarWidth() {
  try {
    localStorage.setItem(RS_W_KEY, String(rightSidebarWidth()));
  } catch {
    pushToast(msg("sidebar-sizing-could-not-save-right-sidebar-width"), "error");
  }
}

type Side = "left" | "right";

function currentWindowWidth(): number {
  const width = viewportOf(mainWindow).width;
  return width > 0 ? width : 1280; // no layout (node-pool tests): a typical laptop window
}

/** Set the side's width in pixels, clamped exactly as dragging (left 180–500,
 * right RIGHT_MIN..rightSidebarMaxWidth of the window). O(1); does not persist
 * until commitSidebarWidth is called. */
export function resizeSidebar(side: Side, width: number, windowWidth: number = currentWindowWidth()): void {
  if (side === "left") setSidebarWidth(Math.min(500, Math.max(180, width)));
  else setRightSidebarWidth(Math.min(rightSidebarMaxWidth(windowWidth), Math.max(RIGHT_MIN, width)));
}

/** Persist the side's current width on this device. O(1); failure shows a toast. */
export function commitSidebarWidth(side: Side): void {
  if (side === "left") persistSidebarWidth();
  else persistRightSidebarWidth();
}

/** Grow/shrink by 48 pixels, sharing pointer bounds and device persistence.
 * O(1). A closed sidebar stays closed; its next opening uses the new width. */
export function adjustSidebarWidth(side: Side, grow: boolean): void {
  resizeSidebar(side, (side === "left" ? sidebarWidth() : rightSidebarWidth()) + (grow ? 48 : -48));
  commitSidebarWidth(side);
}
