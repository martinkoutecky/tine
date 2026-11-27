import { appNow, journalTitle } from "./journal";
import { internalLinkDest, type InternalLinkDest } from "./linkGesture";
import { openRouteInOtherPane } from "./panes";
import { openInNewTab, openJournals } from "./router";
import { openPageInSidebar } from "./ui";

/** Park today's journal in the right sidebar: OG's `:sidebar/open-today-page`
 *  and the Journals Shift-click. Nothing is written here; a day with no file
 *  yet gets one when it is first typed in, as in the Journals feed. */
export function openTodayInSidebar(): void {
  openPageInSidebar(journalTitle(appNow()), "journal");
}

/** The Journals click (left sidebar, toolbar, toolbar overflow) at the shared
 *  link destination (linkGesture.ts, GH #1050): Shift opens today beside the
 *  current page (OG's `sidebar-journals!`), Ctrl/Cmd the feed in a background
 *  tab, Alt the feed in the other pane, a plain click the feed here; a feed the
 *  window already holds is focused instead (panes.ts, one feed). Logseq has
 *  no tabs or panes and ignores Ctrl/Alt on Journals as on page links; those two
 *  are Tine's link contract (GH #283/#438). Returns the destination so a caller
 *  completes its own navigation (the mobile drawer) only for "default". */
export function openJournalsFromClick(event: MouseEvent): InternalLinkDest {
  const dest = internalLinkDest(event);
  if (dest === "sidebar") openTodayInSidebar();
  else if (dest === "background") openJournalsInNewTab();
  else if (dest === "pane") openRouteInOtherPane({ kind: "journals" });
  else openJournals();
  return dest;
}

/** The feed in a background tab: Ctrl/Cmd+click and middle-click on Journals. */
export function openJournalsInNewTab(): void {
  openInNewTab({ kind: "journals" });
}
