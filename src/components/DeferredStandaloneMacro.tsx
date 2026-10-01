import { Show, type JSX } from "solid-js";
import { createNearBlockMount } from "../createNearBlockMount";
import { visibleBody } from "../render/block";

/** Bound standalone query/embed mounting to the near-viewport budget shared by
 * ordinary block bodies. The placeholder preserves source visibility without
 * starting expensive reference work for thousands of off-screen blocks. */
export function DeferredStandaloneMacro(props: {
  blockId: string;
  raw: string;
  children: JSX.Element;
}): JSX.Element {
  const observe = createNearBlockMount(props, "current");
  return (
    <Show
      when={observe.near()}
      fallback={<span ref={observe} class="ast-fallback ast-deferred">{visibleBody(props.raw).join("\n")}</span>}
    >
      {props.children}
    </Show>
  );
}
