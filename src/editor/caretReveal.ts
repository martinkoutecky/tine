import { documentOf, requestFrame } from "../windowRealm";
import { textareaCaretRow } from "./caretRows";

/** GH #745. The one rule for where the view goes when the caret lands in a
 * block editor (Enter split, Backspace/Delete merge, paste, indent/outdent,
 * arrow navigation, click): the browser's own focus() scroll is not it.
 * Chromium centres the field (a merge into a tall image block threw the view up
 * by most of a screen); WebKitGTK does not scroll at all and left the caret off
 * screen. The editor focuses with `preventScroll` and calls this instead: the
 * view does not move while the caret row is already visible, and otherwise
 * moves by the least amount that shows it, below the sticky parent pins. */
const MARGIN = 8;
/** Height of one pinned ancestor row (`.sticky-ancestor`, app.css). */
const PIN = 30;

/** Pins that can cover the top of the scroller above `editor`: one per
 * `[data-sticky-parent]` ancestor block, as StickyAncestor counts them. */
function pinnedInset(editor: HTMLElement, scroller: HTMLElement): number {
  let count = 0;
  const block = editor.closest(".ls-block");
  for (let el = block?.parentElement ?? null; el && el !== scroller; el = el.parentElement) {
    if (el.hasAttribute("data-sticky-parent")) count++;
  }
  return count * PIN;
}

/** Scroll `scroller` the minimum that shows the caret row of the focused
 * `editor`. Returns whether it scrolled. O(editor text) for one row
 * measurement; a no-op without layout, an unfocused editor, or no scroller. */
export function revealCaret(editor: HTMLTextAreaElement, scroller: HTMLElement | null): boolean {
  if (!scroller || !editor.isConnected || !scroller.contains(editor) || documentOf(editor).activeElement !== editor) return false;
  const caret = editor.selectionDirection === "backward" ? editor.selectionStart : editor.selectionEnd;
  const row = textareaCaretRow(editor, caret ?? 0);
  if (!row) return false;
  const box = editor.getBoundingClientRect();
  const view = scroller.getBoundingClientRect();
  const top = box.top + editor.clientTop + row.top - editor.scrollTop;
  const bottom = box.top + editor.clientTop + row.bottom - editor.scrollTop;
  const visibleTop = view.top + scroller.clientTop + pinnedInset(editor, scroller) + MARGIN;
  const visibleBottom = view.top + scroller.clientTop + scroller.clientHeight - MARGIN;
  let delta = 0;
  if (top < visibleTop) delta = top - visibleTop;
  else if (bottom > visibleBottom) delta = bottom - visibleBottom;
  if (Math.abs(delta) <= 0.5) return false;
  scroller.scrollTop += delta;
  return true;
}

/** `revealCaret` now and again on the next `frames` animation frames, so a
 * layout that settles after the caret lands (windowed shells mounting, images
 * resolving their size, the edited block swapping its rendered view for the
 * textarea) cannot strand the caret off screen. Any wheel/touch/pointer/key
 * input on the scroller, a blur, or the returned `cancel()` ends it: the user
 * and any other focus owner always win. */
export function revealCaretSettled(editor: HTMLTextAreaElement, scroller: HTMLElement | null, frames = 2): () => void {
  revealCaret(editor, scroller);
  if (!scroller) return () => {};
  let stopped = false;
  let cancelFrame: (() => void) | undefined;
  const gestures = ["wheel", "touchstart", "pointerdown", "keydown"] as const;
  const userInput = () => stop();
  function stop() {
    if (stopped) return;
    stopped = true;
    cancelFrame?.();
    for (const type of gestures) scroller!.removeEventListener(type, userInput, true);
  }
  for (const type of gestures) scroller.addEventListener(type, userInput, { passive: true, capture: true });
  let left = frames;
  const tick = () => {
    if (stopped) return;
    if (documentOf(editor).activeElement !== editor) return stop();
    revealCaret(editor, scroller);
    if (--left > 0) cancelFrame = requestFrame(editor, tick);
    else stop();
  };
  cancelFrame = requestFrame(editor, tick);
  return stop;
}
