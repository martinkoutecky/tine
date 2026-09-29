import { Show, createSignal, onCleanup, type JSX } from "solid-js";
import { observeNear, renderedBlocks, unobserveNear } from "../lazyObserve";
import { visibleBody } from "../render/block";

/** Bound standalone query/embed mounting to the near-viewport budget shared by
 * ordinary block bodies. The placeholder preserves source visibility without
 * starting expensive reference work for thousands of off-screen blocks. */
export function DeferredStandaloneMacro(props: {
  blockId: string;
  raw: string;
  children: JSX.Element;
}): JSX.Element {
  const [near, setNear] = createSignal(renderedBlocks.has(props.blockId));
  let deferredEl: Element | undefined;
  const observe = (el: Element) => {
    deferredEl = el;
    observeNear(el, () => {
      renderedBlocks.add(props.blockId);
      setNear(true);
    });
  };
  onCleanup(() => {
    if (deferredEl) unobserveNear(deferredEl);
  });
  return (
    <Show
      when={near()}
      fallback={<span ref={observe} class="ast-fallback ast-deferred">{visibleBody(props.raw).join("\n")}</span>}
    >
      {props.children}
    </Show>
  );
}
