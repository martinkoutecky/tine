import { For, type JSX } from "solid-js";
import type { Format } from "../render/ast";
import {
  isImplicitPageRefProperty,
  isQuotedPagePropertyValue,
  normalizeImplicitPageName,
} from "../render/block";
import { InlineText, PageRef } from "../render/inline";

/** Render Logseq's implicit page-reference properties without changing the
 * stored text. Bare alias/aliases/tags values become navigable, while explicit
 * inline markup, separators, spacing, custom properties, and quoted values keep
 * their authored representation. */
export function PagePropertyValue(props: {
  propertyKey: string;
  value: string;
  format: Format;
}): JSX.Element {
  if (!isImplicitPageRefProperty(props.propertyKey) || isQuotedPagePropertyValue(props.value)) {
    return <InlineText text={props.value} format={props.format} />;
  }
  return (
    <For each={props.value.split(/([,，])/g)}>
      {(part) => {
        if (part === "," || part === "，") return part;
        const leading = part.match(/^\s*/)?.[0] ?? "";
        const trailing = part.match(/\s*$/)?.[0] ?? "";
        const value = part.slice(leading.length, part.length - trailing.length);
        if (!value) return part;
        const name = normalizeImplicitPageName(value);
        return <>{leading}<PageRef name={name} alias={name} />{trailing}</>;
      }}
    </For>
  );
}
