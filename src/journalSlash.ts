import { pageByName } from "./document";
import { applyCompletion, pageInsert, withRefCompletionSpace } from "./editor/autocomplete";
import { journalTitle, parseJournalTitle, appNow } from "./journal";
import { captureAssetEditor, assetEditorIsCurrent } from "./assetLanding";
import { spaceAfterRefCompletion } from "./refCompletionSettings";
import { openDatePicker } from "./ui";
import { pushToast } from "./toasts";

/** Complete a journal date slash command from the loaded containing page and
 * active title format. O(1) in graph size. Today/Tomorrow/Yesterday use the local clock; That day
 * uses the containing journal date. A missing/nonjournal page or unparseable
 * That day clears the trigger and shows an info toast. Inserts a [[journal title]]
 * using journalTitle's active graph format and defaults. This only edits the active buffer; ordinary save
 * handles persistence and any save failure. */
export function runJournalSlash(
  action: "today" | "thatday" | "tomorrow" | "yesterday",
  pageName: string,
  replaceTrigger: (text: string) => void,
): void {
  const page = pageByName(pageName);
  const date = action === "thatday" ? page?.kind === "journal" ? parseJournalTitle(page.name) : null : appNow();
  if (date && (action === "tomorrow" || action === "yesterday")) date.setDate(date.getDate() + (action === "tomorrow" ? 1 : -1));
  replaceTrigger(date ? pageInsert(journalTitle(date)) : "");
  if (!date) pushToast("/thatday is only available on journal pages.", "info");
}

/** Open an arbitrary journal-date completion after consuming the slash trigger.
 * O(block text length) on commit, O(1) on opening. Commit inserts [[journal title]]
 * at the captured caret through the caller's ordinary edit path, honoring the
 * reference-completion space preference. journalTitle reads the active graph
 * format at commit. A changed buffer, editor identity, unmount or graph refuses
 * insertion with an info toast and closes the picker; caret movement alone does
 * not invalidate it. The picker commits once and closes. Escape/Back cancel
 * without invoking commit; the consumed slash trigger remains removed. */
export function openJournalDatePicker(textarea: HTMLTextAreaElement, commit: (raw: string) => void, mounted: () => boolean, resize: () => void): void {
  const token = captureAssetEditor(textarea);
  if (!token.editingBlockId) return;
  const before = textarea.value;
  const caret = textarea.selectionStart;
  const rect = textarea.getBoundingClientRect();
  openDatePicker(token.editingBlockId, { insertJournal: title => {
    if (!assetEditorIsCurrent(token, textarea, mounted()) || textarea.value !== before) {
      pushToast("The date was not inserted because the graph or block changed.", "info"); return;
    }
    const text = pageInsert(title);
    const inserted = applyCompletion(before, caret, caret, text);
    const result = withRefCompletionSpace(inserted.raw, inserted.caret, text, spaceAfterRefCompletion());
    commit(result.raw);
    textarea.value = result.raw;
    textarea.setSelectionRange(result.caret, result.caret);
    resize();
  } }, rect.left, rect.bottom + 4);
}
