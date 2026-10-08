// The write gate is separate from the async intent so every edit can consult it
// without pulling backend, save engine, or router dependencies into its module.
let frozen = false;

export function graphRewriteFrozen(): boolean {
  return frozen;
}

export function tryFreezeGraphRewrite(): (() => void) | null {
  if (frozen) return null;
  frozen = true;
  return () => { frozen = false; };
}
