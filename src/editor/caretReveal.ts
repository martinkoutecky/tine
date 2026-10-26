import { documentOf, requestFrame } from "../windowRealm";
import { textareaCaretLeft, textareaCaretRow } from "./caretRows";

/** GH #745. The one rule for where the view goes when the caret lands in a
 * block editor (Enter split, Backspace/Delete merge, paste, indent/outdent,
 * arrow navigation, click): the browser's own focus() scroll is not it.
 * Chromium centres the field (a merge into a tall image block threw the view up
 * by most of a screen); WebKitGTK does not scroll at all and left the caret off
 * screen. The editor focuses with `preventScroll` and calls this instead: the
 * view does not move while the caret row is already visible, and otherwise
 * moves by the least amount that shows it, below the sticky parent pins. */
const MARGIN = 8;
/** Height of one pinned ancestor row and the slot each depth pins into
 * (`StickyAncestor`: depth * 30 from the scroller top). */
const PIN = 30;

/** Viewport y below which the sticky parent pins above `editor` end. A pin is
 * shown when its ancestor's own row lies wholly above the pin's slot, the very
 * rule `StickyAncestor` feeds its IntersectionObserver; that is read here from
 * geometry, not from the pin's `hidden` flag, which a remounted block only
 * flips a frame or two later. 0 when nothing is pinned. */
function pinnedBottom(editor: HTMLElement, scroller: HTMLElement): number {
  const parents: Element[] = [];
  for (let el = editor.closest(".ls-block")?.parentElement ?? null; el && el !== scroller; el = el.parentElement) {
    if (el.hasAttribute("data-sticky-parent")) parents.push(el);
  }
  const viewTop = scroller.getBoundingClientRect().top + scroller.clientTop;
  let bottom = 0;
  parents.forEach((ancestor, i) => {
    const slotBottom = viewTop + (parents.length - i) * PIN; // the outermost ancestor pins at the top
    const row = ancestor.querySelector(":scope > .block-main");
    if (row && row.getBoundingClientRect().bottom <= slotBottom) bottom = Math.max(bottom, slotBottom);
  });
  return bottom;
}

/** A `wrap="off"` editor (a code card) mounts with its whole value assigned,
 * which parks the selection at the end; focusing reveals that end and the
 * later setSelectionRange does not scroll back, so a long line opened the
 * editor on blank space hundreds of columns past the code (GH #489). Put the
 * horizontal view where the caret is; one mirror measure, and only for an
 * editor that can scroll horizontally at all, never an ordinary block. */
export function revealCaretColumn(ref: HTMLTextAreaElement | undefined, offset: number): void {
  if (!ref || ref.scrollWidth <= ref.clientWidth) return;
  const x = textareaCaretLeft(ref, offset);
  if (x === null) return;
  const margin = 24;
  if (x < ref.scrollLeft + margin) ref.scrollLeft = Math.max(0, x - margin);
  else if (x > ref.scrollLeft + ref.clientWidth - margin) ref.scrollLeft = x - ref.clientWidth + margin;
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
  const visibleTop = Math.max(view.top + scroller.clientTop, pinnedBottom(editor, scroller)) + MARGIN;
  const visibleBottom = view.top + scroller.clientTop + scroller.clientHeight - MARGIN;
  let delta = 0;
  if (top < visibleTop) delta = top - visibleTop;
  else if (bottom > visibleBottom) delta = bottom - visibleBottom;
  if (Math.abs(delta) <= 0.5) return false;
  scroller.scrollTop += delta;
  return true;
}

/** Reveal the caret once the layout around the freshly focused editor has
 * stopped moving. A structural edit remounts block windows, and for a few frames
 * the editor sits wherever the interim (estimated) heights put it - measured
 * 6000px off in the real app, so revealing at mount scrolled to a position the
 * settled layout never had. The signature (editor top, scrollTop, scrollHeight)
 * must repeat for `stableFrames` consecutive animation frames before the one
 * reveal; after `maxFrames` frames it reveals anyway, so the caret is never
 * left off screen. Any wheel/touch/pointer/key input on the scroller, a blur,
 * or the returned `cancel()` ends it: the user and any other focus owner always
 * win. */
export function revealCaretSettled(editor: HTMLTextAreaElement, scroller: HTMLElement | null, stableFrames = 2, maxFrames = 40): () => void {
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
  const signature = () => `${editor.getBoundingClientRect().top}|${scroller.scrollTop}|${scroller.scrollHeight}`;
  let last = signature();
  let same = 0;
  let frames = 0;
  const tick = () => {
    if (stopped) return;
    if (!editor.isConnected || documentOf(editor).activeElement !== editor) return stop();
    const now = signature();
    same = now === last ? same + 1 : 0;
    last = now;
    if (same >= stableFrames || ++frames >= maxFrames) {
      revealCaret(editor, scroller);
      return stop();
    }
    cancelFrame = requestFrame(editor, tick);
  };
  cancelFrame = requestFrame(editor, tick);
  return stop;
}
