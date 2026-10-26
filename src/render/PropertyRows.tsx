import { For, Show, createMemo, type JSX } from "solid-js";
import type { Format } from "./ast";
import { PageRef } from "./inline";
import { PagePropertyValue } from "../components/PagePropertyValue";
import { isRenderHiddenProp, propertyKeyNorm } from "./block";
import { graphMeta } from "../graphSession";

// OG block.cljs property-cp/properties-cp: a linked key and one row per key.
// Duplicate values collapse in file order (last wins), as extract-properties.
export function PropertyRows(props: {
  entries: [string, string][]; format: Format; blockId?: string; macroExpansion?: boolean;
}): JSX.Element {
  const visible = createMemo(() => [...new Map(props.entries.map(([key, value]) =>
    [propertyKeyNorm(key), value] as const))].filter(([key]) =>
    !isRenderHiddenProp(key, graphMeta()?.block_hidden_properties ?? [])));
  return <Show when={visible().length > 0}>
    <span class="block-properties">
      <For each={visible()}>{([key, value]) => <span class="prop block-property">
        <span class="prop-key block-property-key"><PageRef name={key} alias={key} blockId={props.blockId} /></span>{": "}
        <span class="prop-value block-property-val"><PagePropertyValue propertyKey={key} value={value} format={props.format} blockId={props.blockId} macroExpansion={props.macroExpansion} /></span>
      </span>}</For>
    </span>
  </Show>;
}
