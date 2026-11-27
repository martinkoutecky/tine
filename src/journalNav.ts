import { journalTitle, parseJournalTitle, appNow } from "./journal";
import { openPage, route } from "./router";

/** Open the adjacent journal route (+1 next, -1 previous) from the current
 * journal date, or today's local date on another route. O(1) before navigation;
 * the page loader owns any read error. No page is written here. */
export function goAdjacentJournal(dir: 1 | -1): void {
  const current = route();
  const anchor = current.kind === "page" && current.pageKind === "journal"
    ? parseJournalTitle(current.name) ?? appNow()
    : appNow();
  openPage(journalTitle(new Date(anchor.getFullYear(), anchor.getMonth(), anchor.getDate() + dir)), "journal");
}
