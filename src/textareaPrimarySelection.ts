import { isElementTag, listen, onEachWindow } from "./windowRealm";

// GH #530. WebKitGTK writes the Linux middle-click (PRIMARY) selection on every
// selection change, and builds its text/html flavour by walking the DOM from
// the selection start to a "node after the selection end". Inside a textarea
// the selection lives in the control's internal shadow tree; when it ends in the
// LAST text node of that tree (every single-line block, and any select-all) there
// is no node after it, so WebKit serializes from the textarea to the end of the
// document: the editing markup (`<textarea><div contenteditable="plaintext-only">`),
// the selection toolbar's buttons and every following block. Rich paste targets
// (Tuta, Anki, ...) prefer text/html and paste all of it.
//
// A textarea with a non-empty `placeholder` has a placeholder element after the
// inner editor in that shadow tree, which gives the walk a definite end. The
// space is invisible, and it is only displayed while the value is empty.
export const PRIMARY_SELECTION_GUARD_PLACEHOLDER = " ";

function guard(target: EventTarget | null): void {
  if (isElementTag(target, "textarea") && !target.placeholder) {
    target.placeholder = PRIMARY_SELECTION_GUARD_PLACEHOLDER;
  }
}

/** Guard every textarea, in every window, before its first selection: focus
 *  always precedes a selection made by the keyboard or the mouse. Returns the
 *  uninstaller. */
export function installTextareaPrimarySelectionGuard(): () => void {
  return onEachWindow((win) => listen(win, "focusin", (event) => guard(event.target), { capture: true, on: "document" }));
}
