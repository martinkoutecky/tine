import { For, type JSX } from "solid-js";
import type { Format } from "../render/ast";
import {
  isImplicitPageRefProperty,
  isQuotedPagePropertyValue,
  normalizeImplicitPageName,
  splitLinkableProperty,
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
    <For each={implicitPropertyParts(props.value)}>
      {(part) => {
        if (typeof part === "string") return part;
        const name = normalizeImplicitPageName(part.value);
        return <>{part.leading}<PageRef name={name} alias={name} />{part.trailing}</>;
      }}
    </For>
  );
}

/** The value as separator text and navigable members, in order. The member
 *  boundaries come from the parser-side `split_linkable_property` (the same
 *  separator the native reference scan uses), never from a separator list of
 *  this file's own; the separator character itself is read back from the value. */
export function implicitPropertyParts(value: string): (string | { leading: string; value: string; trailing: string })[] {
  const out: (string | { leading: string; value: string; trailing: string })[] = [];
  let at = 0;
  splitLinkableProperty(value).forEach((member, index) => {
    if (index > 0) {
      out.push(value.slice(at, at + 1));
      at += 1;
    }
    at += member.length;
    const leading = member.match(/^\s*/)?.[0] ?? "";
    const trailing = member.match(/\s*$/)?.[0] ?? "";
    const trimmed = member.slice(leading.length, member.length - trailing.length);
    out.push(trimmed ? { leading, value: trimmed, trailing } : member);
  });
  return out;
}
