// The write gate is separate from the async intent so every edit can consult it
// without pulling backend, page-host, or router dependencies into its module.
import { beginResultsChange } from "../queryResultsEpoch";
let frozen = false;
let thawed: () => void = () => {};

/** Deferred reloads and waiters install their wakeup here; every owner that
 * releases the freeze (rename, graph open, backup restore, root recovery) runs it. */
export function onGraphRewriteThaw(run: () => void): void { thawed = run; }

export function graphRewriteFrozen(): boolean {
  return frozen;
}

export function tryFreezeGraphRewrite(): (() => void) | null {
  if (frozen) return null;
  frozen = true;
  const finish = beginResultsChange();
  return () => { frozen = false; finish(); thawed(); };
}
