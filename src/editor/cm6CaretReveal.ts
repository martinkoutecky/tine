import { EditorView } from "@codemirror/view";
import { nearestScrollableY } from "../components/blockParts";
import { revealCaretRect, watchCaretReveal } from "./caretReveal";

/** CM focus already uses preventScroll. Route its scroll requests and late
 * layout changes through the same pane/notice/pin policy as the textarea. */
export function cm6CaretReveal(view: EditorView) {
  const editor = view.contentDOM;
  // A late image can turn a short page into a scrolling page. Keep watching
  // its pane even before it overflows; other hosts retain CM's native scrolling.
  const pane = nearestScrollableY(editor) ?? editor.closest<HTMLElement>(".main-content, .right-sidebar");
  let settling = view.hasFocus && pane !== null;
  const reveal = () => {
    settling = false;
    if (!view.hasFocus || !editor.isConnected) return;
    const rect = view.coordsAtPos(view.state.selection.main.head);
    if (!rect || rect.bottom <= rect.top) return;
    revealCaretRect(editor, pane, rect);
    const scroll = view.scrollDOM;
    if (scroll.scrollWidth > scroll.clientWidth) {
      const box = scroll.getBoundingClientRect();
      if (rect.left < box.left + 24) scroll.scrollLeft -= box.left + 24 - rect.left;
      else if (rect.right > box.right - 24) scroll.scrollLeft += rect.right - box.right + 24;
    }
  };
  const stopWatch = watchCaretReveal(editor, pane, reveal);
  const gestures = ["wheel", "touchstart", "pointerdown", "keydown"] as const;
  const userInput = () => { settling = false; };
  for (const type of gestures) pane?.addEventListener(type, userInput, { passive: true, capture: true });
  const cancel = () => {
    stopWatch();
    for (const type of gestures) pane?.removeEventListener(type, userInput, true);
  };
  return { cancel, extension: EditorView.scrollHandler.of(() => {
    if (!pane) return false;
    // Initial CM measurement may see interim window heights. The settling
    // watcher owns that first reveal; subsequent typing reveals immediately.
    if (!settling) reveal();
    return true;
  }) };
}
