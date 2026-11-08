import { createContext, createEffect, createMemo, createSignal, onCleanup, Show, untrack, useContext, type JSX } from "solid-js";
import { isCommentId, onBlockRawChanged, someBlockInTrees, threadRootOf } from "../document";
import { threadResolved } from "../commentResolution";
import { commentMarginHidden } from "../commentMarginPref";
import { replyFocusRequest } from "../commentReplyFocus";
import { t, msg } from "../i18n";
import { hasPendingReplyDraft } from "../commentMargin";
import { pushToast } from "../toasts";
import { editingId, endEdit } from "../editorController";
import type { Format } from "../types";

const ResolvedVisibility = createContext<() => boolean>(() => false);
export const useResolvedComments = (): (() => boolean) => useContext(ResolvedVisibility);

/** One state per rendered page (including journal days). Index structure once;
 * ordinary typing does no page scan, root edits update only their membership. */
export function CommentResolutionScope(props: { roots: readonly string[]; format: Format; children: JSX.Element }): JSX.Element {
  const [shown, setShown] = createSignal(false);
  const [count, setCount] = createSignal(0);
  const roots = createMemo(() => {
    const ids: string[] = [];
    someBlockInTrees(props.roots, n => {
      if (isCommentId(n.id, props.format) && threadRootOf(n.id, props.format) === n.id) ids.push(n.id);
      return false;
    });
    return ids;
  });
  createEffect(() => {
    const ids = roots();
    const resolved = new Set(untrack(() => ids.filter(threadResolved)));
    setCount(resolved.size);
    onCleanup(onBlockRawChanged(id => {
      if (threadResolved(id)) resolved.add(id); else resolved.delete(id);
      setCount(resolved.size);
    }, ids));
  });
  createEffect(() => {
    const request = replyFocusRequest();
    if (request && roots().includes(request.id) && threadResolved(request.id)) setShown(true);
  });
  return <ResolvedVisibility.Provider value={shown}>
    <Show when={count() > 0 && !commentMarginHidden()}>
      <button type="button" class="comment-resolved-toggle" aria-pressed={shown()} onClick={() => {
        const resolvedRoots = roots().filter(threadResolved);
        if (shown() && hasPendingReplyDraft(resolvedRoots)) return pushToast(msg("comment-margin-draft-pending"), "warn");
        const editing = editingId();
        if (shown() && editing && resolvedRoots.includes(threadRootOf(editing) ?? "")) {
          endEdit("page-navigation");
          if (editingId() !== null) return;
        }
        setShown(value => !value);
      }}>{t("comment-resolved-count", { count: count() })}</button>
    </Show>
    {props.children}
  </ResolvedVisibility.Provider>;
}
