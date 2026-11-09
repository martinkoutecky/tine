/** Per-graph publication evidence. Receiving stages keys; the existing refresh
 * gates release them after publication, independently of legacy dataRev readers. */
import { clearOnBindingInvalidated, graphScopedSignal } from "./binding";
import type { ChangeSummary } from "./backendTypes";
import { graphEpoch } from "./graphSession";
import { stageChangeKeys } from "./changeKeys";

let epoch = -1;
let revision: bigint | undefined;
const [latest, setLatest] = graphScopedSignal<ChangeSummary>();
clearOnBindingInvalidated(() => { epoch = -1; revision = undefined; });

/** Graph open seeds the baseline from the open answer (`published_rev`: the
 * last publication the host had sent this window), after the binding's epoch
 * is set and before its readers fetch, so they read at or after it. The first
 * summary then reads as the delta it is. Without a baseline (an older host, a
 * later epoch bump) the first summary is full. A summary already received in
 * this epoch wins: it is the newer evidence. */
export function setChangeSummaryBaseline(rev: string | null | undefined): void {
  if (rev == null || (epoch === graphEpoch() && revision !== undefined)) return;
  epoch = graphEpoch();
  revision = BigInt(rev);
}

/** A binding without a baseline reads its first publication as full; a
 * revision gap (a summary missed) also requires a full refetch, even for a
 * precise host summary. Duplicate/reordered delivery cannot move the
 * revision backwards. */
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
  const accepted = full === change.full ? change : { ...change, full };
  setLatest(accepted);
  stageChangeKeys(accepted);
}

/** undefined means this binding has no summary baseline yet. New consumers
 * must fetch their initial answer; they cannot replay earlier publications. */
export function changeSummary(): ChangeSummary | undefined {
  return epoch === graphEpoch() ? latest() ?? undefined : undefined;
}
