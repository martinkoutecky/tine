import { onCleanup } from "solid-js";
import { textareaCaretLineRect } from "../../editor/caretRows";
import { nearestScrollableY } from "../blockParts";
import { documentOf, listen, newResizeObserver, viewportOf } from "../../windowRealm";

/** The mobile formatting row can wrap at high interface zoom. Publish its
 * layout height so the help button clears the whole row, including a wrap. */
export function trackSelectionToolbarHeight(toolbar: HTMLDivElement): void {
  const root = documentOf(toolbar).documentElement;
  const update = () => {
    if (toolbar.isConnected && toolbar.offsetHeight > 0) {
      root.style.setProperty("--mobile-selection-toolbar-height", `${toolbar.offsetHeight}px`);
    }
  };
  const observer = newResizeObserver(toolbar, update);
  observer?.observe(toolbar);
  queueMicrotask(update);
  onCleanup(() => {
    observer?.disconnect();
    root.style.removeProperty("--mobile-selection-toolbar-height");
  });
}

/** Desktop selection toolbar placement (UI-SEL-TOOLBAR): above the selection's
 * first line, below its last when there is no room, clamped to the pane and
 * below a sticky seen notice. `active` is false on touch or with no selection. */
export function selectionToolbarPlacement(editor: () => HTMLTextAreaElement | undefined, active: () => boolean) {
  let toolbarRef: HTMLDivElement | undefined;
  const position = () => {
    const ref = editor();
    if (!active() || !toolbarRef || !ref?.isConnected) return;
    const toolbar = toolbarRef;
    const pane = ref.closest<HTMLElement>(".main-content, .right-sidebar") ?? nearestScrollableY(ref);
    const viewport = viewportOf(ref);
    const bounds = pane?.getBoundingClientRect();
    const leftEdge = Math.max(0, bounds?.left ?? 0) + 8;
    const rightEdge = Math.min(viewport.width, bounds?.right ?? viewport.width) - 8;
    const notice = pane?.querySelector<HTMLElement>("[data-seen-header]")?.getBoundingClientRect();
    const topEdge = Math.max(0, bounds?.top ?? 0, notice?.bottom ?? 0) + 4;
    const bottomEdge = Math.min(viewport.height, bounds?.bottom ?? viewport.height) - 4;
    toolbar.style.maxWidth = `${Math.max(0, rightEdge - leftEdge)}px`;
    const box = ref.getBoundingClientRect();
    const line = textareaCaretLineRect(ref, ref.selectionStart) ?? box;
    const end = textareaCaretLineRect(ref, Math.max(ref.selectionStart, ref.selectionEnd - 1)) ?? line;
    const size = toolbar.getBoundingClientRect();
    const above = line.top - size.height - 4;
    const top = Math.max(topEdge, Math.min(above >= topEdge ? above : end.bottom + 4, bottomEdge - size.height));
    const left = Math.max(leftEdge, Math.min(box.left, rightEdge - size.width));
    const parent = toolbar.offsetParent as HTMLElement | null;
    const origin = parent?.getBoundingClientRect();
    toolbar.style.top = `${top - (origin?.top ?? 0) + (parent?.scrollTop ?? 0) - (parent?.clientTop ?? 0)}px`;
    toolbar.style.left = `${left - (origin?.left ?? 0) + (parent?.scrollLeft ?? 0) - (parent?.clientLeft ?? 0)}px`;
    // A selection scrolled wholly out of the pane must not leave a stray menu
    // clamped on top of unrelated text. It reappears when that selection returns.
    toolbar.style.visibility = end.bottom < topEdge || line.top > bottomEdge ? "hidden" : "";
  };
  return {
    position,
    toolbar: (el: HTMLDivElement) => {
      toolbarRef = el;
      trackSelectionToolbarHeight(el);
      queueMicrotask(position);
    },
    /** Call from the editor's onMount: follows scroll and editor resizing. */
    follow: (ref: HTMLTextAreaElement) => {
      onCleanup(listen(ref, "scroll", position, true));
      onCleanup(listen(ref, "resize", position));
      const observer = newResizeObserver(ref, position);
      observer?.observe(ref);
      onCleanup(() => observer?.disconnect());
    },
  };
}
