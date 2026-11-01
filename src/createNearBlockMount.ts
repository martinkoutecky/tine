import { listenForOutlinePrint, printingOutline } from "./outlineViewport";
import { createSignal, onCleanup, onMount } from "solid-js";
import { observeNear, renderedBlocks, unobserveNear } from "./lazyObserve";
import { preserveAboveViewport } from "./scrollContainer";

/** One observer registration/cleanup per placeholder. Missing ids render eagerly;
 * supplied ids latch on first intersection and render eagerly on remount. Body
 * ids are captured; standalone macro props choose the current-id latch. Returns
 * a ref callback with its near accessor, reusing that callback rather than
 * allocating a wrapper per block. O(1). */
export function createNearBlockMount(props: { blockId?: string }, latch: "captured" | "current" = "captured") {
  onCleanup(listenForOutlinePrint());
  const blockId = props.blockId;
  const [near, setNear] = createSignal(blockId == null || renderedBlocks.has(blockId));
  let element: Element | undefined;
  let mounted = false;
  function observe(el: Element) {
    if (element && element !== el) unobserveNear(element);
    element = el;
    // A placeholder can first appear after mount (for example after printing).
    // Its ref still precedes insertion, so admit it after this DOM commit.
    if (mounted) queueMicrotask(() => { if (mounted && element === el) register(el); });
  };
  // JSX refs run before insertion. Resolve the scroller only after this body's
  // ancestors are attached, just like LiveRefGroup and outline windows (#810).
  onMount(() => {
    mounted = true;
    if (element) register(element);
  });
  function register(el: Element) {
    observeNear(el, (entry) => {
      // The placeholder itself is replaced; its parent survives the swap.
      preserveAboveViewport(el.parentElement ?? el, () => {
        const id = latch === "current" ? props.blockId : blockId;
        if (id != null) renderedBlocks.add(id);
        setNear(true);
      }, undefined, false, entry, 1200);
    });
  }
  onCleanup(() => { mounted = false; if (element) unobserveNear(element); });
  observe.near = () => near() || printingOutline();
  return observe;
}
