/** Start the ordinary guarded save when a graph WebView may be reclaimed.
 * Costs O(dirty pages); failures remain in the save engine for normal retry.
 * This listener cannot hold a native WebView open while the promise settles. */
export interface BackgroundFlushDeps {
  endEdit(): void;
  flushAll(): Promise<boolean>;
  closeInFlight(): boolean;
  isHidden?(): boolean;
  addEventListener?: typeof document.addEventListener;
  removeEventListener?: typeof document.removeEventListener;
}

const triggers = ["visibilitychange", "pagehide", "freeze"] as const;

export function installBackgroundFlush(deps: BackgroundFlushDeps): () => void {
  const add = deps.addEventListener ?? document.addEventListener.bind(document);
  const remove = deps.removeEventListener ?? document.removeEventListener.bind(document);
  const isHidden = deps.isHidden ?? (() => document.visibilityState !== "visible");
  let inFlight = false;
  const flush = () => {
    if (!isHidden() || inFlight || deps.closeInFlight()) return;
    inFlight = true;
    try {
      deps.endEdit();
      void deps.flushAll().catch(() => {
        console.error("Background flush failed");
      }).finally(() => { inFlight = false; });
    } catch {
      console.error("Background flush could not start");
      inFlight = false;
    }
  };
  for (const trigger of triggers) add(trigger, flush);
  return () => { for (const trigger of triggers) remove(trigger, flush); };
}
