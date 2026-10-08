import { graphOwner, latestOwner, readOwned } from "./owned";
import { reportUiFailure } from "./uiFailure";
import { createEffect, createRoot, createSignal } from "solid-js";
import { backend, type GraphAnswersChange } from "./backend";
import { graphEpoch } from "./graphSession";
import { waitForWarmCache } from "./warmCache";
import { blockExternalId } from "./document";
import { observeGraphAnswers } from "./graphAnswers";

let applyCounts: (change: GraphAnswersChange) => void;

// One initial count map per graph; native publication deltas update only their
// targets. A pulse notifies badges without copying the graph-sized map (I-25).
const countsMap = createRoot(() => {
  let counts = new Map<string, number>();
  const updates = new Map<string, { rev: bigint; count: number }>();
  const [changed, setChanged] = createSignal(0);
  const publish = () => setChanged((n) => n + 1);
  applyCounts = (change) => {
    if (heldEpoch !== graphEpoch()) { heldEpoch = graphEpoch(); counts.clear(); updates.clear(); }
    const rev = BigInt(change.rev);
    for (const [id, count] of Object.entries(change.blockRefCounts)) {
      if ((updates.get(id)?.rev ?? -1n) >= rev) continue;
      updates.set(id, { rev, count });
      counts.set(id, count);
    }
    if (Object.keys(change.blockRefCounts).length) publish();
  };
  const scope = {};
  let heldEpoch = graphEpoch();
  createEffect(() => {
    const epoch = graphEpoch();
    if (epoch !== heldEpoch) { heldEpoch = epoch; counts.clear(); updates.clear(); publish(); }
    void (async () => {
      try {
        const owner = latestOwner(scope, "counts", graphOwner(() => epoch === graphEpoch()));
        try {
          if (!(await waitForWarmCache(epoch)) || !owner()) return;
          const result = await readOwned(owner, backend().getBlockRefCounts());
          if (result.kind === "current") {
            counts = new Map(Object.entries(result.value));
            // A late initial fetch cannot overwrite a save/watcher delta.
            for (const [id, update] of updates) counts.set(id, update.count);
            publish();
          }
        } catch (error) {
          if (owner()) reportUiFailure("block-counts", error);
        }
      } catch (error) {
        if (epoch === graphEpoch()) reportUiFailure("block-counts", error);
      }
    })();
  });
  return () => { changed(); return counts; };
});

observeGraphAnswers((change) => applyCounts(change));

/** Number of blocks that reference block `id` in the current graph (0 if none /
 *  not yet loaded). Reactive: re-runs when the map (re)loads. */
export function blockRefCount(id: string): number {
  const externalId = blockExternalId(id) ?? id;
  return countsMap().get(externalId) ?? 0;
}
