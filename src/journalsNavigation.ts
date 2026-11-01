import { appNow, journalTitle } from "./journal";
import { openJournals } from "./router";
import { openPageInSidebar } from "./ui";

/** OG's Journals click gesture: Shift opens today beside the current page. */
export function openJournalsFromClick(event: MouseEvent): void {
  if (event.shiftKey) openPageInSidebar(journalTitle(appNow()), "journal");
  else openJournals();
}
