import type { SafeCloseCoordinator, SafeClosePrepareResult } from "./safeClose";

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

  void deps.platform()
    .then(async (platform) => {
      if (platform !== "android" || disposed) return null;
      return deps.subscribe((payload) => { dispatchAndroidBack(payload, deps); });
    })
    .then((installed) => {
      if (!installed) return;
      if (disposed) void installed.unregister();
      else listener = installed;
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
  | "native_prepare_failed"
  | "exit_requested"
  | "exit_failed";

/** The actor remains active through preparation, then becomes deliberately
 * non-editable once native clean shutdown has succeeded.  Activity exit is the
 * only operation that may retry from NativePreparedAwaitingFinish. */
export enum AndroidRootClosePhase {
  Idle = "Idle",
  PreparingFrontend = "PreparingFrontend",
  PreparingNative = "PreparingNative",
  NativePreparedAwaitingFinish = "NativePreparedAwaitingFinish",
}

interface AndroidRootCloseState {
  phase: AndroidRootClosePhase;
}

export interface AndroidRootCloseCoordinator {
  request(): Promise<AndroidRootCloseResult>;
  phase(): AndroidRootClosePhase;
}

/** Android's managed root-close path has two native operations with different
 * retry semantics: preparation is allowed to re-arm the editor on refusal;
 * once it succeeds, only the existing AppPlugin activity exit may retry. */
export async function requestAndroidRootClose(
  safeClose: SafeCloseCoordinator,
  state: AndroidRootCloseState,
  prepareNativeClose: () => Promise<void>,
  finishActivity: () => Promise<void>,
  nativePrepareFailed: () => void,
  finishActivityFailed: () => void,
): Promise<AndroidRootCloseResult> {
  if (state.phase === AndroidRootClosePhase.NativePreparedAwaitingFinish) {
    try {
      await finishActivity();
      return "exit_requested";
    } catch {
      // Native shutdown is already durable. Keep the transition shield in
      // place: a later Back must only retry this AppPlugin handoff.
      finishActivityFailed();
      return "exit_failed";
    }
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

  state.phase = AndroidRootClosePhase.PreparingNative;
  try {
    await prepareNativeClose();
  } catch {
    // We have not reached the safe native point, so this is still an ordinary
    // refusal: release the shield and let a later Back run the full sequence.
    safeClose.reset();
    state.phase = AndroidRootClosePhase.Idle;
    nativePrepareFailed();
    return "native_prepare_failed";
  }

  state.phase = AndroidRootClosePhase.NativePreparedAwaitingFinish;
  try {
    await finishActivity();
    return "exit_requested";
  } catch {
    // Do not reset after native preparation. Replaying the frontend flush or
    // clean shutdown can race the already-prepared actor; only exit retries.
    finishActivityFailed();
    return "exit_failed";
  }
}

export function createAndroidRootCloseCoordinator(
  safeClose: SafeCloseCoordinator,
  {
    prepareNativeClose,
    finishActivity,
    nativePrepareFailed,
    finishActivityFailed,
  }: {
    prepareNativeClose: () => Promise<void>;
    finishActivity: () => Promise<void>;
    nativePrepareFailed: () => void;
    finishActivityFailed: () => void;
  },
): AndroidRootCloseCoordinator {
  const state: AndroidRootCloseState = { phase: AndroidRootClosePhase.Idle };
  return {
    request: () => requestAndroidRootClose(
      safeClose,
      state,
      prepareNativeClose,
      finishActivity,
      nativePrepareFailed,
      finishActivityFailed,
    ),
    phase: () => state.phase,
  };
}
