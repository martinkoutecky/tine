import { describe, expect, it, vi } from "vitest";
import { requestAndroidRootClose } from "./androidBack";
import { createSafeCloseCoordinator, type SafeCloseDeps } from "./safeClose";

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
    notifyConfirmationFailure: vi.fn(),
    runBounded: async (operation) => operation,
    ...overrides,
  };
  return { deps, transitions, safeClose: createSafeCloseCoordinator(deps) };
}

describe("GH #161 shared safe-close transaction", () => {
  it("flushes graph and session once before an accepted Android root exit", async () => {
    const { deps, safeClose, transitions } = harness();
    const exit = vi.fn(async () => {});

    await expect(requestAndroidRootClose(safeClose, exit, vi.fn())).resolves.toBe("exit_requested");
    expect(deps.blurActive).toHaveBeenCalledOnce();
    expect(deps.endEdit).toHaveBeenCalledOnce();
    expect(deps.flushPdfWork).toHaveBeenCalledOnce();
    expect(deps.flushAll).toHaveBeenCalledOnce();
    expect(deps.confirmDiscard).not.toHaveBeenCalled();
    expect(deps.flushSession).toHaveBeenCalledOnce();
    expect(exit).toHaveBeenCalledOnce();
    expect(transitions).toEqual([true]);
    expect(safeClose.inFlight()).toBe(true);
  });

  it("consumes repeated root Back while the first flush is in flight", async () => {
    const flush = deferred<boolean>();
    const { deps, safeClose } = harness({ flushAll: vi.fn(() => flush.promise) });
    const exit = vi.fn(async () => {});

    const first = requestAndroidRootClose(safeClose, exit, vi.fn());
    await Promise.resolve();
    await expect(requestAndroidRootClose(safeClose, exit, vi.fn())).resolves.toBe("in_flight");
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

    await expect(requestAndroidRootClose(safeClose, exit, vi.fn())).resolves.toBe("rejected");
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

  it("continues only after explicit discard when graph flush fails or times out", async () => {
    for (const mode of ["failure", "timeout"] as const) {
      const never = new Promise<boolean>(() => {});
      const flushAll = mode === "failure" ? vi.fn(async () => false) : vi.fn(() => never);
      const runBounded: SafeCloseDeps["runBounded"] = async (operation, timeoutMs, fallback) => {
        if (mode === "timeout" && operation === never && timeoutMs === 4000) return fallback;
        return operation;
      };
      const { deps, safeClose } = harness({
        flushAll,
        confirmDiscard: vi.fn(async () => true),
        runBounded,
      });
      const exit = vi.fn(async () => {});

      await expect(requestAndroidRootClose(safeClose, exit, vi.fn())).resolves.toBe("exit_requested");
      expect(deps.confirmDiscard).toHaveBeenCalledOnce();
      expect(deps.flushSession).toHaveBeenCalledOnce();
      expect(exit).toHaveBeenCalledOnce();
    }
  });

  it("treats confirmation failure as rejection and leaves edits open", async () => {
    const { deps, safeClose, transitions } = harness({
      flushAll: vi.fn(async () => { throw new Error("save failed"); }),
      confirmDiscard: vi.fn(async () => { throw new Error("dialog failed"); }),
    });
    const exit = vi.fn(async () => {});

    await expect(requestAndroidRootClose(safeClose, exit, vi.fn())).resolves.toBe("rejected");
    expect(deps.notifyConfirmationFailure).toHaveBeenCalledOnce();
    expect(exit).not.toHaveBeenCalled();
    expect(transitions).toEqual([true, false]);
  });

  it("keeps accepted graph-close policy when the best-effort session flush fails", async () => {
    const { safeClose } = harness({
      flushSession: vi.fn(async () => { throw new Error("session failed"); }),
    });
    const exit = vi.fn(async () => {});

    await expect(requestAndroidRootClose(safeClose, exit, vi.fn())).resolves.toBe("exit_requested");
    expect(exit).toHaveBeenCalledOnce();
  });

  it("resets an accepted transaction after invoke failure so a later Back can retry", async () => {
    const { deps, safeClose, transitions } = harness();
    const exit = vi.fn()
      .mockRejectedValueOnce(new Error("plugin unavailable"))
      .mockResolvedValueOnce(undefined);
    const exitFailed = vi.fn();

    await expect(requestAndroidRootClose(safeClose, exit, exitFailed)).resolves.toBe("exit_failed");
    expect(exitFailed).toHaveBeenCalledOnce();
    expect(safeClose.inFlight()).toBe(false);
    await expect(requestAndroidRootClose(safeClose, exit, exitFailed)).resolves.toBe("exit_requested");
    expect(deps.flushAll).toHaveBeenCalledTimes(2);
    expect(exit).toHaveBeenCalledTimes(2);
    expect(transitions).toEqual([true, false, true]);
  });
});
