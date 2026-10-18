/** Native sticky geometry stays inside the existing windowed subtree shell.
 * Only mounted parents register; IO reports boundary crossings, with no scroll
 * listener, page traversal, model-sized cache or per-frame JavaScript. Mount
 * costs O(rendered ancestor depth), label changes O(this block's text). */
import { createSignal, onCleanup, onMount, type JSX } from "solid-js";
import { newIntersectionObserver } from "../windowRealm";
import { revealOutlineBlock } from "../outlineViewport";
import { visibleBody } from "../render/block";
import { InlineText } from "../render/inline";
import { formatForBlock } from "../document";

/** GH #643: above outline content (which stays at z-index 8 or below) and below
 * every resizer, menu, toolbar and modal (20 and up). Shallower pins stay on
 * top of deeper ones while one slides out; past ten levels later rows paint last. */
export function stickyZ(depth: number): number {
  return 9 + Math.max(0, 10 - depth);
}

export function StickyAncestor(props: { id: string; raw: string }): JSX.Element {
  let host!: HTMLDivElement;
  const [above, setAbove] = createSignal(false);
  const [depth, setDepth] = createSignal(0);
  onMount(() => {
    const block = host.parentElement!;
    const row = block.querySelector<HTMLElement>(":scope > .block-main")!;
    // These are the existing scroll shells of pages/feeds/panes/windows and
    // sidebar cards. Do not re-measure computed styles at every ancestor of
    // every mounted parent (quadratic style reads on a deep chain).
    const root = host.closest<HTMLElement>(".main-content, .right-sidebar");
    if (!root) return;
    let count = 0;
    for (let el = block.parentElement; el && el !== root; el = el.parentElement) {
      if (el.hasAttribute("data-sticky-parent")) count++;
    }
    setDepth(count);
    row.style.scrollMarginTop = `${count * 30}px`;
    const observer = newIntersectionObserver(host, entries => {
      for (const entry of entries) {
        // Out below the viewport is not an offscreen ancestor. Replace the
        // row when its bottom reaches the bottom of its sticky slot. Waiting
        // for the slot's top lets the next child enter that slot first, then
        // suddenly covers it when this pin appears (GH #717/#718).
        setAbove(!entry.isIntersecting && entry.boundingClientRect.bottom <= (entry.rootBounds?.top ?? 0));
      }
    }, { root, rootMargin: `-${(count + 1) * 30}px 0px 0px 0px` });
    observer?.observe(row);
    onCleanup(() => observer?.disconnect());
  });
  return <div ref={host} class="sticky-ancestor" data-sticky-block={props.id}
    style={{ top: `${depth() * 30}px`, "z-index": String(stickyZ(depth())) }}>
    <button type="button" hidden={!above()} onClick={() => {
      const block = host.parentElement!;
      revealOutlineBlock(props.id, block);
      block.querySelector<HTMLElement>(":scope > .block-main")?.scrollIntoView({ block: "start" });
    }}>
      {/* GH #643: the same toggle and bullet columns as the row, so the pinned
          bullet sits over the real one, and the first line rendered as the zoom
          breadcrumb renders it (marks, links and block refs, not raw syntax). */}
      <span class="sticky-ancestor-toggle" aria-hidden="true" />
      <span class="sticky-ancestor-bullet" aria-hidden="true"><span class="sticky-ancestor-dot" /></span>
      <span class="sticky-ancestor-label"><InlineText text={visibleBody(props.raw)[0] || "…"} format={formatForBlock(props.id)} /></span>
    </button>
  </div>;
}
