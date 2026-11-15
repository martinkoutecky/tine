import { appNow, journalTitle } from "./journal";
import { openJournals } from "./router";
import { openPageInSidebar } from "./ui";

/** Park today's journal in the right sidebar: OG's `:sidebar/open-today-page`
 *  and the Journals Shift-click. Nothing is written here; a day with no file
 *  yet gets one when it is first typed in, as in the Journals feed. */
export function openTodayInSidebar(): void {
  openPageInSidebar(journalTitle(appNow()), "journal");
}

/** OG's Journals click gesture: Shift opens today beside the current page. */
export function openJournalsFromClick(event: MouseEvent): void {
  if (event.shiftKey) openTodayInSidebar();
  else openJournals();
}
