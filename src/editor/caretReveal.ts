import { documentOf, requestFrame, windowOf } from "../windowRealm";
import { textareaCaretLeft, textareaCaretRow } from "./caretRows";
import { cssInterfaceZoom } from "../zoom";

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

/** Android's interface zoom is CSS `zoom` on the document root (`zoom.ts`): rects
 * are in viewport pixels, while the pin and margin sizes here, computed-style
 * lengths, mirror rows, client* and scrollTop are pre-zoom CSS pixels. Every CSS
 * length is multiplied by this before it meets a rect, and every viewport delta
 * divided by it before it meets scrollTop. 1 everywhere else (desktop zooms
 * natively, which already converts). GH #783: unscaled, at zoom 1.25 the visible
 * top stopped 7px above the painted pins and a line was left under them. */
function zoom(): number {
  const z = cssInterfaceZoom();
  return z > 0 ? z : 1;
}

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
  const z = zoom();
  const viewTop = scroller.getBoundingClientRect().top + scroller.clientTop * z;
  const reserved = parseFloat(windowOf(scroller).getComputedStyle(scroller).getPropertyValue("--seen-header-height")) || 0;
  let bottom = 0;
  parents.forEach((ancestor, i) => {
    const slotBottom = viewTop + (reserved + (parents.length - i) * PIN) * z; // outermost pins below the notice
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
export function caretViewRow(editor: HTMLTextAreaElement, scroller: HTMLElement | null): { top: number; bottom: number; visibleTop: number; visibleBottom: number } | null {
  if (!scroller || !editor.isConnected || !scroller.contains(editor) || documentOf(editor).activeElement !== editor) return null;
  const caret = editor.selectionDirection === "backward" ? editor.selectionStart : editor.selectionEnd;
  const row = textareaCaretRow(editor, caret ?? 0);
  if (!row) return null;
  const z = zoom();
  const box = editor.getBoundingClientRect();
  const view = scroller.getBoundingClientRect();
  const top = box.top + (editor.clientTop + row.top - editor.scrollTop) * z;
  const bottom = box.top + (editor.clientTop + row.bottom - editor.scrollTop) * z;
  const reservedTop = parseFloat(windowOf(scroller).getComputedStyle(scroller).scrollPaddingTop) || 0;
  const visibleTop = Math.max(view.top + (scroller.clientTop + reservedTop) * z, pinnedBottom(editor, scroller)) + MARGIN * z;
  const visibleBottom = view.top + (scroller.clientTop + scroller.clientHeight - MARGIN) * z;
  return { top, bottom, visibleTop, visibleBottom };
}

/** Scroll `scroller` the minimum that shows the caret row of the focused
 * `editor`. Returns whether it scrolled. O(editor text) for one row
 * measurement; a no-op without layout, an unfocused editor, or no scroller. */
export function revealCaret(editor: HTMLTextAreaElement, scroller: HTMLElement | null, viewport?: { top: number; bottom: number }): boolean {
  const g = caretViewRow(editor, scroller);
  if (!g || !scroller) return false;
  const top = Math.max(g.visibleTop, viewport ? viewport.top + MARGIN * zoom() : -Infinity);
  const bottom = Math.min(g.visibleBottom, viewport ? viewport.bottom - MARGIN * zoom() : Infinity);
  let delta = 0;
  if (g.top < top) delta = g.top - top;
  else if (g.bottom > bottom) delta = g.bottom - bottom;
  if (Math.abs(delta) <= 0.5) return false;
  scroller.scrollTop += delta / zoom();
  return true;
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
export function scrollToClearToolbar(editor: HTMLTextAreaElement, scroller: HTMLElement | null, toolbarTop: number): number {
  const limit = toolbarTop - MARGIN * zoom();
  const bottomRule = editor.getBoundingClientRect().bottom - limit;
  if (bottomRule <= 0) return 0;
  const g = caretViewRow(editor, scroller);
  if (!g) return bottomRule;
  const room = limit - g.visibleTop;
  if (editor.getBoundingClientRect().height <= room) return bottomRule;
  return Math.max(0, g.bottom - limit);
}

/** Where the keyboard puts the edited line back when something has moved it out
 * of view after the keyboard opened (GH #783, beta.4 report). Not the minimum
 * move: that parked the line at the top edge, just under the pinned parents, and
 * on the reporter's phone the pins still covered it (the block's first line was
 * hidden under the pin, its caret line squeezed beneath). The line goes as low as
 * it can instead, just above the keyboard toolbar - where 0.7.4's only rule, the
 * editor's bottom 8px above the toolbar (GH #384), left it and the reporter found
 * it right. The same anchor as `scrollToClearToolbar`, so the next keystroke's
 * toolbar reveal does not move it again: the editor's bottom while the whole
 * editor fits between the pins and the toolbar, else the caret row's bottom.
 * A caret row already inside the visible band is left alone, as 0.7.4 did; the
 * scroller clamps at either end of the page. `band` is the visual viewport's top
 * and the toolbar's top in viewport pixels, converted to scrollTop pixels under
 * Android's CSS interface zoom. Returns whether it scrolled. */
export function revealCaretAboveKeyboard(editor: HTMLTextAreaElement, scroller: HTMLElement | null, band: { top: number; bottom: number }): boolean {
  const delta = keyboardCaretOffset(editor, scroller, band);
  if (!scroller || Math.abs(delta) <= 0.5) return false;
  scroller.scrollTop += delta / zoom();
  return true;
}

/** The viewport-pixel scroll `revealCaretAboveKeyboard` makes; 0 while the caret
 * row is visible or without layout. Exported for its unit test. */
export function keyboardCaretOffset(editor: HTMLTextAreaElement, scroller: HTMLElement | null, band: { top: number; bottom: number }): number {
  const g = caretViewRow(editor, scroller);
  if (!g) return 0;
  const top = Math.max(g.visibleTop, band.top + MARGIN * zoom());
  const bottom = Math.min(g.visibleBottom, band.bottom - MARGIN * zoom());
  if (g.top >= top && g.bottom <= bottom) return 0;
  const box = editor.getBoundingClientRect();
  return (box.height <= bottom - top ? box.bottom : g.bottom) - bottom;
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
export function revealCaretSettled(editor: HTMLTextAreaElement, scroller: HTMLElement | null, stableFrames = 2, maxFrames = 40, lateMs = 10_000, watch: {
  /** Keyboard resizing and WebView focus scrolling can finish in separate turns.
   * Follow their scroll events after the initial reveal; user input still wins.
   * With lateMs=Infinity this is event-driven until cancelled, never polling. */
  scroll?: boolean;
  /** Replaces the minimum reveal (the keyboard's low placement, GH #783). */
  reveal?: () => boolean;
} = {}): () => void {
  if (!scroller) return () => {};
  let stopped = false;
  let cancelFrame: (() => void) | undefined;
  let lateTimer: number | undefined;
  let lateFrame: (() => void) | undefined;
  const gestures = ["wheel", "touchstart", "pointerdown", "keydown"] as const;
  const late = watch.scroll ? ["load", "error", "scroll"] : ["load", "error"];
  const win = windowOf(editor);
  const vv = watch.scroll ? win.visualViewport : null;
  const reveal = watch.reveal ?? (() => revealCaret(editor, scroller));
  const userInput = () => stop();
  function stop() {
    if (stopped) return;
    stopped = true;
    cancelFrame?.();
    lateFrame?.();
    if (lateTimer !== undefined) windowOf(editor).clearTimeout(lateTimer);
    for (const type of gestures) scroller!.removeEventListener(type, userInput, true);
    for (const type of late) scroller!.removeEventListener(type, onLate, true);
    vv?.removeEventListener("scroll", onLate);
    if (watch.scroll) win.removeEventListener("scroll", onLate);
    editor.removeEventListener("blur", stop);
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
    vv?.addEventListener("scroll", onLate, { passive: true });
    if (watch.scroll) win.addEventListener("scroll", onLate, { passive: true });
    if (Number.isFinite(lateMs)) lateTimer = windowOf(editor).setTimeout(stop, lateMs);
  }
  editor.addEventListener("blur", stop);
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
