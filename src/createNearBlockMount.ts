import { createSignal, onCleanup } from "solid-js";
import { observeNear, renderedBlocks, unobserveNear } from "./lazyObserve";

/** One observer registration/cleanup per placeholder. Missing ids render eagerly;
 * supplied ids latch on first intersection and render eagerly on remount. Body
 * ids are captured; standalone macro props choose the current-id latch. Returns
 * a ref callback with its near accessor, reusing that callback rather than
 * allocating a wrapper per block. O(1). */
export function createNearBlockMount(props: { blockId?: string }, latch: "captured" | "current" = "captured") {
  const blockId = props.blockId;
  const [near, setNear] = createSignal(blockId == null || renderedBlocks.has(blockId));
  let element: Element | undefined;
  function observe(el: Element) {
    element = el;
    observeNear(el, () => {
      const id = latch === "current" ? props.blockId : blockId;
      if (id != null) renderedBlocks.add(id);
      setNear(true);
    });
  };
  onCleanup(() => { if (element) unobserveNear(element); });
  observe.near = near;
  return observe;
}
