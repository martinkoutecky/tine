// Graph home page: the ONE answer to "which page is home" (I-12). OG reads
// config.edn `:default-home {:page "..."}` (state/get-default-home) and keeps it
// only while that page exists (container.cljs `get-default-home-if-valid`); the
// :home route then redirects there, else shows the Journals feed. `g h` and
// graph open both land here. Nothing is ever created for a missing page.
import { backend } from "./backend";
import { graphMeta } from "./graphSession";
import { isJournalTitle } from "./journal";
import { graphOwner, readOwned } from "./owned";
import { focusedRouter } from "./panes";
import { openJournals, sameRoute } from "./router";
import { pushToast } from "./toasts";

/** Configured home page name, trimmed (the backend keeps the config value
 *  untrimmed and drops only blank ones); null when none. O(1), no I/O. */
export function configuredHomePage(): string | null {
  return graphMeta()?.default_home?.trim() || null;
}

/** What a home navigation did: `opened` the configured page; `unresolved` —
 *  none configured, the page no longer resolves, or its read failed (reported);
 *  `stale` — a graph rebind or a navigation of the focused route landed first. */
export type HomeOutcome = "opened" | "unresolved" | "stale";

/** Open the configured home page in place in the focused tab when it resolves
 *  and neither the graph binding, the focused pane/tab, nor that tab's route
 *  intent changed during the lookup. A name in the graph's journal title format
 *  resolves as that journal. "In place" matches master (GH #245/#276): it
 *  replaces the focused tab's route even when the tab is pinned (history keeps
 *  a Back entry), and on graph open it replaces the restored session tab.
 *  Cost: one `getPage`, which waits (no timeout) for the backend's initial
 *  whole-graph parse — O(pages) — so on graph open the home page can land
 *  seconds after the landing on a large graph, or not at all if the user
 *  navigates first. Writes nothing to the graph (navigation schedules the
 *  usual session save). The name is read from `graphMeta` at call time, which
 *  follows an outside config.edn edit, a Settings choice and a rename of the
 *  home page without reopening the graph. */
export async function openConfiguredHomePage(): Promise<HomeOutcome> {
  const name = configuredHomePage();
  if (!name) return "unresolved";
  // Own the focused pane's router, its active tab and its monotonic route
  // intent revision (the ContextMenu rename exemplar): an A→B→A navigation or
  // a focus move to another pane/tab showing an equal route retires the read.
  const router = focusedRouter();
  const tabId = router.activeId();
  const intent = router.routeIntentRevision();
  const startingRoute = { ...router.route() };
  const owner = graphOwner(() => focusedRouter() === router && router.activeId() === tabId
    && router.routeIntentRevision() === intent && sameRoute(router.route(), startingRoute));
  try {
    // A journal-titled home (OG resolves any page entity) opens that journal;
    // the classifier is the one `[[links]]` and favorites use.
    const kind = isJournalTitle(name) ? "journal" : "page";
    const read = await readOwned(owner, backend().getPage(name, kind));
    if (read.kind === "stale") return "stale";
    if (!read.value) return "unresolved";
    router.openPage(read.value.name, read.value.kind, { inPlace: true });
    return "opened";
  } catch (error) {
    if (!owner()) return "stale";
    pushToast(`Couldn't open the home page "${name}". (${String(error)})`, "error");
    return "unresolved";
  }
}

/** `g h`: the configured home page, else the Journals feed (OG's default home). */
export function goHome(): void {
  void openConfiguredHomePage().then((outcome) => {
    if (outcome === "unresolved") openJournals();
  });
}
