import { createResource, createRoot } from "solid-js";
import { backend } from "./backend";
import { graphEpoch } from "./ui";

// One graph-wide `block uuid → referrer count` map, fetched once per graph
// generation and shared by every block's count badge (Block.tsx). Reading
// `blockRefCount(id)` inside a tracking scope subscribes to the map, so all badges
// update together when the graph changes (a new ref is saved → graphEpoch bumps →
// refetch). Created in its own root: it lives for the app's lifetime by design.
const countsMap = createRoot(() => {
  // `+ 1` so the initial epoch (0) is still truthy and the resource fetches on the
  // first paint; the value still changes whenever graphEpoch does → refetch.
  const [counts] = createResource(
    () => graphEpoch() + 1,
    () => backend().getBlockRefCounts().catch(() => ({}) as Record<string, number>)
  );
  return counts;
});

/** Number of blocks that reference block `id` in the current graph (0 if none /
 *  not yet loaded). Reactive: re-runs when the map (re)loads. */
export function blockRefCount(id: string): number {
  return countsMap()?.[id] ?? 0;
}
