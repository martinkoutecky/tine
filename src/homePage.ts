// Graph home page: the ONE answer to "which page is home" (I-12). OG reads
// config.edn `:default-home {:page "..."}` (state/get-default-home) and keeps it
// only while that page exists (container.cljs `get-default-home-if-valid`); the
// :home route then redirects there, else shows the Journals feed. `g h` and
// graph open both land here. Nothing is ever created for a missing page.
import { backend } from "./backend";
import { graphMeta } from "./graphSession";
import { graphOwner, readOwned } from "./owned";
import { openJournals, openPage, route, sameRoute } from "./router";
import { pushToast } from "./toasts";

/** Configured home page name, trimmed; null when none. O(1), no I/O. */
export function configuredHomePage(): string | null {
  return graphMeta()?.default_home?.trim() || null;
}

/** What a home navigation did: `opened` the configured page; `unresolved` —
 *  none configured, the page no longer resolves, or its read failed (reported);
 *  `stale` — a graph rebind or a navigation of the focused route landed first. */
export type HomeOutcome = "opened" | "unresolved" | "stale";

/** Open the configured home page in place in the focused tab when it resolves
 *  and neither the graph binding nor the focused route changed during the
 *  lookup. Cost: one page read when a home page is configured; writes nothing. */
export async function openConfiguredHomePage(): Promise<HomeOutcome> {
  const name = configuredHomePage();
  if (!name) return "unresolved";
  const startingRoute = { ...route() };
  const owner = graphOwner(() => sameRoute(route(), startingRoute));
  try {
    const read = await readOwned(owner, backend().getPage(name, "page"));
    if (read.kind === "stale") return "stale";
    if (!read.value) return "unresolved";
    openPage(read.value.name, "page", { inPlace: true });
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
