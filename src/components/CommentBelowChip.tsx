import { createMemo, Show } from "solid-js";
import { descendantOpenThreads } from "../commentGroups";
import { commentMarginHidden } from "../commentMarginPref";
import { t } from "../i18n";
import type { Format } from "../types";

export function CommentBelowChip(props: { id: string; format: Format; collapsed: boolean; expand: () => void }) {
  const count = createMemo(() => props.collapsed && !commentMarginHidden() ? descendantOpenThreads(props.id, props.format) : 0);
  return <Show when={count() > 0}>
    <button type="button" class="comment-below-chip" aria-label={t("comment-below-expand", { count: count() })}
      onMouseDown={e => e.stopPropagation()} onClick={e => { e.stopPropagation(); props.expand(); }}>{t("comment-below", { count: count() })}</button>
  </Show>;
}
