import { createResource, createRoot } from "solid-js";
import { backend } from "./backend";
import { graphEpoch } from "./ui";
import type { PageEntry } from "./types";

// ONE graph-wide page list, fetched once per graph generation and shared by every
// namespace + sidebar consumer. `graphEpoch` bumps only on graph load/switch,
// rename, and config-format change — NOT per edit and NOT per navigation — so this
// is a per-graph fetch, not a per-anything-else fetch.
//
// Before this, `NamespaceHierarchy` keyed its `listPages()` resource on the page
// NAME, so it re-pulled the whole page list over IPC (+ an O(allPages) scan) on
// EVERY page navigation, even for a page with no namespace; and the sidebar, the
// namespace tree, and the {{namespace}} macro each ran their own duplicate
// whole-graph fetch. On a large graph that was hundreds of ms + GC churn per nav.
//
// Own root: lives for the app's lifetime by design (mirrors blockRefCounts.ts /
// resolveBatch.ts). `+ 1` so the initial epoch (0) is still truthy → fetch on
// first paint; the value still changes whenever graphEpoch does → refetch (which
// also fixes the "graph still loading at mount" race the sidebar guarded against).
const pagesRes = createRoot(() => {
  const [pages] = createResource(
    () => graphEpoch() + 1,
    () => backend().listPages().catch(() => [] as PageEntry[])
  );
  return pages;
});

/** All pages in the current graph (`undefined` until the first load). Reactive:
 *  reading it in a tracking scope re-runs when the list (re)loads. */
export function allPages(): PageEntry[] | undefined {
  return pagesRes();
}

/** All page names in the current graph (`[]` until the first load). Reactive. */
export function allPageNames(): string[] {
  return pagesRes()?.map((p) => p.name) ?? [];
}
