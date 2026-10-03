import type { SafeCloseCoordinator, SafeClosePrepareResult } from "./safeClose";
import { dispatchAppBack, type AppBackDeps, type AppBackDisposition } from "./appBack";
import { ownedWhen, readOwned, readOwnedResource } from "./owned";

export interface AndroidBackPayload {
  canGoBack: boolean;
}

export interface AndroidBackListener {
  unregister(): Promise<void> | void;
}

type AndroidProcessApi = { exit(code?: number): Promise<void> };

/** Exit only after the safe-close coordinator has made graph state durable.
 * Tauri exposes Activity/process exit through plugin-process (capability
 * `process:allow-exit`); plugin:app has no exit command on the Rust side, so
 * `invoke("plugin:app|exit")` never closed the app (master cb7a10fd3). */
export async function exitAndroidActivity(
  loadProcess: () => Promise<AndroidProcessApi> = () => import("@tauri-apps/plugin-process"),
): Promise<void> {
  const { exit } = await loadProcess();
  await exit(0);
}

/** Kept as names for the Android-facing seam; the ladder itself lives in
 * src/appBack.ts and is shared with the iOS edge swipe. */
export type AndroidBackDispatchDeps = AppBackDeps;
export type AndroidBackDisposition = AppBackDisposition;

/** The native `canGoBack` payload is not consulted (master 07cb27262). */
export function dispatchAndroidBack(
  _payload: AndroidBackPayload,
  deps: AndroidBackDispatchDeps,
): AndroidBackDisposition {
  return dispatchAppBack(deps);
}

export interface AndroidBackInstallDeps extends AndroidBackDispatchDeps {
  platform(): Promise<"android" | "ios" | "desktop">;
  subscribe(handler: (payload: AndroidBackPayload) => void): Promise<AndroidBackListener>;
  setupFailed?(error: unknown): void;
}

/** On Android, register one SafeBack listener (the native owner's event) for this installation.
 * Dispatch dismisses a transient, then a drawer, then router history, then
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
