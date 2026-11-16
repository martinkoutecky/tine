// Bring one seen item (a changed block or a removed-block marker) into view for
// Previous / Next change (ADR 0079, D-20261029-SS). The same reveal the router
// and in-page find use: open the collapsed ancestors without an edit, mount the
// window that holds the row, scroll it in below the sticky notice, flash it.
// It never focuses a block or opens an editor.
import { node as docNode, revealNode } from "../document";
import { renderedBlocks } from "../lazyObserve";
import { revealOutlineBlock } from "../outlineViewport";
import { requestFrame, windowOf } from "../windowRealm";
import type { SeenItem } from "./steps";

const FRAMES = 24;
const FLASH_MS = 1200;

function find(item: SeenItem, scroller: HTMLElement): HTMLElement | null {
  const wanted = item.kind === "block" ? item.reveal : item.key;
  if (!wanted) return null;
  const candidates = item.kind === "block"
    ? scroller.querySelectorAll<HTMLElement>(".ls-block[data-block-id]")
    : scroller.querySelectorAll<HTMLElement>("[data-seen-item]");
  const attribute = item.kind === "block" ? "data-block-id" : "data-seen-item";
  for (const el of candidates) if (el.getAttribute(attribute) === wanted) return el;
  return null;
}

/** Is `el` in the scroller's view below the notice? A taller-than-view row only
 *  needs its top there. */
function placed(el: HTMLElement, scroller: HTMLElement): boolean {
  const view = scroller.getBoundingClientRect();
  const reserved = parseFloat(windowOf(scroller).getComputedStyle(scroller).getPropertyValue("--seen-header-height")) || 0;
  const top = view.top + scroller.clientTop + reserved;
  const bottom = view.top + scroller.clientTop + scroller.clientHeight;
  const box = el.getBoundingClientRect();
  return box.top >= top - 0.5 && (box.bottom <= bottom + 0.5 || (box.height > bottom - top && box.top <= bottom));
}

function bringIn(el: HTMLElement, scroller: HTMLElement): void {
  if (typeof el.scrollIntoView !== "function") return;
  const tall = el.getBoundingClientRect().height > scroller.clientHeight * 0.8;
  // scroll-padding-top on the scroller keeps the row clear of the sticky notice.
  el.scrollIntoView({ block: tall ? "start" : "center", behavior: "auto" });
}

function flash(el: HTMLElement): void {
  el.classList.add("block-flash");
  windowOf(el).setTimeout(() => el.classList.remove("block-flash"), FLASH_MS);
}

/** Resolve with the item's element once it is in view, or null when it never
 *  mounted or `current()` went false (the page changed, a newer step started).
 *  Windows remount around a scroll and shift rows, so the position is checked
 *  again on the next frames. */
export function revealSeenItem(item: SeenItem, scroller: HTMLElement, current: () => boolean): Promise<HTMLElement | null> {
  if (item.reveal) {
    renderedBlocks.add(item.reveal);
    // A collapsed parent renders none of its children; open them in memory only.
    for (let parent = docNode(item.reveal)?.parent ?? null; parent !== null; ) {
      const ancestor = docNode(parent);
      if (!ancestor) break;
      if (ancestor.collapsed) revealNode(parent);
      parent = ancestor.parent;
    }
  }
  // Only the stop we are on is highlighted, not the one stepped away from a moment ago.
  for (const old of scroller.querySelectorAll(".block-flash")) old.classList.remove("block-flash");
  return new Promise((resolve) => {
    let frames = 0;
    let shown: HTMLElement | null = null;
    let settled = 0;
    const tick = () => {
      if (!current()) { resolve(null); return; }
      if (item.reveal) revealOutlineBlock(item.reveal, scroller);
      const el = find(item, scroller);
      if (el) {
        if (el !== shown) { shown = el; flash(el); settled = 0; }
        if (placed(el, scroller)) { if (++settled >= 2) { resolve(el); return; } }
        else { settled = 0; bringIn(el, scroller); }
      }
      if (++frames >= FRAMES) { resolve(shown); return; }
      requestFrame(scroller, tick);
    };
    requestFrame(scroller, tick);
  });
}
