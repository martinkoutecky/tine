/** Device-local sidebar widths. Reads and changes are O(1), independent of the graph.
 * Keyboard and pointer resizing share resizeSidebar's bounds; commitSidebarWidth
 * persists the current width using the existing localStorage keys. Load/save
 * failures show a toast and keep the in-memory width usable. Callers need no
 * knowledge of storage keys or bounds; live drag updates commit on release.
 */
import { createSignal } from "solid-js";
import { pushToast } from "./toasts";

const SIDEBAR_W_KEY = "logseq-claude.sidebarWidth";
function loadSidebarWidth(): number {
  try {
    const v = Number(localStorage.getItem(SIDEBAR_W_KEY));
    if (v >= 180 && v <= 600) return v;
  } catch {
    if (typeof localStorage !== "undefined") pushToast("Could not load sidebar width.", "error");
  }
  return 246;
}
export const [sidebarWidth, setSidebarWidth] = createSignal(loadSidebarWidth());
export function persistSidebarWidth() {
  try {
    localStorage.setItem(SIDEBAR_W_KEY, String(sidebarWidth()));
  } catch {
    pushToast("Could not save sidebar width.", "error");
  }
}

const RS_W_KEY = "logseq-claude.rightSidebarWidth";
function loadRsWidth(): number {
  try {
    const v = Number(localStorage.getItem(RS_W_KEY));
    if (v >= 220 && v <= 800) return v;
  } catch {
    if (typeof localStorage !== "undefined") pushToast("Could not load right sidebar width.", "error");
  }
  return 360;
}
export const [rightSidebarWidth, setRightSidebarWidth] = createSignal(loadRsWidth());
export function persistRightSidebarWidth() {
  try {
    localStorage.setItem(RS_W_KEY, String(rightSidebarWidth()));
  } catch {
    pushToast("Could not save right sidebar width.", "error");
  }
}

type Side = "left" | "right";

/** Set the side's width in pixels, clamped exactly as dragging (left 180–500,
 * right 220–800). O(1); does not persist until commitSidebarWidth is called. */
export function resizeSidebar(side: Side, width: number): void {
  if (side === "left") setSidebarWidth(Math.min(500, Math.max(180, width)));
  else setRightSidebarWidth(Math.min(800, Math.max(220, width)));
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
