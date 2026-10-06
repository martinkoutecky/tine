import { documentOf, registeredWindows, selectionOf } from "./windowRealm";
// A drag that MOVES something is not a text gesture.
//
// WebKit will happily start a text selection from wherever the pointer went
// down and smear it across everything the drag passes over: sidebar favorites
// smeared their titles blue (GH #211 follow-up, Martin 2026-08-25) and an
// outline block dragged by its bullet turned every block it crossed blue on
// macOS (GH #424). Chromium/WebView2 does not, which is why the same build
// looked clean on Windows.
//
// `preventDefault()` on the initial pointerdown would fix it and break more:
// these drags deliberately preserve focus and the ordinary click that follows a
// non-drag press. So the document is made unselectable only once the drag
// threshold is crossed, and any selection the press already began is dropped.
// WebKit can re-anchor a selection mid-drag, so callers call dropSelection()
// again on every move — the class by itself is not enough.
//
// Both call sites are transient: the class exists between the drag threshold
// and the drop, and is released on drop and on pointercancel.

export const DRAG_SELECTION_CLASS = "drag-selection-suppressed";

/** Discard whatever the press already selected. */
export function dropSelection(realm?: Node | Event): void {
  const selection = selectionOf(realm);
  if (selection && selection.rangeCount > 0) selection.removeAllRanges();
}

/** Make the whole document of `realm`'s window unselectable for the duration
 * of a move-drag. Releasing clears every Tine window, so a drag that ended in
 * another window never leaves one stuck unselectable. */
export function setDragSelectionSuppressed(on: boolean, realm?: Node | Event): void {
  if (on) {
    documentOf(realm).documentElement.classList.add(DRAG_SELECTION_CLASS);
    dropSelection(realm);
    return;
  }
  for (const win of registeredWindows()) win.document.documentElement.classList.remove(DRAG_SELECTION_CLASS);
}
