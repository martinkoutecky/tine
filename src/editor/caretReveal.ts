import { documentOf, isElementTag, requestFrame, windowOf } from "../windowRealm";
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
  const reserved = parseFloat(windowOf(scroller).getComputedStyle(scroller).getPropertyValue("--seen-header-height")) || 0;
  let bottom = 0;
  parents.forEach((ancestor, i) => {
    const slotBottom = viewTop + reserved + (parents.length - i) * PIN; // outermost pins below the notice
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

/** Where the caret row of the focused `editor` is, in viewport pixels, and the
 * top edge the view must keep it below (scroller top, its scroll-padding, the
 * sticky pins, the margin). Null without layout, an unfocused editor, or no scroller. */
export function caretViewRow(editor: HTMLTextAreaElement, scroller: HTMLElement | null): CaretViewRow | null {
  if (!scroller || !editor.isConnected || !scroller.contains(editor) || documentOf(editor).activeElement !== editor) return null;
  const caret = editor.selectionDirection === "backward" ? editor.selectionStart : editor.selectionEnd;
  const row = textareaCaretRow(editor, caret ?? 0);
  if (!row) return null;
  const box = editor.getBoundingClientRect();
  const top = box.top + editor.clientTop + row.top - editor.scrollTop;
  const bottom = box.top + editor.clientTop + row.bottom - editor.scrollTop;
  return caretViewBounds(editor, scroller, { top, bottom });
}

interface CaretViewRow { top: number; bottom: number; visibleTop: number; visibleBottom: number }

/** Both editor engines supply viewport coordinates; pane/pin policy has one owner. */
function caretViewBounds(editor: HTMLElement, scroller: HTMLElement | null, row: { top: number; bottom: number }): CaretViewRow | null {
  if (!scroller || !editor.isConnected || !scroller.contains(editor) || documentOf(editor).activeElement !== editor) return null;
  const view = scroller.getBoundingClientRect();
  const reservedTop = parseFloat(windowOf(scroller).getComputedStyle(scroller).scrollPaddingTop) || 0;
  const visibleTop = Math.max(view.top + scroller.clientTop + reservedTop, pinnedBottom(editor, scroller)) + MARGIN;
  const visibleBottom = view.top + scroller.clientTop + scroller.clientHeight - MARGIN;
  return { top: row.top, bottom: row.bottom, visibleTop, visibleBottom };
}

function scrollToShow(scroller: HTMLElement | null, g: CaretViewRow | null): boolean {
  if (!g || !scroller) return false;
  let delta = 0;
  if (g.top < g.visibleTop) delta = g.top - g.visibleTop;
  else if (g.bottom > g.visibleBottom) delta = g.bottom - g.visibleBottom;
  if (Math.abs(delta) <= 0.5) return false;
  scroller.scrollTop += delta;
  return true;
}

/** Scroll `scroller` the minimum that shows the caret row of the focused
 * `editor`. Returns whether it scrolled. O(editor text) for one row
 * measurement; a no-op without layout, an unfocused editor, or no scroller. */
export function revealCaret(editor: HTMLTextAreaElement, scroller: HTMLElement | null): boolean {
  return scrollToShow(scroller, caretViewRow(editor, scroller));
}

/** The same reveal for an editor engine that measures its own caret row (CM6). */
export function revealCaretRect(editor: HTMLElement, scroller: HTMLElement | null, row: { top: number; bottom: number }): boolean {
  return scrollToShow(scroller, caretViewBounds(editor, scroller, row));
}

/** How far to scroll so the focused editor clears a keyboard toolbar whose top is
 * `toolbarTop` (GH #384; GH #783). The rule is the editor's bottom edge, 8px above
 * the toolbar, but only while the whole editor fits the room between the pins and
 * the toolbar: then the caret row, which is inside it, stays visible too. An
 * editor taller than that room (a long paragraph) cannot fit, and scrolling its
 * bottom above the toolbar carried the caret row off the top of the screen - once
 * from the dock appearing before the keyboard had resized the view, again at the
 * resize, and nothing scrolled it back when the keyboard outran the settled
 * reveal. For such an editor only the caret row is brought clear of the toolbar,
 * and never upward. Without layout the rule is the editor's bottom, as before. */
export function scrollToClearToolbar(editor: HTMLElement, scroller: HTMLElement | null, toolbarTop: number): number {
  const limit = toolbarTop - MARGIN;
  const bottomRule = editor.getBoundingClientRect().bottom - limit;
  if (bottomRule <= 0) return 0;
  // The CodeMirror editor measures its caret itself; it keeps the bottom rule.
  const g = isElementTag(editor, "textarea") ? caretViewRow(editor, scroller) : null;
  if (!g) return bottomRule;
  const room = limit - g.visibleTop;
  if (editor.getBoundingClientRect().height <= room) return bottomRule;
  return Math.max(0, g.bottom - limit);
}

/** Reveal the caret once the layout around the freshly focused editor has
 * stopped moving. A structural edit remounts block windows, and for a few frames
 * the editor sits wherever the interim (estimated) heights put it - measured
 * 6000px off in the real app, so revealing at mount scrolled to a position the
 * settled layout never had. The signature (editor top, scrollTop, scrollHeight)
 * must repeat for `stableFrames` consecutive animation frames before the one
 * reveal; after `maxFrames` frames it reveals anyway, so the caret is never
 * left off screen. The reveal does not end the watch: for `lateMs` more, an
 * image that finishes loading (`load`/`error`, which do not bubble but can be
 * captured) above the caret - an asset-cache miss decoded after the frame cap
 * - moves the editor again, so the caret is re-checked one frame after each and
 * brought back if it left the view. Any wheel/touch/pointer/key input on the
 * scroller, a blur, or the returned `cancel()` ends it: the user and any other
 * focus owner always win. */
export function revealCaretSettled(editor: HTMLTextAreaElement, scroller: HTMLElement | null, stableFrames = 2, maxFrames = 40, lateMs = 10_000): () => void {
  return watchCaretReveal(editor, scroller, () => revealCaret(editor, scroller), stableFrames, maxFrames, lateMs);
}

export function watchCaretReveal(editor: HTMLElement, scroller: HTMLElement | null, reveal: () => void, stableFrames = 2, maxFrames = 40, lateMs = 10_000): () => void {
  if (!scroller) return () => {};
  let stopped = false;
  let cancelFrame: (() => void) | undefined;
  let lateTimer: number | undefined;
  let lateFrame: (() => void) | undefined;
  const gestures = ["wheel", "touchstart", "pointerdown", "keydown"] as const;
  const late = ["load", "error"] as const;
  const userInput = () => stop();
  function stop() {
    if (stopped) return;
    stopped = true;
    cancelFrame?.();
    lateFrame?.();
    if (lateTimer !== undefined) windowOf(editor).clearTimeout(lateTimer);
    for (const type of gestures) scroller!.removeEventListener(type, userInput, true);
    for (const type of late) scroller!.removeEventListener(type, onLate, true);
  }
  function onLate() {
    if (stopped || lateFrame) return;
    lateFrame = requestFrame(editor, () => {
      lateFrame = undefined;
      if (stopped) return;
      if (!editor.isConnected || documentOf(editor).activeElement !== editor) return stop();
      reveal();
    });
  }
  function follow() {
    cancelFrame = undefined;
    if (stopped || lateMs <= 0) return stop();
    for (const type of late) scroller!.addEventListener(type, onLate, { passive: true, capture: true });
    lateTimer = windowOf(editor).setTimeout(stop, lateMs);
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
      reveal();
      return follow();
    }
    cancelFrame = requestFrame(editor, tick);
  };
  cancelFrame = requestFrame(editor, tick);
  return stop;
}
