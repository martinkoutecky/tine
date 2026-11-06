import { Show, createEffect, createSignal, onCleanup, onMount, untrack, type JSX } from "solid-js";
import { observeViewportWindow } from "../viewportWindow";
import { reservedGroupHeight } from "../reservedGroupHeight";
import { preserveAboveViewport } from "../scrollContainer";
import { inPageFindOpen } from "../inpageFind";
import { printingOutline } from "../outlineViewport";
import { trackRetirementPin } from "../retirementPins";
import { queueReferenceCommit } from "../referenceAdmission";

/** Small chunks for exceptional DTO headers. Uses the outline scroller
 * observer, measured geometry and anchor transactions, with no text cache. */
export function MeasuredContentWindow(props: { heightKey: string; estimate: number; children: JSX.Element }): JSX.Element {
  let host!: HTMLSpanElement;
  let alive = true;
  const [near, setNear] = createSignal(typeof IntersectionObserver === "undefined");
  const [mounted, setMounted] = createSignal(untrack(near));
  const [pin, setPin] = createSignal(false);
  const height = reservedGroupHeight(() => props.heightKey, () => host, mounted, true);
  const wanted = () => near() || (height() > 0 && inPageFindOpen()) || (mounted() && (pin() || printingOutline()));
  createEffect(() => {
    if (!wanted()) { setMounted(false); return; }
    if (mounted()) return;
    let current = true;
    void queueReferenceCommit(() => preserveAboveViewport(host, () => setMounted(true), undefined, true),
      () => alive && current && wanted(), () => 0);
    onCleanup(() => { current = false; });
  });
  createEffect(() => {
    if (!mounted() || !host) return;
    const pins = trackRetirementPin(host);
    createEffect(() => setPin(pins.pinned()));
    onCleanup(() => { pins.stop(); setPin(false); });
  });
  onMount(() => onCleanup(observeViewportWindow(host, (entry, margin) => {
    preserveAboveViewport(host, () => {
      if (!entry.isIntersecting && untrack(mounted)) height.remember(entry.boundingClientRect.height, entry.boundingClientRect.width);
      setNear(entry.isIntersecting);
    }, undefined, false, entry, margin);
  }, "viewport")));
  onCleanup(() => { alive = false; });
  return <span ref={host} style={{ display: "block", "min-height": mounted() ? undefined : `${height() || props.estimate}px` }}>
    <Show when={mounted()}>{props.children}</Show>
  </span>;
}
