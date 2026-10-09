import { For, Show, createMemo, type JSX } from "solid-js";
import type { Format } from "./ast";
import { InlineText, PageRef } from "./inline";
import { isRenderHiddenProp, propertyKeyNorm } from "./block";
import { graphMeta } from "../graphSession";
import { isCommentBlock, isCommentPresentationKey } from "../comments";
import { node as docNode } from "../document";

// OG block.cljs property-cp/properties-cp: a linked key and one row per key.
// Duplicate values collapse in file order (last wins), as extract-properties.
export function PropertyRows(props: {
  entries: [string, string][]; format: Format; blockId?: string; macroExpansion?: boolean;
}): JSX.Element {
  const visible = createMemo(() => {
    // Margin comments (vision §3.7): `author` is the block's author chip and a
    // comment's quote keys are its header, so neither shows as a row.
    const comment = props.blockId !== undefined
      && isCommentBlock(props.entries, (docNode(props.blockId)?.parent ?? null) !== null);
    return [...new Map(props.entries.map(([key, value]) =>
      [propertyKeyNorm(key), value] as const))].filter(([key]) =>
      !isRenderHiddenProp(key, graphMeta()?.block_hidden_properties ?? [])
      && (props.blockId === undefined || !isCommentPresentationKey(key, comment)));
  });
  return <Show when={visible().length > 0}>
    <span class="block-properties">
      <For each={visible()}>{([key, value]) => <span class="prop block-property">
        <span class="prop-key block-property-key"><PageRef name={key} alias={key} blockId={props.blockId} /></span>{": "}
        <span class="prop-value block-property-val"><InlineText text={value} format={props.format} macroExpansion={props.macroExpansion} /></span>
      </span>}</For>
    </span>
  </Show>;
}
