import { externalActivityHeld } from "./externalActivity";
import { anyWindowVisible, mainWindow, onEachWindow } from "./windowRealm";

export interface BackgroundFlushDeps {
  endEdit(): void;
  flushAll(): Promise<boolean>;
  closeInFlight(): boolean;
  isHidden?(): boolean;
  externalActivityHeld?(): boolean;
  addEventListener?: typeof document.addEventListener;
  removeEventListener?: typeof document.removeEventListener;
}

const triggers = ["visibilitychange", "pagehide", "freeze"] as const;

/** On visibilitychange, pagehide, or freeze, end the edit and start one flushAll
 * when hidden and no flush or close is in flight. While a native picker/camera
 * holds external activity (GH #622) the hide is part of the edit: flush, but keep
 * the edit open for the insert. Cost O(dirty pages + pending asset
 * writes). Startup errors and rejections are logged and swallowed; a resolved false
 * is ignored, so dirty/conflicted work can remain unsaved. Listener setup can throw.
 * Dispose removes all three listeners. This cannot keep a native WebView alive until
 * the flush settles.
 *
 * Workspace windows (OG-MULTIWINDOW P5): "hidden" means NO Tine window is
 * visible, so minimizing main while the user types in a workspace window keeps
 * the edit open. The triggers are listened for on every window's document,
 * and `pagehide` (which fires at the window, not the document) on the window.
 * Main's own `pagehide` is a reload or navigation that takes every window's
 * rendering with it, so it flushes even while a workspace window is still
 * visible (review F2). */
export function installBackgroundFlush(deps: BackgroundFlushDeps): () => void {
  const isHidden = deps.isHidden ?? (() => !anyWindowVisible());
  const pickerHeld = deps.externalActivityHeld ?? externalActivityHeld;
  let inFlight = false;
  const flush = (force = false) => {
    if ((!force && !isHidden()) || inFlight || deps.closeInFlight()) return;
    inFlight = true;
    try {
      if (!pickerHeld()) deps.endEdit();
      void deps.flushAll().catch(() => {
        console.error("Background flush failed");
      }).finally(() => { inFlight = false; });
    } catch {
      console.error("Background flush could not start");
      inFlight = false;
    }
  };
  if (deps.addEventListener && deps.removeEventListener) {
    const { addEventListener: add, removeEventListener: remove } = deps;
    const onTrigger = () => flush();
    for (const trigger of triggers) add(trigger, onTrigger);
    return () => { for (const trigger of triggers) remove(trigger, onTrigger); };
  }
  return onEachWindow((win) => {
    const doc = win.document;
    const onTrigger = () => flush();
    const onPageHide = () => flush(win === mainWindow);
    for (const trigger of triggers) doc.addEventListener(trigger, onTrigger);
    win.addEventListener("pagehide", onPageHide);
    return () => {
      for (const trigger of triggers) doc.removeEventListener(trigger, onTrigger);
      win.removeEventListener("pagehide", onPageHide);
    };
  });
}
