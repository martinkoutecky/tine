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

/** Installs exactly one listener owned by Tine's native SafeBackPlugin. Until
 * setup resolves, after setup rejection, and after cleanup, that native owner
 * consumes Back rather than falling through to WebView history/activity exit. */
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
  | "native_prepare_refused"
  | "native_prepare_partial"
  | "native_prepare_uncertain"
  | "exit_requested"
  | "exit_failed";

/** The only response that can authorize Android's final activity exit is
 * `safe`. A `partial` result records concrete progress across graph slots; a
 * transport failure is deliberately treated as equally uncertain. */
export type AndroidNativePrepareResult =
  | { status: "safe" }
  | { status: "refused"; detail: string }
  | { status: "partial"; safe_slots: string[]; detail: string };

export type AndroidNativePrepareFailure = Exclude<AndroidNativePrepareResult, { status: "safe" }>
  | { status: "transport_unknown"; detail: string };

function isAndroidNativePrepareResult(value: unknown): value is AndroidNativePrepareResult {
  if (value === null || typeof value !== "object") return false;
  const result = value as Partial<AndroidNativePrepareResult>;
  if (result.status === "safe") return true;
  if (result.status === "refused") return typeof result.detail === "string";
  return result.status === "partial"
    && typeof result.detail === "string"
    && Array.isArray(result.safe_slots)
    && result.safe_slots.every((slot) => typeof slot === "string");
}

/** The actor remains active through frontend preparation, then becomes
 * deliberately non-editable once native progress is partial/uncertain. */
export enum AndroidRootClosePhase {
  Idle = "Idle",
  PreparingFrontend = "PreparingFrontend",
  PreparingNative = "PreparingNative",
  NativePartiallyPrepared = "NativePartiallyPrepared",
  NativePreparedAwaitingFinish = "NativePreparedAwaitingFinish",
}

interface AndroidRootCloseState {
  phase: AndroidRootClosePhase;
}

export interface AndroidRootCloseCoordinator {
  request(): Promise<AndroidRootCloseResult>;
  phase(): AndroidRootClosePhase;
}

/** Android's managed root-close path has two native operations with distinct
 * retry semantics. A typed zero-progress refusal may re-arm the editor;
 * partial/unknown native progress can only retry native preparation. */
export async function requestAndroidRootClose(
  safeClose: SafeCloseCoordinator,
  state: AndroidRootCloseState,
  prepareNativeClose: () => Promise<AndroidNativePrepareResult>,
  finishActivity: () => Promise<void>,
  nativePrepareFailed: (failure: AndroidNativePrepareFailure) => void,
  finishActivityFailed: () => void,
): Promise<AndroidRootCloseResult> {
  if (state.phase === AndroidRootClosePhase.NativePreparedAwaitingFinish) {
    return requestAndroidActivityExit(finishActivity, finishActivityFailed);
  }
  if (state.phase === AndroidRootClosePhase.NativePartiallyPrepared) {
    state.phase = AndroidRootClosePhase.PreparingNative;
    // A previous Partial or transport failure may already have stopped one
    // managed slot. Even a later typed refusal cannot unmake that progress.
    return requestAndroidNativePreparation(
      safeClose,
      state,
      prepareNativeClose,
      finishActivity,
      nativePrepareFailed,
      finishActivityFailed,
      true,
    );
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
  return requestAndroidNativePreparation(
    safeClose,
    state,
    prepareNativeClose,
    finishActivity,
    nativePrepareFailed,
    finishActivityFailed,
    false,
  );
}

async function requestAndroidActivityExit(
  finishActivity: () => Promise<void>,
  finishActivityFailed: () => void,
): Promise<"exit_requested" | "exit_failed"> {
  try {
    await finishActivity();
    return "exit_requested";
  } catch {
    // Native shutdown is already durable. Keep the transition shield in place:
    // a later Back must only retry this activity-exit handoff.
    finishActivityFailed();
    return "exit_failed";
  }
}

async function requestAndroidNativePreparation(
  safeClose: SafeCloseCoordinator,
  state: AndroidRootCloseState,
  prepareNativeClose: () => Promise<AndroidNativePrepareResult>,
  finishActivity: () => Promise<void>,
  nativePrepareFailed: (failure: AndroidNativePrepareFailure) => void,
  finishActivityFailed: () => void,
  mayAlreadyHaveNativeProgress: boolean,
): Promise<AndroidRootCloseResult> {
  let preparation: AndroidNativePrepareResult;
  try {
    const result: unknown = await prepareNativeClose();
    if (!isAndroidNativePrepareResult(result)) {
      throw new Error("native shutdown returned an unrecognized result");
    }
    preparation = result;
  } catch (error) {
    // The command might have completed on the native side after transport was
    // interrupted. Preserve the frontend shield and do not replay its flush.
    state.phase = AndroidRootClosePhase.NativePartiallyPrepared;
    nativePrepareFailed({ status: "transport_unknown", detail: String(error) });
    return "native_prepare_uncertain";
  }

  if (preparation.status === "safe") {
    state.phase = AndroidRootClosePhase.NativePreparedAwaitingFinish;
    return requestAndroidActivityExit(finishActivity, finishActivityFailed);
  }
  if (preparation.status === "refused" && !mayAlreadyHaveNativeProgress) {
    // A typed zero-progress refusal proves that no managed slot crossed the
    // native-safe boundary. It is therefore safe to reopen the editor and let
    // the next Back perform the complete frontend + native transaction.
    safeClose.reset();
    state.phase = AndroidRootClosePhase.Idle;
    nativePrepareFailed(preparation);
    return "native_prepare_refused";
  }

  // Partial is explicit native progress. A refusal after an earlier Partial or
  // unknown transport cannot prove zero historical progress either, so both
  // remain non-editable and retry only native preparation.
  state.phase = AndroidRootClosePhase.NativePartiallyPrepared;
  nativePrepareFailed(preparation);
  return "native_prepare_partial";
}

export function createAndroidRootCloseCoordinator(
  safeClose: SafeCloseCoordinator,
  {
    prepareNativeClose,
    finishActivity,
    nativePrepareFailed,
    finishActivityFailed,
  }: {
    prepareNativeClose: () => Promise<AndroidNativePrepareResult>;
    finishActivity: () => Promise<void>;
    nativePrepareFailed: (failure: AndroidNativePrepareFailure) => void;
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
