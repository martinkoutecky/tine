// Mouse-drag block selection, matching OG Logseq: a drag that starts inside a
// block selects text *within* that block normally, but the moment it crosses
// into another block it switches to whole-block (multi-block) selection. The
// block-selection state + ops (copy/cut/delete/indent/move) already exist in the
// store and are keyboard-wired; this only adds the pointer entry point.

import { selectBlock, extendSelectionTo } from "./store";

// Interactive chrome whose drags mean something else (bullet = reorder handle,
// links/buttons/chips = clicks). A text-selection drag must not start on these.
const CHROME = ".bullet-container, .block-controls, .collapse-toggle, a, button, .date-chip";
const SHEET_INTERNAL =
  ".block-sheet-container, .sheet-grid, .sheet-table, .sheet-board, .sheet-cell, .sheet-board-card";

function blockIdAt(x: number, y: number): string | null {
  const el = document.elementFromPoint(x, y);
  return el?.closest("[data-block-id]")?.getAttribute("data-block-id") ?? null;
}

export function installBlockSelectionDrag(): () => void {
  let armed = false; // a potential drag started inside a block's content
  let startId: string | null = null;
  let converting = false; // crossed a boundary → now in block-selection mode

  const onMouseDown = (e: MouseEvent) => {
    armed = false;
    converting = false;
    startId = null;
    if (e.button !== 0) return;
    const target = e.target as Element | null;
    if (!target || target.closest(CHROME)) return;
    if (target.closest(SHEET_INTERNAL)) return;
    const block = target.closest("[data-block-id]");
    if (!block) return; // not in the outline (sidebar, title, dialogs, …)
    startId = block.getAttribute("data-block-id");
    armed = !!startId;
  };

  const onMouseMove = (e: MouseEvent) => {
    if (!armed || startId === null) return;
    if (e.buttons === 0) {
      armed = false; // button was released without a mouseup reaching us
      return;
    }
    const overId = blockIdAt(e.clientX, e.clientY);
    if (overId === null) return; // off the outline — keep current selection
    // Still inside the start block and haven't crossed yet: leave the native
    // in-textarea text selection alone (this is the "select part of a bullet" case).
    if (!converting && overId === startId) return;
    if (!converting) {
      converting = true;
      selectBlock(startId); // exits editing (commits via blur), anchor = focus = start
    }
    extendSelectionTo(overId);
    window.getSelection()?.removeAllRanges(); // drop the half-formed text highlight
    e.preventDefault();
  };

  const onMouseUp = () => {
    armed = false;
    if (!converting) return;
    converting = false;
    // Swallow the click the browser synthesizes after this drag so it doesn't
    // start editing a block and clear the fresh selection. The synthetic click
    // fires synchronously before the timeout, so the timeout is just cleanup.
    const swallow = (ev: MouseEvent) => {
      ev.preventDefault();
      ev.stopPropagation();
      document.removeEventListener("click", swallow, true);
    };
    document.addEventListener("click", swallow, true);
    setTimeout(() => document.removeEventListener("click", swallow, true), 0);
  };

  document.addEventListener("mousedown", onMouseDown, true);
  document.addEventListener("mousemove", onMouseMove, true);
  document.addEventListener("mouseup", onMouseUp, true);
  return () => {
    document.removeEventListener("mousedown", onMouseDown, true);
    document.removeEventListener("mousemove", onMouseMove, true);
    document.removeEventListener("mouseup", onMouseUp, true);
  };
}
