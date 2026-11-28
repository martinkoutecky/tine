/** What kind of element a key event lands on. Pure DOM predicates, no state;
 * the window keyboard dispatcher (`keybindings.ts`) asks them per key. O(depth). */

/** A field that takes typed text: a textarea, any input, editable content or a
 *  CodeMirror block editor (including the controls inside it). */
export function isEditableTarget(t: EventTarget | null): boolean {
  const el = t as HTMLElement | null;
  if (!el) return false;
  const tag = el.tagName;
  return tag === "TEXTAREA" || tag === "INPUT" || el.isContentEditable
    || !!el.closest?.('[data-block-editor="cm6"]');
}

const CARET_INPUT_TYPES = new Set(["text", "search", "url", "tel", "email", "password", "number"]);

/** A field with a text caret: a textarea, a text-like input, or editable content
 *  (CodeMirror's). The target itself, not its editor wrapper: a button inside an
 *  editor has no caret. */
export function isTextCaretTarget(t: EventTarget | null): boolean {
  const el = t as HTMLElement | null;
  if (!el?.tagName) return false;
  if (el.tagName === "TEXTAREA") return true;
  if (el.tagName === "INPUT") return CARET_INPUT_TYPES.has((el as HTMLInputElement).type);
  // isContentEditable, with the nearest contenteditable attribute where it is missing.
  return el.isContentEditable ?? (el.closest?.("[contenteditable]")?.getAttribute("contenteditable") ?? "false") !== "false";
}
