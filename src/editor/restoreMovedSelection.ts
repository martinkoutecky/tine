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

/** After a programmatic edit committed `committed`, put the textarea back in step
 *  with the store's text `live` and map the selection onto it. The commit may
 *  have canonicalised the block (a marker change clocks in or out, GH #741), and
 *  writing the pre-commit string back would let the next keystroke commit text
 *  without that transition. */
export function syncTextareaToStore(
  ta: HTMLTextAreaElement,
  committed: string,
  live: string,
  start: number,
  end = start,
  direction?: "forward" | "backward" | "none",
): void {
  ta.value = live;
  ta.setSelectionRange(offsetThroughRewrite(committed, live, start), offsetThroughRewrite(committed, live, end), direction);
}
