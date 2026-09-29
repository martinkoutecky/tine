import { pageByName } from "./document";
import { pageInsert } from "./editor/autocomplete";
import { journalTitle, parseJournalTitle, appNow } from "./journal";
import { pushToast } from "./toasts";

/** Complete a journal date slash command from the loaded containing page and
 * active title format. O(1) in graph size. Today uses the local clock; That day
 * uses the containing journal date. A non-journal That day clears the trigger
 * and shows an info toast. This only edits the active buffer; ordinary save
 * handles persistence and any save failure. */
export function runJournalSlash(
  action: "today" | "thatday",
  pageName: string,
  replaceTrigger: (text: string) => void,
): void {
  const page = pageByName(pageName);
  const date = action === "today" ? appNow() : page?.kind === "journal" ? parseJournalTitle(page.name) : null;
  replaceTrigger(date ? pageInsert(journalTitle(date)) : "");
  if (!date) pushToast("/thatday is only available on journal pages.", "info");
}
