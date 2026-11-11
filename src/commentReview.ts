/** Page comment review. Each resolution scope owns its order and focus cursor.
 * The margin and compact views read its groups; commands and visible controls
 * walk that same order. A step scans at most the page's threads, reveals only
 * the target's ancestor path/window, and requests the existing live composer.
 * No graph reads or writes, except the ordinary persisted ancestor expansion.
 * Mounted scopes register their host and release it on unmount. */
import { createContext, createMemo, createSignal, useContext, type Accessor } from "solid-js";
import type { CommentIndex } from "./commentIndex";
import { formatForBlock, node, expandAncestors, threadRootOf } from "./document";
import { groupCommentEntries, threadOrder } from "./commentGroups";
import { focusCommentReply } from "./commentReplyFocus";
import { revealCommentMargin } from "./commentMargin";
import { editingId, endEdit } from "./editorController";
import { revealOutlineBlock } from "./outlineViewport";
import { activeDocument } from "./windowRealm";
import { focusedPaneId } from "./panes";
import { clearOnBindingInvalidated } from "./binding";
import { resolvedCommentAnchorTop } from "./components/resolvedCommentAnchor";
import { scrollContainer } from "./scrollContainer";

export interface CommentReview {
  groups(parent: string): readonly string[][];
  order(): readonly string[];
  canMove(step: 1 | -1, from?: string): boolean;
  move(step: 1 | -1, from?: string): void;
  focus(id: string): void;
  registerRow(parent: string, row: HTMLElement, surface: string): () => void;
  mount(host: HTMLElement): () => void;
}
export const CommentReviewContext = createContext<CommentReview>();
export const useCommentReview = () => useContext(CommentReviewContext);

/** Maintain the page's review index ahead of commands, so even a first step
 * only walks cached order. Membership uses the existing index; only a parent's
 * raw text or comment metadata re-anchors its groups. Cards remain view-owned. */
export function createCommentReview(index: CommentIndex, shown: Accessor<boolean>): CommentReview {
  let host: HTMLElement | undefined;
  const parents = createMemo(() => new Map(index.parents().map(parent => {
    const children = index.childrenOf(parent);
    return [parent, createMemo(() => groupCommentEntries(node(parent)?.raw ?? "",
      children().entries.filter(entry => shown() || !entry.resolved), formatForBlock(parent)))] as const;
  })));
  const order = createMemo(() => [...parents().values()].flatMap(groups => threadOrder(groups())));
  const [current, setCurrent] = createSignal<string>();
  const rows = new Map<string, { row: HTMLElement; surface: string }>();
  const destination = (step: 1 | -1, from = current()) => {
    const ids = order(), at = from ? ids.indexOf(from) : -1;
    return ids[at < 0 ? (step === 1 ? 0 : ids.length - 1) : at + step];
  };
  const review: CommentReview = {
    groups: parent => parents().get(parent)?.() ?? [], order,
    mount(element) { host = element; return registerCommentReview(element, review); },
    focus(id) { setCurrent(id); if (host) last = host; },
    registerRow(parent, row, surface) { rows.set(parent, { row, surface }); return () => { if (rows.get(parent)?.row === row) rows.delete(parent); }; },
    canMove: (step, from) => !!destination(step, from),
    move(step, from) {
      const id = destination(step, from), parent = id && node(id)?.parent;
      if (!id || !parent || !host) return;
      endEdit("page-navigation");
      if (editingId() !== null) return;
      revealCommentMargin();
      expandAncestors(parent);
      revealOutlineBlock(parent, host);
      setCurrent(id);
      const entry = rows.get(parent), row = entry?.row;
      if (!row) return;
      const group = new Set(review.groups(parent).find(ids => ids.includes(id)));
      const anchor = [...row.querySelectorAll<HTMLElement>(".comment-quote-anchor[data-comment-id]")].find(el => group.has(el.dataset.commentId!));
      if (anchor) anchor.scrollIntoView({ block: "center" });
      else {
        const top = resolvedCommentAnchorTop(id, parent, row), scroller = scrollContainer(row);
        if (top !== null && scroller) {
          const bounds = scroller.getBoundingClientRect();
          scroller.scrollTop += top - bounds.top - bounds.height / 2;
        } else row.scrollIntoView({ block: "center" });
      }
      focusCommentReply(id, entry.surface, true);
    },
  };
  return review;
}

const scopes = new Map<HTMLElement, CommentReview>();
let last: HTMLElement | undefined;
clearOnBindingInvalidated(() => { scopes.clear(); last = undefined; });
/** Register one mounted review scope. Pointer/focus entry selects a journal
 * day; palette focus may leave it without changing that selection. */
export function registerCommentReview(host: HTMLElement, review: CommentReview): () => void {
  scopes.set(host, review);
  const select = (event: Event) => {
    const target = event.target as HTMLElement;
    if (event.composedPath().find(element => scopes.has(element as HTMLElement)) !== host) return;
    last = host;
    const id = target.closest<HTMLElement>("[data-reply-root]")?.dataset.replyRoot ?? target.closest<HTMLElement>("[data-block-id]")?.dataset.blockId;
    const root = id && threadRootOf(id);
    if (root) review.focus(root);
  };
  host.addEventListener("pointerdown", select); host.addEventListener("focusin", select);
  return () => {
    scopes.delete(host); if (last === host) last = undefined;
    host.removeEventListener("pointerdown", select); host.removeEventListener("focusin", select);
  };
}

/** Palette/rebindable command. Uses the active pane's selected day, else the
 * first visible day; never steps across days or into another pane/window. */
export function stepPageComment(step: 1 | -1): void {
  const doc = activeDocument();
  const eligible = (host: HTMLElement) => host.ownerDocument === doc && host.isConnected &&
    (!host.closest<HTMLElement>("[data-pane-id]") || host.closest<HTMLElement>("[data-pane-id]")!.dataset.paneId === focusedPaneId());
  const nested = (host: HTMLElement) => {
    for (let parent = host.parentElement; parent; parent = parent.parentElement) if (scopes.has(parent)) return true;
    return false;
  };
  const picked = last && eligible(last) ? last : [...scopes.keys()].find(host => eligible(host) &&
    !nested(host) && (host.querySelector(".page-blocks")?.getBoundingClientRect().bottom ?? 0) >= 0);
  if (picked) { last = picked; scopes.get(picked)?.move(step); }
}
