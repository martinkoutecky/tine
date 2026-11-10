import { createContext, For, Show, useContext, type JSX } from "solid-js";
import { Block, SurfaceContext } from "./Block";
import { CommentQuoteHeader } from "./CommentParts";
import { blockWritable, formatForBlock, node } from "../document";
import { quoteSelectorOf } from "../comments";
import { facetsOf } from "../render/facets";
import { appendThreadForPassage } from "../commentActions";
import { t } from "../i18n";
import { passageKey } from "../commentGroups";

export const CommentGroupContext = createContext<() => readonly string[]>();
export function quoteInGroup(id: string): () => boolean {
  const group = useContext(CommentGroupContext);
  return () => !!group && group().length > 1 && group().includes(id);
}

/** Each root retains its own message actions, reply composer and future resolve
 * action slot. Only the passage header and outer card are shared. */
export function CommentGroup(props: { ids: readonly string[]; render?: (id: string) => JSX.Element }): JSX.Element {
  const surface = useContext(SurfaceContext);
  const first = () => props.ids[0];
  const selector = () => quoteSelectorOf(facetsOf(node(first())!.raw, formatForBlock(first())).properties);
  const resolved = () => {
    const parent = node(node(first())!.parent!);
    return !!parent && !!selector() && passageKey(parent.raw, selector()!, formatForBlock(first())) !== null;
  };
  return <div class="comment-group" data-comment-group={first()}>
    <Show when={props.ids.length > 1}><CommentQuoteHeader parentId={node(first())!.parent} selector={selector()} /></Show>
    <CommentGroupContext.Provider value={() => props.ids}>
      <For each={props.ids}>{id => props.render ? props.render(id) : <Block id={id} />}</For>
    </CommentGroupContext.Provider>
    <Show when={blockWritable(first()) && resolved()}>
      {/* Keep blur from collapsing the empty composer and moving this button during its press. */}
      <button type="button" class="comment-new-thread" onMouseDown={e => { e.preventDefault(); e.stopPropagation(); }} onClick={e => { e.stopPropagation(); appendThreadForPassage(first(), surface); }}>{t("comment-new-thread")}</button>
    </Show>
  </div>;
}
