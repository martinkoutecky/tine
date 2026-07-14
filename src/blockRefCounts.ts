import { createResource, createRoot } from "solid-js";
import { backend } from "./backend";
import { dataRev, graphEpoch } from "./ui";
import { waitForWarmCache } from "./warmCache";

// One graph-wide `block uuid → referrer count` map, fetched once per graph and
// after each landed save, and shared by every block's count badge (Block.tsx). Reading
// `blockRefCount(id)` inside a tracking scope subscribes to the map, so all badges
// update together when the graph changes (a new ref is saved → graphEpoch bumps →
// refetch). Created in its own root: it lives for the app's lifetime by design.
const countsMap = createRoot(() => {
  const [counts] = createResource(
    () => ({ epoch: graphEpoch(), revision: dataRev() }),
    async ({ epoch }) => {
      if (!(await waitForWarmCache(epoch))) return {};
      if (epoch !== graphEpoch()) return {};
      return backend().getBlockRefCounts().catch(() => ({}) as Record<string, number>);
    }
  );
  return counts;
});

/** Number of blocks that reference block `id` in the current graph (0 if none /
 *  not yet loaded). Reactive: re-runs when the map (re)loads. */
export function blockRefCount(id: string): number {
  return countsMap()?.[id] ?? 0;
}
