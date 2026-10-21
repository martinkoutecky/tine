import { queryAllWindowsAll } from "../windowRealm";
import { offsetThroughRewrite } from "./caretThroughRewrite";
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

/** Run `write` (a commit that may canonicalise the block, GH #741: typing DOING
 *  appends the :LOGBOOK: clock-in) and, if that re-synced the textarea, keep the
 *  caret on the text it was on. The browser otherwise puts it at the end, and
 *  the next keystrokes land after `:END:`. OG never rewrites the buffer while
 *  editing. `enabled` is false for views whose textarea is not the raw text. */
export function keepCaretThroughRewrite(ta: HTMLTextAreaElement | undefined, enabled: boolean, write: () => void): void {
  const before = enabled && ta ? { value: ta.value, start: ta.selectionStart, end: ta.selectionEnd, direction: ta.selectionDirection } : null;
  write();
  if (!before || !ta || ta.value === before.value) return;
  ta.setSelectionRange(
    offsetThroughRewrite(before.value, ta.value, before.start),
    offsetThroughRewrite(before.value, ta.value, before.end),
    before.direction ?? undefined,
  );
}
