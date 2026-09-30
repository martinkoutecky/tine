/** Native publication answers, consumed identically by saves and notifications.
 * The caller owns the graph binding. No content metadata is derived here. */
import type { GraphAnswersChange } from "./backend";
import { graphEpoch, bumpPageInventoryRev } from "./graphSession";
let epoch = -1, inventoryRev = -1n;
// One app-lifetime subscriber: the count-badge cache. This slot has a fixed bound.
let countObserver: ((change: GraphAnswersChange) => void) | undefined;
/** The optional count-badge cache subscribes when loaded. Unloaded consumers
 * need no deltas: their first graph-wide read supplies the current answer. */
export function observeGraphAnswers(observer: (change: GraphAnswersChange) => void): void {
  countObserver = observer;
}
export function applyGraphAnswers(change: GraphAnswersChange | null | undefined): void {
  if (!change) return;
  if (epoch !== graphEpoch()) { epoch = graphEpoch(); inventoryRev = -1n; }
  countObserver?.(change);
  const rev = BigInt(change.rev);
  if (change.inventoryChanged && rev > inventoryRev) {
    inventoryRev = rev;
    bumpPageInventoryRev();
  }
}
