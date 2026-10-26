// The write gate is separate from the async intent so every edit can consult it
// without pulling backend, page-host, or router dependencies into its module.
import { beginResultsChange } from "../queryResultsEpoch";
let frozen = false;

export function graphRewriteFrozen(): boolean {
  return frozen;
}

export function tryFreezeGraphRewrite(): (() => void) | null {
  if (frozen) return null;
  frozen = true;
  const finish = beginResultsChange();
  return () => { frozen = false; finish(); };
}
