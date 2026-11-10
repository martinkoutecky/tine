// Test support: open a block's compact comment thread the way a touch tablet
// does (tap on the highlight, D-20261108-MC7). A thread without a highlight
// (quoted text changed, ambiguous, whole block, inside code or across markup)
// keeps a block marker (MC8), which is used instead.
export function openCommentThread(root: ParentNode, id?: string): void {
  const mark = root.querySelector<HTMLElement>(id ? `.comment-quote-anchor[data-comment-id="${id}"]` : ".comment-quote-anchor[data-comment-id]");
  if (mark) tapHighlight(mark);
  else root.querySelector<HTMLButtonElement>(".margin-marker")?.click();
}

/** A touch tap: pointerdown with pointerType touch, then the compatibility click. */
export function tapHighlight(mark: Element): void {
  const View = mark.ownerDocument.defaultView as Window & typeof globalThis;
  const PointerCtor = (View.PointerEvent ?? View.MouseEvent) as typeof MouseEvent;
  const down = new PointerCtor("pointerdown", { bubbles: true, cancelable: true });
  Object.defineProperty(down, "pointerType", { value: "touch" });
  mark.dispatchEvent(down);
  mark.dispatchEvent(new View.MouseEvent("click", { bubbles: true, cancelable: true }));
}

/** Hover the highlight with a real mouse pointer (the preview's trigger). */
export function hoverHighlight(mark: Element): void {
  const View = mark.ownerDocument.defaultView as Window & typeof globalThis;
  const PointerCtor = (View.PointerEvent ?? View.MouseEvent) as typeof MouseEvent;
  const event = new PointerCtor("pointerover", { bubbles: true, relatedTarget: null });
  Object.defineProperty(event, "pointerType", { value: "mouse" });
  mark.dispatchEvent(event);
}

/** Leave the highlight with the mouse. */
export function leaveHighlight(mark: Element): void {
  const View = mark.ownerDocument.defaultView as Window & typeof globalThis;
  const PointerCtor = (View.PointerEvent ?? View.MouseEvent) as typeof MouseEvent;
  const event = new PointerCtor("pointerout", { bubbles: true, relatedTarget: mark.ownerDocument.body });
  Object.defineProperty(event, "pointerType", { value: "mouse" });
  mark.dispatchEvent(event);
}

/** In an open thread view, walk passages with the view's own previous/next
 * controls until thread `id` is shown (only the shown passage is mounted,
 * review round 3). Returns its row, or null when the view has no such thread. */
export function showThread(id: string, doc: Document = document): HTMLElement | null {
  const popup = () => doc.querySelector<HTMLElement>(".comment-thread-popup");
  const button = (cls: string) => popup()?.querySelector<HTMLButtonElement>(cls) ?? null;
  const shown = () => popup()?.querySelector<HTMLElement>(`.ls-block[data-block-id="${id}"]`) ?? null;
  while (!shown() && button(".comment-prev-thread") && !button(".comment-prev-thread")!.disabled) button(".comment-prev-thread")!.click();
  for (;;) {
    const row = shown();
    const next = button(".comment-next-thread");
    if (row || !next || next.disabled) return row;
    next.click();
  }
}
