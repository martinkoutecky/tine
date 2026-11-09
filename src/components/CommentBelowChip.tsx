import { Show } from "solid-js";
import { createFoldedThreadCount } from "../commentBelowCount";
import { commentMarginHidden } from "../commentMarginPref";
import { t } from "../i18n";
import type { Format } from "../types";

export function CommentBelowChip(props: { id: string; format: Format; collapsed: boolean; expand: () => void }) {
  // Follows structure and comment membership, never the text (review round 2).
  const count = createFoldedThreadCount(() => props.id, () => props.format, () => props.collapsed && !commentMarginHidden());
  return <Show when={count() > 0}>
    <button type="button" class="comment-below-chip" aria-label={t("comment-below-expand", { count: count() })}
      onMouseDown={e => e.stopPropagation()} onClick={e => { e.stopPropagation(); props.expand(); }}>{t("comment-below", { count: count() })}</button>
  </Show>;
}
