export interface BackgroundFlushDeps {
  endEdit(): void;
  flushAll(): Promise<boolean>;
  closeInFlight(): boolean;
  isHidden?(): boolean;
  addEventListener?: typeof document.addEventListener;
  removeEventListener?: typeof document.removeEventListener;
}

const triggers = ["visibilitychange", "pagehide", "freeze"] as const;

/** On visibilitychange, pagehide, or freeze, end the edit and start one flushAll
 * when hidden and no flush or close is in flight. Cost O(dirty pages + pending asset
 * writes). Startup errors and rejections are logged and swallowed; a resolved false
 * is ignored, so dirty/conflicted work can remain unsaved. Listener setup can throw.
 * Dispose removes all three listeners. This cannot keep a native WebView alive until
 * the flush settles. */
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
