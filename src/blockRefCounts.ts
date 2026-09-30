import { graphOwner, latestOwner, readOwned } from "./owned";
import { reportUiFailure } from "./uiFailure";
import { createEffect, createRoot, createSignal } from "solid-js";
import { backend } from "./backend";
import { dataRev, graphEpoch } from "./graphSession";
import { waitForWarmCache } from "./warmCache";
import { blockExternalId } from "./document";

// One graph-wide `block uuid → referrer count` map, fetched once per graph and
// after each landed save, and shared by every block's count badge (Block.tsx). Reading
// `blockRefCount(id)` inside a tracking scope subscribes to the map, so all badges
// update together when the graph changes (a new ref is saved → graphEpoch bumps →
// refetch). Created in its own root: it lives for the app's lifetime by design.
const countsMap = createRoot(() => {
  const [counts, setCounts] = createSignal<Record<string, number>>({});
  const scope = {};
  let heldEpoch = graphEpoch();
  createEffect(() => {
    const epoch = graphEpoch();
    dataRev();
    if (epoch !== heldEpoch) { heldEpoch = epoch; setCounts({}); }
    const owner = latestOwner(scope, "counts", graphOwner(() => epoch === graphEpoch()));
    void (async () => {
      try {
        if (!(await waitForWarmCache(epoch)) || !owner()) return;
        const result = await readOwned(owner, backend().getBlockRefCounts());
        if (result.kind === "current") setCounts(result.value);
      } catch (error) {
        if (owner()) reportUiFailure("block-counts", error);
      }
    })();
  });
  return counts;
});

/** Number of blocks that reference block `id` in the current graph (0 if none /
 *  not yet loaded). Reactive: re-runs when the map (re)loads. */
export function blockRefCount(id: string): number {
  const externalId = blockExternalId(id) ?? id;
  return countsMap()?.[externalId] ?? 0;
}
