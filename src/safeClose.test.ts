import { describe, expect, it, vi } from "vitest";
import { AndroidRootClosePhase, createAndroidRootCloseCoordinator } from "./androidBack";
import { createSafeCloseCoordinator, type DiscardReason, type SafeCloseDeps } from "./safeClose";

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
}

function harness(overrides: Partial<SafeCloseDeps> = {}) {
  const transitions: boolean[] = [];
  const deps: SafeCloseDeps = {
    blurActive: vi.fn(),
    endEdit: vi.fn(),
    flushPdfWork: vi.fn(async () => true),
    flushAll: vi.fn(async () => true),
    confirmDiscard: vi.fn(async () => false),
    flushSession: vi.fn(async () => {}),
    setTransition: vi.fn((active) => transitions.push(active)),
    notifyPdfFailure: vi.fn(),
    notifyStillSaving: vi.fn(),
    notifyConfirmationFailure: vi.fn(),
    runBounded: async (operation) => operation,
    ...overrides,
  };
  return { deps, transitions, safeClose: createSafeCloseCoordinator(deps) };
}

function androidRootClose(
  safeClose: ReturnType<typeof createSafeCloseCoordinator>,
  finishActivity: () => Promise<void>,
  overrides: Partial<{
    prepareNativeClose: () => Promise<void>;
    nativePrepareFailed: () => void;
    finishActivityFailed: () => void;
  }> = {},
) {
  const deps = {
    prepareNativeClose: vi.fn(async () => {}),
    finishActivity,
    nativePrepareFailed: vi.fn(),
    finishActivityFailed: vi.fn(),
    ...overrides,
  };
  return { deps, rootClose: createAndroidRootCloseCoordinator(safeClose, deps) };
}

