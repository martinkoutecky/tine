import { graphOwner, latestOwner, readOwned } from "./owned";
import { reportUiFailure } from "./uiFailure";
import { createEffect, createRoot, createSignal } from "solid-js";
import { backend, type GraphAnswersChange } from "./backend";
import { graphEpoch } from "./graphSession";
import { waitForWarmCache } from "./warmCache";
import { blockExternalId } from "./document";
import { observeGraphAnswers } from "./graphAnswers";
import { installReferenceChangeCounter } from "./toasts";

let applyCounts: (change: GraphAnswersChange) => void;
let captureCounts: (ids: readonly string[]) => number | Promise<number | null | undefined>;

// One initial count map per graph; native publication deltas update only their
// targets. A pulse notifies badges without copying the graph-sized map (I-25).
const countsMap = createRoot(() => {
  let counts = new Map<string, number>();
  const updates = new Map<string, { rev: bigint; count: number }>();
  const [changed, setChanged] = createSignal(0);
  const publish = () => setChanged((n) => n + 1);
  applyCounts = (change) => {
    if (heldEpoch !== graphEpoch()) { heldEpoch = graphEpoch(); counts.clear(); updates.clear(); ready = false; }
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
  let ready = false;
  type InitialCounts = { kind: "ready"; counts: Record<string, number> } | { kind: "unavailable" };
  let initial: { epoch: number; result: Promise<InitialCounts> } | null = null;
  const loadCounts = (epoch: number) => {
    if (initial?.epoch === epoch) return initial.result;
    const owner = latestOwner(scope, "counts", graphOwner(() => epoch === graphEpoch()));
    const result = (async (): Promise<InitialCounts> => {
      try {
        const result = await readOwned(owner, backend().getBlockRefCounts());
        if (result.kind !== "current") return { kind: "unavailable" };
        counts = new Map(Object.entries(result.value));
        for (const [id, update] of updates) counts.set(id, update.count);
        ready = true;
        publish();
        // Only in-flight readers retain the response dictionary; the live map
        // remains the sole retained graph-sized count cache.
        if (initial?.epoch === epoch) initial = null;
        return { kind: "ready", counts: result.value };
      } catch (error) {
        if (owner()) reportUiFailure("block-counts", error);
        return { kind: "unavailable" };
      }
    })();
    initial = { epoch, result };
    return result;
  };
  captureCounts = (ids) => {
    const epoch = graphEpoch();
    const sum = (map: ReadonlyMap<string, number>) => ids.reduce((n, id) => n + (map.get(id) ?? 0), 0);
    if (heldEpoch === epoch && (ready || ids.every((id) => counts.has(id)))) return sum(counts);
    // Reuse the ONE initial map read, including when an edit gets there before
    // the startup warm-cache waiter. Never turn a missing cold entry into zero.
    const known = new Map(ids.filter((id) => counts.has(id)).map((id) => [id, counts.get(id)!]));
    return loadCounts(epoch).then((result) => result.kind === "unavailable"
      ? sum(known) > 0 ? null : undefined
      : ids.reduce((n, id) => n + (known.get(id) ?? result.counts[id] ?? 0), 0));
  };
  createEffect(() => {
    const epoch = graphEpoch();
    if (epoch !== heldEpoch) { heldEpoch = epoch; counts.clear(); updates.clear(); ready = false; publish(); }
    void (async () => {
      try {
        try {
          if (!(await waitForWarmCache(epoch)) || epoch !== graphEpoch()) return;
          if (ready && heldEpoch === epoch) return;
          await loadCounts(epoch);
        } catch (error) {
          if (epoch === graphEpoch()) reportUiFailure("block-counts", error);
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

/** Capture counts by external identity BEFORE an edit removes/transfers it.
 * Warm reads are synchronous; cold reads share the initial graph count request.
 * null means a positive count is known but the total is unavailable; undefined
 * means no positive evidence is available. Neither is a fabricated zero. */
export function captureBlockReferenceCount(ids: readonly string[]): number | Promise<number | null | undefined> {
  if (!ids.length) return 0;
  return captureCounts([...new Set(ids)]);
}

installReferenceChangeCounter(captureBlockReferenceCount);
