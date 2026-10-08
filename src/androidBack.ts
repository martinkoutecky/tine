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

export type AndroidRootCloseResult =
  | SafeClosePrepareResult
  | "exit_requested"
  | "exit_failed";

/** Once frontend preparation is accepted the graph is durable, and the only
 * remaining step is the activity exit. Direct Files has no native runtime to
 * drain before it (ADR 0066). */
export enum AndroidRootClosePhase {
  Idle = "Idle",
  PreparingFrontend = "PreparingFrontend",
  PreparedAwaitingExit = "PreparedAwaitingExit",
}

interface AndroidRootCloseState {
  phase: AndroidRootClosePhase;
}

export interface AndroidRootCloseCoordinator {
  request(): Promise<AndroidRootCloseResult>;
  phase(): AndroidRootClosePhase;
}

/** Android's root close: the shared safe-close transaction, then the activity
 * exit. A failed exit keeps the transition shield, and a later Back retries
 * only the exit, never the flush. */
export async function requestAndroidRootClose(
  safeClose: SafeCloseCoordinator,
  state: AndroidRootCloseState,
  finishActivity: () => Promise<void>,
  finishActivityFailed: () => void,
): Promise<AndroidRootCloseResult> {
  if (state.phase === AndroidRootClosePhase.PreparedAwaitingExit) {
    return requestAndroidActivityExit(finishActivity, finishActivityFailed);
  }
  if (state.phase !== AndroidRootClosePhase.Idle) return "in_flight";

  state.phase = AndroidRootClosePhase.PreparingFrontend;
  let prepared: SafeClosePrepareResult;
  try {
    prepared = await safeClose.prepare();
  } catch {
    // SafeClose itself releases its shield in its finally block. Keep this
    // coordinator retryable too if a frontend dependency throws unexpectedly.
    state.phase = AndroidRootClosePhase.Idle;
    return "rejected";
  }
  if (prepared !== "accepted") {
    state.phase = AndroidRootClosePhase.Idle;
    return prepared;
  }

  state.phase = AndroidRootClosePhase.PreparedAwaitingExit;
  return requestAndroidActivityExit(finishActivity, finishActivityFailed);
}

async function requestAndroidActivityExit(
  finishActivity: () => Promise<void>,
  finishActivityFailed: () => void,
): Promise<"exit_requested" | "exit_failed"> {
  try {
    await finishActivity();
    return "exit_requested";
  } catch {
    // The graph is already durable. Keep the transition shield in place: a
    // later Back must only retry this activity-exit handoff.
    finishActivityFailed();
    return "exit_failed";
  }
}

export function createAndroidRootCloseCoordinator(
  safeClose: SafeCloseCoordinator,
  {
    finishActivity,
    finishActivityFailed,
  }: {
    finishActivity: () => Promise<void>;
    finishActivityFailed: () => void;
  },
): AndroidRootCloseCoordinator {
  const state: AndroidRootCloseState = { phase: AndroidRootClosePhase.Idle };
  return {
    request: () => requestAndroidRootClose(safeClose, state, finishActivity, finishActivityFailed),
    phase: () => state.phase,
  };
}
