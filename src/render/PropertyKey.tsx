import { Show } from "solid-js";
import { graphMeta } from "../graphSession";
import { PageRef } from "./inline";
import { isRenderHiddenPageProp, propertyKeyNorm } from "./block";

/** Page projections use the header policy, retaining aliases because they have
 * no AliasGroupHeader. Explicitly hidden aliases still stay hidden. */
export function visiblePageProperties(entries: readonly [string, string][]): [string, string][] {
  const hidden = graphMeta()?.block_hidden_properties ?? [];
  return entries.filter(([key]) => propertyKeyNorm(key) === "alias"
    ? !hidden.some(item => propertyKeyNorm(item) === "alias")
    : !isRenderHiddenPageProp(key, hidden));
}

/** OG 0.10.9 block.cljs property-cp: enabled unless explicitly false.
 * The excludelist controls backlink membership, not rendering (graph_parser/
 * block.cljs get-page-refs-from-property-names). Visibility belongs to the row.
 */
export function PropertyKey(props: { name: string; blockId?: string }) {
  const key = () => propertyKeyNorm(props.name);
  return <Show when={graphMeta()?.property_pages_enabled !== false} fallback={key()}>
    <PageRef name={key()} alias={key()} blockId={props.blockId} />
  </Show>;
}
