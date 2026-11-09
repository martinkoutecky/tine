import { createContext, createEffect, createMemo, createSignal, Show, useContext, type JSX } from "solid-js";
import { threadRootOf } from "../document";
import { commentMarginHidden } from "../commentMarginPref";
import { replyFocusRequest } from "../commentReplyFocus";
import { t, msg } from "../i18n";
import { hasPendingReplyDraft } from "../commentMargin";
import { pushToast } from "../toasts";
import { editingId, endEdit } from "../editorController";
import { CommentIndexContext, createCommentIndex } from "../commentIndex";

const ResolvedVisibility = createContext<() => boolean>(() => false);
export const useResolvedComments = (): (() => boolean) => useContext(ResolvedVisibility);

const sameIds = (a: readonly string[], b: readonly string[]) => a.length === b.length && a.every((id, i) => id === b[i]);

/** One state per rendered page (including journal days), sidebar item,
 * reference group and zoomed block. It owns the comment index of its trees
 * (`commentIndex.ts`, review round 3): thread roots and their resolution follow
 * structural and raw-property deltas, so a block that gains or loses a quote,
 * or a root that becomes a reply, is counted here at once. */
export function CommentResolutionScope(props: { roots: readonly string[]; children: JSX.Element }): JSX.Element {
  const index = createCommentIndex(() => props.roots);
  const [shown, setShown] = createSignal(false);
  const resolvedRoots = createMemo(() => index.roots().flatMap(root => (root.resolved ? [root.id] : [])), [], { equals: sameIds });
  const count = () => resolvedRoots().length;
  createEffect(() => {
    const request = replyFocusRequest();
    if (request && resolvedRoots().includes(request.id)) setShown(true);
  });
  return <CommentIndexContext.Provider value={index}><ResolvedVisibility.Provider value={shown}>
    <Show when={count() > 0 && !commentMarginHidden()}>
      <button type="button" class="comment-resolved-toggle" aria-pressed={shown()} onClick={() => {
        const resolved = resolvedRoots();
        if (shown() && hasPendingReplyDraft(resolved)) return pushToast(msg("comment-margin-draft-pending"), "warn");
        const editing = editingId();
        if (shown() && editing && resolved.includes(threadRootOf(editing) ?? "")) {
          endEdit("page-navigation");
          if (editingId() !== null) return;
        }
        setShown(value => !value);
      }}>{t("comment-resolved-count", { count: count() })}</button>
    </Show>
    {props.children}
  </ResolvedVisibility.Provider></CommentIndexContext.Provider>;
}
