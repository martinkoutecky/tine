import { Show } from "solid-js";
import { createFoldedThreadCount, createFoldedChangedThreadCount } from "../commentBelowCount";
import { commentMarginHidden } from "../commentMarginPref";
import { t } from "../i18n";

export function CommentBelowChip(props: { id: string; collapsed: boolean; expand: () => void }) {
  return <Show when={props.collapsed && !commentMarginHidden()}><FoldedCommentBelowChip {...props} /></Show>;
}

function FoldedCommentBelowChip(props: Parameters<typeof CommentBelowChip>[0]) {
  // Reads the comment index: structure and membership, never the text (rounds 2-3).
  const count = createFoldedThreadCount(() => props.id, () => true);
  // Threads below with a message changed since the user last looked (DESIGN §12): the page's
  // existing seen marks, no read of their own; 0 outside a git graph or a tracked page.
  const changed = createFoldedChangedThreadCount(() => props.id, () => true);
  return <Show when={count() > 0}>
    <button type="button" class="comment-below-chip" classList={{ "comment-below-changed": changed() > 0 }}
      aria-label={changed() > 0 ? t("comment-below-changed-expand", { count: count(), changed: changed() }) : t("comment-below-expand", { count: count() })}
      onMouseDown={e => e.stopPropagation()} onClick={e => { e.stopPropagation(); props.expand(); }}>
      {changed() > 0 ? t("comment-below-changed", { count: count(), changed: changed() }) : t("comment-below", { count: count() })}</button>
  </Show>;
}
