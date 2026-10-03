/** Block navigation for sidebar, tab and pane actions. Opening a block never
 * writes the graph: OG stamps an `id::` only when a reference is created (copy
 * ref/embed, `((` autocomplete), so these actions hand the live ref straight to
 * the destination and a saved session names an ID-less block by position
 * (blockPositionRef, GH #623, I-2). */
import { blockRef } from "./document";
import { openInNewTab } from "./router";
import { openBlockInSidebar } from "./ui";
import { openRouteInOtherPane } from "./panes";

/** Open a block in the sidebar, a new tab, or the other pane. Synchronous: no save. */
export function openDurableBlock(id: string, destination: "sidebar" | "tab" | "pane"): void {
  const ref = blockRef(id);
  if (destination === "sidebar") openBlockInSidebar(ref);
  else {
    const route = { kind: "page" as const, name: ref.page, pageKind: ref.pageKind, block: ref.uuid, ...(ref.path ? { path: ref.path } : {}) };
    if (destination === "pane") openRouteInOtherPane(route);
    else openInNewTab(route);
  }
}
