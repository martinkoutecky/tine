import { createMemo, For, Show, useContext, type JSX } from "solid-js";
import type { Format } from "../render/ast";
import {
  isLinkablePropertyKey,
  isQuotedPagePropertyValue,
  propertyKeyNorm,
} from "../render/block";
import { graphMeta } from "../graphSession";
import { InlineText, PageRef } from "../render/inline";
import { linkable_property_names } from "../render/wasm/lsdoc_wasm.js";
import { PAGE_ICON_PROPERTIES } from "../pageIconImage";
import { QueryLocationContext, queryBlockLocation, queryPageLocation } from "../render/queryLocation";

/** OG's linkable properties (alias/tags and every `:property/separated-by-commas`
 * key) display their parsed reference set, each comma item a page link.
 * Tokenization and plain-member extraction are shared with native property
 * atomization; the stored text stays untouched. Other properties retain their
 * inline text. */
export function PagePropertyValue(props: {
  propertyKey: string;
  value: string;
  format: Format;
  blockId?: string;
  page?: { kind?: string; name?: string };
  macroExpansion?: boolean;
}): JSX.Element {
  const parent = useContext(QueryLocationContext);
  const location = () => {
    const host = parent ?? (props.blockId ? queryBlockLocation(props.blockId)
      : props.page ? queryPageLocation(props.page.kind, props.page.name) : undefined);
    return host && `${host}/property/${JSON.stringify(propertyKeyNorm(props.propertyKey))}`;
  };
  const names = createMemo(() =>
    isLinkablePropertyKey(props.propertyKey, graphMeta()?.linkable_property_keys) && !isQuotedPagePropertyValue(props.value)
      ? linkable_property_names(props.value, props.format === "org")
      : [],
  );
  return (
    <Show when={names().length > 0}
      fallback={<Show when={!PAGE_ICON_PROPERTIES.includes(props.propertyKey.toLowerCase()) && !isQuotedPagePropertyValue(props.value)} fallback={props.value}>
        <QueryLocationContext.Provider value={location()}>
          <InlineText text={props.value} format={props.format} blockId={props.blockId} macroExpansion={props.macroExpansion} />
        </QueryLocationContext.Provider>
      </Show>}>
      <For each={names()}>
        {(name, index) => <>{index() > 0 ? ", " : ""}<PageRef name={name} alias={name} blockId={props.blockId} /></>}
      </For>
    </Show>
  );
}
