/** A page embed's main row contains the entire foreign outline. Its controls
 * mark the visible host bullet, not the bottom of that outline (GH #898).
 * IO and caret reveal share this; host-kind changes remount the pin. */
export function stickyRow(block: Element): HTMLElement | null {
  return block.querySelector<HTMLElement>(":scope > .block-main > [data-sticky-row]")
    ?? block.querySelector<HTMLElement>(":scope > .block-main");
}
