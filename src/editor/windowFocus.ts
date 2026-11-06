import { onCleanup, onMount } from "solid-js";
import { editingId } from "../editorController";
import { documentHasFocus, listen, onEachWindow, windowOf } from "../windowRealm";

/** App switches preserve a transaction; moving to another Tine window ends it.
 * Each adapter restores its own selection coordinates after native focus returns. */
export function editorWindowFocus<Selection>(ctx: {
  id: string;
  element(): HTMLElement | undefined;
  selection(): Selection;
  focus(selection: Selection | null): void;
  exit(): void;
}): { remember(): void } {
  let saved: Selection | null = null;
  onMount(() => onCleanup(onEachWindow(win => listen(win, "focus", () => {
    const element = ctx.element();
    if (editingId() !== ctx.id || !element?.isConnected) return;
    if (win === windowOf(element)) {
      ctx.focus(saved);
      saved = null;
    } else if (!documentHasFocus(element)) {
      saved = null;
      ctx.exit();
    }
  }))));
  return { remember: () => { saved = ctx.selection(); } };
}
