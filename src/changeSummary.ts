/** Per-graph publication evidence for later keyed refresh slices. Receiving a
 * summary never bumps dataRev or starts a read. No views subscribe in S1. */
import { clearOnBindingInvalidated, graphScopedSignal } from "./binding";
import type { ChangeSummary } from "./backendTypes";
import { graphEpoch } from "./graphSession";

let epoch = -1;
let revision: bigint | undefined;
const [latest, setLatest] = graphScopedSignal<ChangeSummary>();
clearOnBindingInvalidated(() => { epoch = -1; revision = undefined; });

/** A new binding has no baseline. Its first publication is necessarily full;
 * revision gaps also require a full refetch, even for a precise host summary.
 * Duplicate/reordered delivery cannot move the revision backwards. */
export function applyChangeSummary(change: ChangeSummary | null | undefined): void {
  if (!change) return;
  if (epoch !== graphEpoch()) {
    epoch = graphEpoch();
    revision = undefined;
  }
  const rev = BigInt(change.rev);
  if (revision !== undefined && rev <= revision) return;
  const full = change.full || revision === undefined || rev !== revision + 1n;
  revision = rev;
  setLatest(full === change.full ? change : { ...change, full });
}

/** undefined means this binding has no summary baseline yet. New consumers
 * must fetch their initial answer; they cannot replay earlier publications. */
export function changeSummary(): ChangeSummary | undefined {
  return epoch === graphEpoch() ? latest() ?? undefined : undefined;
}
