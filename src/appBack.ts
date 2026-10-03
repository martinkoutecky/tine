/** The ONE Back ladder (GH #492, #501). Every Back gesture the app can receive,
 * whatever delivers it, runs through `dispatchAppBack`:
 *
 *   - Android hardware/gesture Back  -> src/androidBack.ts (native listener)
 *   - iOS left-edge swipe            -> src/edgeSwipe.ts (webview touch gesture)
 *
 * Neither caller knows the order, and nothing else may encode it: a second copy
 * of "transient, then drawer, then history" is how Android once popped a route
 * behind an open modal. src/appBack.guard.test.ts pins the single owner. */

export interface AppBackDeps {
  dismissTransient(): boolean;
  dismissDrawer(): boolean;
  restoreDrawerFocus(): void;
  /** Whether Tine actually went back. The WebView's own `canGoBack` cannot
   * answer this: the mobile router pushes same-URL entries, so its history
   * moves without the address or the entry count changing, and entries that
   * are not Tine's can sit in the same stack. Only the router knows. */
  historyBack(): boolean;
  /** Back with nothing left to pop: Android closes the app through the shared
   * safe-close transaction; iOS has no such rung and passes a no-op. */
  closeRoot(): void;
}

/** The read-only question the iOS edge swipe asks before it starts following
 * the finger: would Back do anything? Answered without performing it. */
export interface AppBackProbe {
  hasTransient(): boolean;
  hasDrawer(): boolean;
  canGoBack(): boolean;
}

export type AppBackDisposition = "transient" | "drawer" | "history" | "root";

/** Synchronous ordering matters: a Back gesture selects exactly one rung and
 * never synthesizes a KeyboardEvent or a second router back action. The
 * history rung is taken iff the router moved (master 07cb27262). */
export function dispatchAppBack(deps: AppBackDeps): AppBackDisposition {
  if (deps.dismissTransient()) return "transient";
  if (deps.dismissDrawer()) {
    deps.restoreDrawerFocus();
    return "drawer";
  }
  // `canGoBack` was true on a phone whose router had nothing to pop, so Back
  // landed on the history rung and silently did nothing, forever.
  if (deps.historyBack()) return "history";
  deps.closeRoot();
  return "root";
}

/** True when `dispatchAppBack` would take a rung above "root". */
export function appBackAvailable(probe: AppBackProbe): boolean {
  return probe.hasTransient() || probe.hasDrawer() || probe.canGoBack();
}
