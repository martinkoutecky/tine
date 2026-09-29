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
  /** Go back one step in Tine's own router and say whether it moved. The
   * WebView's `canGoBack` cannot answer this: its stack can hold entries that
   * are not Tine's, and the mobile router pushes same-URL entries. */
  historyBack(): boolean;
  closeRoot(): void;
}

export type AndroidBackDisposition = "transient" | "drawer" | "history" | "root";

/** Synchronous ordering matters: a hardware Back gesture selects exactly one
 * rung (transient, drawer, router history, root close) and never synthesizes a
 * KeyboardEvent or a second router back action. The payload is not consulted. */
export function dispatchAndroidBack(
  _payload: AndroidBackPayload,
  deps: AndroidBackDispatchDeps,
): AndroidBackDisposition {
  if (deps.dismissTransient()) return "transient";
  if (deps.dismissDrawer()) {
    deps.restoreDrawerFocus();
    return "drawer";
  }
  // The rung is chosen by whether the router moved, not by the WebView's
  // opinion of its own stack: `canGoBack` could be true with nothing for the
  // router to pop, so Back landed here and silently did nothing, forever.
  if (deps.historyBack()) return "history";
  deps.closeRoot();
  return "root";
}

export interface AndroidBackInstallDeps extends AndroidBackDispatchDeps {
  platform(): Promise<"android" | "ios" | "desktop">;
  subscribe(handler: (payload: AndroidBackPayload) => void): Promise<AndroidBackListener>;
  setupFailed?(error: unknown): void;
}

/** On Android, register one AppPlugin Back listener for this installation.
 * Dispatch dismisses a transient, then a drawer, then WebView history, then
 * requests root close. Other platforms install nothing. Setup failures call
 * setupFailed when supplied and do not reject through the returned cleanup
 * function. Cleanup unregisters an installed listener; dispatch is O(1). */
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

type AndroidProcessApi = { exit(code?: number): Promise<void> };

/** End the Android activity through the installed process plugin. Tauri's
 * `plugin:app` has no exit command, so invoking it failed and left a gray,
 * unusable screen after the root Back (GH #386). Call only after the
 * safe-close coordinator accepted; rejects when the plugin call fails. */
export async function exitAndroidActivity(
  loadProcess: () => Promise<AndroidProcessApi> = () => import("@tauri-apps/plugin-process"),
): Promise<void> {
  const { exit } = await loadProcess();
  await exit(0);
}

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
