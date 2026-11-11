/** Native publication answers, consumed identically by saves and notifications.
 * The caller owns the graph binding. No content metadata is derived here. */
import type { GraphAnswersChange } from "./backend";
import { graphEpoch, bumpPageAliasRev, bumpPageInventoryRev } from "./graphSession";
import { patchPageIndex } from "./pageIndex";
import { applyChangeSummary } from "./changeSummary";
let epoch = -1, inventoryRev = -1n;
// App-lifetime subscribers: the count badges and lazy All Pages metrics.
let countObserver: ((change: GraphAnswersChange) => void) | undefined;
let metricObserver: ((change: GraphAnswersChange) => void) | undefined;
/** The optional count-badge cache subscribes when loaded. Unloaded consumers
 * need no deltas: their first graph-wide read supplies the current answer. */
export function observeGraphAnswers(observer: (change: GraphAnswersChange) => void): void {
  countObserver = observer;
}
/** The lazy All Pages owner occupies one app-lifetime observer slot. */
export function observePageMetricsAnswers(observer: (change: GraphAnswersChange) => void): void {
  metricObserver = observer;
}
export function applyGraphAnswers(change: GraphAnswersChange | null | undefined): void {
  if (!change) return;
  applyChangeSummary(change.summary);
  if (epoch !== graphEpoch()) { epoch = graphEpoch(); inventoryRev = -1n; }
  countObserver?.(change);
  metricObserver?.(change);
  const rev = BigInt(change.rev);
  if (change.inventoryChanged && rev > inventoryRev) {
    inventoryRev = rev;
    // A name-only change patches the held inventory (no IPC); anything the
    // page index cannot patch in order refetches, as before.
    const patched = change.inventoryDelta ? patchPageIndex(rev, change.inventoryDelta) : null;
    if (patched === null) bumpPageInventoryRev();
    else if (patched.aliases) bumpPageAliasRev();
  }
}
