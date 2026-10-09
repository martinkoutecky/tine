import type { JSX } from "solid-js";

/** Shared presentation for reference/query disclosures; the host owns the action. */
export function ReferenceDisclosure(props: {
  collapsed: boolean;
  class?: string;
  title?: string;
  onClick?: JSX.EventHandlerUnion<HTMLSpanElement, MouseEvent>;
}): JSX.Element {
  return <span class={`ref-collapse${props.class ? ` ${props.class}` : ""}`}
    classList={{ collapsed: props.collapsed }} aria-hidden={props.onClick ? undefined : "true"} title={props.title} onClick={props.onClick}>
    <svg viewBox="0 0 24 24" class="triangle"><path d="M8 5l8 7-8 7z" /></svg>
  </span>;
}
