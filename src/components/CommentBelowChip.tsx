import { Show } from "solid-js";
import { createFoldedThreadCount } from "../commentBelowCount";
import { commentMarginHidden } from "../commentMarginPref";
import { t } from "../i18n";

export function CommentBelowChip(props: { id: string; collapsed: boolean; expand: () => void }) {
  // Reads the comment index: structure and membership, never the text (rounds 2-3).
  const count = createFoldedThreadCount(() => props.id, () => props.collapsed && !commentMarginHidden());
  return <Show when={count() > 0}>
    <button type="button" class="comment-below-chip" aria-label={t("comment-below-expand", { count: count() })}
      onMouseDown={e => e.stopPropagation()} onClick={e => { e.stopPropagation(); props.expand(); }}>{t("comment-below", { count: count() })}</button>
  </Show>;
}
