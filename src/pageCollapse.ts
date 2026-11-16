import { pageRoots, setCollapsedTrees } from "./document";
import { focusedRouter } from "./panes";
import { resolveRouteBlock } from "./router";
import { appNow, journalTitle } from "./journal";

function currentRoots(): string[] {
  const route = focusedRouter().route();
  // OG falls back to date/today in the journal feed, rather than folding every
  // loaded historical day merely because it is visible in the same feed.
  if (route.kind === "journals") return [...pageRoots(journalTitle(appNow()))];
  if (route.kind !== "page") return [];
  // A zoomed block is the current outline, just as in OG's root-block walk.
  const zoom = resolveRouteBlock(route);
  if (zoom) return [zoom];
  return [...pageRoots(route.name)];
}

export function setCurrentPageCollapsed(collapsed: boolean): void {
  setCollapsedTrees(currentRoots(), collapsed);
}

/** OG toggle-open!: any folded block => expand everything, otherwise fold. */
export function toggleCurrentPageCollapsed(): void {
  setCollapsedTrees(currentRoots());
}
