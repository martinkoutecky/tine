import { queryAllWindowsAll } from "../windowRealm";
/** A feed move can remount a block's editor under another day. Find its live
 * textarea before restoring the selection captured by the keyboard handler. */
export function restoreMovedSelection(
  old: HTMLTextAreaElement,
  blockId: string,
  start: number,
  end: number,
  direction: "forward" | "backward" | "none",
): void {
  const live = old.isConnected ? old : queryAllWindowsAll<HTMLTextAreaElement>("textarea.block-editor")
    .find((candidate) => candidate.closest("[data-block-id]")?.getAttribute("data-block-id") === blockId);
  if (!live) return;
  live.focus();
  live.setSelectionRange(Math.min(start, live.value.length), Math.min(end, live.value.length), direction);
}
