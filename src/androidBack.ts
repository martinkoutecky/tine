import type { SafeCloseCoordinator, SafeClosePrepareResult } from "./safeClose";
import { ownedWhen, readOwned, readOwnedResource } from "./owned";

export interface AndroidBackPayload {
  canGoBack: boolean;
}

export interface AndroidBackListener {
  unregister(): Promise<void> | void;
}

export interface AndroidBackDispatchDeps {
  dismissTransient(): boolean;
  dismissDrawer(): boolean;
  restoreDrawerFocus(): void;
  historyBack(): void;
  closeRoot(): void;
}

export type AndroidBackDisposition = "transient" | "drawer" | "history" | "root";

/** Synchronous ordering matters: a hardware Back gesture selects exactly one
 * rung and never synthesizes a KeyboardEvent or a second router back action. */
export function dispatchAndroidBack(
  payload: AndroidBackPayload,
  deps: AndroidBackDispatchDeps,
): AndroidBackDisposition {
  if (deps.dismissTransient()) return "transient";
  if (deps.dismissDrawer()) {
    deps.restoreDrawerFocus();
    return "drawer";
  }
  if (payload.canGoBack) {
    deps.historyBack();
    return "history";
  }
  deps.closeRoot();
  return "root";
}

export interface AndroidBackInstallDeps extends AndroidBackDispatchDeps {
  platform(): Promise<"android" | "ios" | "desktop">;
  subscribe(handler: (payload: AndroidBackPayload) => void): Promise<AndroidBackListener>;
  setupFailed?(error: unknown): void;
}

/** Installs exactly one official AppPlugin listener on Android.  Until setup
 * resolves, after setup rejection, and after cleanup, no JS listener exists and
 * Tauri's AppPlugin retains its native WebView/activity fallback. */
export function installAndroidBackHandler(deps: AndroidBackInstallDeps): () => void {
  let disposed = false;
  let listener: AndroidBackListener | null = null;
  const owner = ownedWhen(() => !disposed);

  void readOwned(owner, deps.platform())
    .then(async (platform) => {
      if (platform.kind === "stale" || platform.value !== "android") return;
      const installed = await readOwnedResource(owner,
        deps.subscribe((payload) => { dispatchAndroidBack(payload, deps); }),
        (handle) => handle.unregister());
      if (installed.kind === "current") listener = installed.value;
    })
    .catch((error) => deps.setupFailed?.(error));

  return () => {
    if (disposed) return;
    disposed = true;
    const installed = listener;
    listener = null;
    if (installed) void installed.unregister();
  };
}

export type AndroidRootCloseResult = SafeClosePrepareResult | "exit_requested" | "exit_failed";

/** Root close shares the desktop coordinator.  A failed native invoke resets
 * the accepted transaction so the next hardware Back can safely retry. */
export async function requestAndroidRootClose(
  safeClose: SafeCloseCoordinator,
  exit: () => Promise<void>,
  exitFailed: () => void,
): Promise<AndroidRootCloseResult> {
  const prepared = await safeClose.prepare();
  if (prepared !== "accepted") return prepared;
  try {
    await exit();
    return "exit_requested";
  } catch {
    safeClose.reset();
    exitFailed();
    return "exit_failed";
  }
}
