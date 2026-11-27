import { appNow, journalTitle } from "./journal";
import { internalLinkDest } from "./linkGesture";
import { openRouteInOtherPane, revealOpenJournalsFeed } from "./panes";
import { openInNewTab, openJournals } from "./router";
import { openPageInSidebar } from "./ui";

/** Park today's journal in the right sidebar: OG's `:sidebar/open-today-page`
 *  and the Journals Shift-click. Nothing is written here; a day with no file
 *  yet gets one when it is first typed in, as in the Journals feed. */
export function openTodayInSidebar(): void {
  openPageInSidebar(journalTitle(appNow()), "journal");
}

/** A click or middle-click on Journals (left sidebar, toolbar, toolbar overflow): the one answer for all three.
 *  Shift opens today beside the current page (OG's Journals Shift-click), a plain click the feed here. Ctrl/Cmd,
 *  middle-click and Alt take the page-link destinations (linkGesture.ts, GH #1050): a feed some tab of this window
 *  already shows is brought forward instead; else Ctrl/Cmd and middle-click open it in a background tab, and Alt
 *  in the other pane, or here when the layout has no other pane. Logseq has no tabs or panes and treats these
 *  as a plain click. True when the feed is shown where the click was (a drawer then closes, as on a plain click). */
export function openJournalsFromClick(event: MouseEvent): boolean {
  if (event.shiftKey && event.button !== 1) {
    openTodayInSidebar();
    return false;
  }
  const dest = event.button === 1 ? "background" : internalLinkDest(event);
  if (dest !== "default" && revealOpenJournalsFeed()) return true;
  if (dest === "background") {
    openInNewTab({ kind: "journals" });
    return false;
  }
  if (dest === "pane" && openRouteInOtherPane({ kind: "journals" })) return false;
  openJournals();
  return true;
}