describe("GH #161 shared safe-close transaction", () => {
  it("flushes graph and session once before an accepted Android root exit", async () => {
    const order: string[] = [];
    const { deps, safeClose, transitions } = harness({
      flushAll: vi.fn(async () => {
        order.push("frontend");
        return true;
      }),
    });
    const exit = vi.fn(async () => { order.push("activity"); });
    const { deps: closeDeps, rootClose } = androidRootClose(safeClose, exit, {
      prepareNativeClose: vi.fn(async () => { order.push("native"); }),
    });

    await expect(rootClose.request()).resolves.toBe("exit_requested");
    expect(deps.blurActive).toHaveBeenCalledOnce();
    expect(deps.endEdit).toHaveBeenCalledOnce();
    expect(deps.flushPdfWork).toHaveBeenCalledOnce();
    expect(deps.flushAll).toHaveBeenCalledOnce();
    expect(deps.confirmDiscard).not.toHaveBeenCalled();
    expect(deps.flushSession).toHaveBeenCalledOnce();
    expect(closeDeps.prepareNativeClose).toHaveBeenCalledOnce();
    expect(exit).toHaveBeenCalledOnce();
    expect(order).toEqual(["frontend", "native", "activity"]);
    expect(transitions).toEqual([true]);
    expect(safeClose.inFlight()).toBe(true);
  });

  it("consumes repeated root Back while the first flush is in flight", async () => {
    const flush = deferred<boolean>();
    const { deps, safeClose } = harness({ flushAll: vi.fn(() => flush.promise) });
    const exit = vi.fn(async () => {});
    const { rootClose } = androidRootClose(safeClose, exit);

    const first = rootClose.request();
    await Promise.resolve();
    await expect(rootClose.request()).resolves.toBe("in_flight");
    expect(deps.flushAll).toHaveBeenCalledOnce();
    expect(exit).not.toHaveBeenCalled();

    flush.resolve(true);
    await expect(first).resolves.toBe("exit_requested");
    expect(exit).toHaveBeenCalledOnce();
  });

  it("keeps the app open and resets when a failed flush is not explicitly discarded", async () => {
    const { deps, safeClose, transitions } = harness({
      flushAll: vi.fn(async () => false),
      confirmDiscard: vi.fn(async () => false),
    });
    const exit = vi.fn(async () => {});
    const { rootClose } = androidRootClose(safeClose, exit);

    await expect(rootClose.request()).resolves.toBe("rejected");
    expect(deps.confirmDiscard).toHaveBeenCalledOnce();
    expect(deps.flushSession).not.toHaveBeenCalled();
    expect(exit).not.toHaveBeenCalled();
    expect(transitions).toEqual([true, false]);
    expect(safeClose.inFlight()).toBe(false);
  });

  it("enrolls pending PDF state before page flush and rejects a failed PDF drain", async () => {
    const order: string[] = [];
    const { deps, safeClose, transitions } = harness({
      flushPdfWork: vi.fn(async () => {
        order.push("pdf");
        return false;
      }),
      flushAll: vi.fn(async () => {
        order.push("pages");
        return true;
      }),
      confirmDiscard: vi.fn(async () => false),
    });

    await expect(safeClose.prepare()).resolves.toBe("rejected");

    expect(order).toEqual(["pdf"]);
    expect(deps.flushAll).not.toHaveBeenCalled();
    expect(deps.confirmDiscard).not.toHaveBeenCalled();
    expect(deps.notifyPdfFailure).toHaveBeenCalledOnce();
    expect(transitions).toEqual([true, false]);
  });

  it("continues only after explicit discard when the graph flush fails", async () => {
    const { deps, safeClose } = harness({
      flushAll: vi.fn(async () => false),
      confirmDiscard: vi.fn(async () => true),
    });
    const exit = vi.fn(async () => {});
    const { rootClose } = androidRootClose(safeClose, exit);

    await expect(rootClose.request()).resolves.toBe("exit_requested");
    expect(deps.confirmDiscard).toHaveBeenCalledExactlyOnceWith("failed");
    expect(deps.flushSession).toHaveBeenCalledOnce();
    expect(exit).toHaveBeenCalledOnce();
  });

  // Direct Files data-safety audit, finding 11. This case previously shared the
  // assertion above: a flush that had merely not finished within 4 s was
  // reported to the user in the same words as one that could never succeed, and
  // they were offered the same "close anyway and lose them". A slow or network
  // filesystem, a fsync behind a busy disk, or simply many dirty pages can
  // exceed that bound with nothing wrong — so the close now waits out a grace
  // period first, and only then asks, in different words.
  it("waits out a grace period before treating a slow flush as unsaved", async () => {
    const landsLate = deferred<boolean>();
    const waited: number[] = [];
    const runBounded: SafeCloseDeps["runBounded"] = async (operation, timeoutMs, fallback) => {
      if (operation !== landsLate.promise) return operation;
      waited.push(timeoutMs);
      // Only the soft bound expires; the grace period sees it through.
      if (timeoutMs === 4000) return fallback;
      return operation;
    };
    const { deps, safeClose } = harness({
      flushAll: vi.fn(() => landsLate.promise),
      confirmDiscard: vi.fn(async () => true),
      runBounded,
    });
    const exit = vi.fn(async () => {});
    const { rootClose } = androidRootClose(safeClose, exit);

    const closing = rootClose.request();
    await Promise.resolve();
    landsLate.resolve(true);

    await expect(closing).resolves.toBe("exit_requested");
    expect(deps.notifyStillSaving).toHaveBeenCalledOnce();
    expect(deps.confirmDiscard).not.toHaveBeenCalled();
    expect(waited.filter((ms) => ms > 4000)).not.toEqual([]);
    expect(deps.flushAll).toHaveBeenCalledOnce();
    expect(exit).toHaveBeenCalledOnce();
  });

  it("asks in different words when even the grace period runs out", async () => {
    const never = new Promise<boolean>(() => {});
    const runBounded: SafeCloseDeps["runBounded"] = async (operation, _timeoutMs, fallback) =>
      (operation === never ? fallback : operation);
    const reasons: DiscardReason[] = [];
    const { deps, safeClose } = harness({
      flushAll: vi.fn(() => never),
      confirmDiscard: vi.fn(async (reason) => { reasons.push(reason); return true; }),
      runBounded,
    });
    const exit = vi.fn(async () => {});
    const { rootClose } = androidRootClose(safeClose, exit);

    await expect(rootClose.request()).resolves.toBe("exit_requested");
    expect(deps.notifyStillSaving).toHaveBeenCalledOnce();
    expect(reasons).toEqual(["still-saving"]);
    expect(exit).toHaveBeenCalledOnce();
  });

  it("treats confirmation failure as rejection and leaves edits open", async () => {
    const { deps, safeClose, transitions } = harness({
      flushAll: vi.fn(async () => { throw new Error("save failed"); }),
      confirmDiscard: vi.fn(async () => { throw new Error("dialog failed"); }),
    });
    const exit = vi.fn(async () => {});
    const { rootClose } = androidRootClose(safeClose, exit);

    await expect(rootClose.request()).resolves.toBe("rejected");
    expect(deps.notifyConfirmationFailure).toHaveBeenCalledOnce();
    expect(exit).not.toHaveBeenCalled();
    expect(transitions).toEqual([true, false]);
  });

  it("keeps accepted graph-close policy when the best-effort session flush fails", async () => {
    const { safeClose } = harness({
      flushSession: vi.fn(async () => { throw new Error("session failed"); }),
    });
    const exit = vi.fn(async () => {});
    const { rootClose } = androidRootClose(safeClose, exit);

    await expect(rootClose.request()).resolves.toBe("exit_requested");
    expect(exit).toHaveBeenCalledOnce();
  });

  it("resets after native preparation refusal so a later Back retries the full close", async () => {
    const { deps, safeClose, transitions } = harness();
    const prepareNativeClose = vi.fn()
      .mockRejectedValueOnce(new Error("managed shutdown refused"))
      .mockResolvedValueOnce(undefined);
    const exit = vi.fn(async () => {});
    const nativePrepareFailed = vi.fn();
    const { rootClose } = androidRootClose(safeClose, exit, { prepareNativeClose, nativePrepareFailed });

    await expect(rootClose.request()).resolves.toBe("native_prepare_failed");
    expect(nativePrepareFailed).toHaveBeenCalledOnce();
    expect(safeClose.inFlight()).toBe(false);
    expect(rootClose.phase()).toBe(AndroidRootClosePhase.Idle);
    expect(exit).not.toHaveBeenCalled();
    await expect(rootClose.request()).resolves.toBe("exit_requested");
    expect(deps.flushAll).toHaveBeenCalledTimes(2);
    expect(prepareNativeClose).toHaveBeenCalledTimes(2);
    expect(exit).toHaveBeenCalledOnce();
    expect(transitions).toEqual([true, false, true]);
  });

  it("keeps the shield after activity exit rejection and retries only the exit", async () => {
    const { deps, safeClose, transitions } = harness();
    const prepareNativeClose = vi.fn(async () => {});
    const exit = vi.fn()
      .mockRejectedValueOnce(new Error("plugin unavailable"))
      .mockResolvedValueOnce(undefined);
    const finishActivityFailed = vi.fn();
    const { rootClose } = androidRootClose(safeClose, exit, { prepareNativeClose, finishActivityFailed });

    await expect(rootClose.request()).resolves.toBe("exit_failed");
    expect(finishActivityFailed).toHaveBeenCalledOnce();
    expect(rootClose.phase()).toBe(AndroidRootClosePhase.NativePreparedAwaitingFinish);
    expect(safeClose.inFlight()).toBe(true);
    expect(transitions).toEqual([true]);
    expect(deps.flushAll).toHaveBeenCalledOnce();
    expect(prepareNativeClose).toHaveBeenCalledOnce();

    await expect(rootClose.request()).resolves.toBe("exit_requested");
    expect(deps.flushAll).toHaveBeenCalledOnce();
    expect(prepareNativeClose).toHaveBeenCalledOnce();
    expect(exit).toHaveBeenCalledTimes(2);
    expect(transitions).toEqual([true]);
  });
});
