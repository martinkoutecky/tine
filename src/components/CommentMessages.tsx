import { createMemo, For, Show, type JSX } from "solid-js";
import { Block } from "./Block";
import { node } from "../document";
import { datedThreadRoot, commentMessageIds } from "../document";
import { t } from "../i18n";

/** File pre-order messages, independent of body folds. The root owns the one
 * traversal: O(thread blocks), zero reads, writes or IPC. Legacy roots return
 * no projection and continue through the ordinary recursive renderer. */
export function CommentMessages(props: { id: string; collapsed: boolean; expand: () => void }): JSX.Element {
  const ids = createMemo(() => commentMessageIds(props.id));
  return <>
    <Show when={props.collapsed}>
      <button type="button" class="comment-expand" onClick={props.expand}>{t("comment-replies", { count: datedThreadRoot(props.id) ? ids().length : legacyReplies(props.id) })}</button>
    </Show>
    <Show when={!props.collapsed}><div class="comment-messages"><For each={ids()}>{id => <Block id={id} />}</For></div></Show>
  </>;
}

function legacyReplies(id: string): number {
  return (node(id)?.children ?? []).reduce((count, child) => count + 1 + legacyReplies(child), 0);
}
