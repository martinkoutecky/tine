import { afterEach, describe, expect, it, vi } from "vitest";
import {
  activatePdfOwnership,
  currentPdfOwnership,
  drainPdfWork,
  isPdfOwnershipCurrent,
  registerPdfParticipant,
  resetPdfOwnershipForTest,
  retirePdfOwnership,
  trackPdfMutation,
} from "./pdfOwnership";

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

afterEach(() => {
  resetPdfOwnershipForTest();
  vi.restoreAllMocks();
});

describe("window-local PDF graph ownership", () => {
  it("flushes pending state and complete mutations before retiring an owner", async () => {
    const owner = activatePdfOwnership("/graphs/A");
    const mutation = deferred<void>();
    const events: string[] = [];
    registerPdfParticipant(owner, {
      flush: async () => {
        events.push("flush-view");
        void trackPdfMutation(owner, async () => {
          events.push("write-start");
          await mutation.promise;
          events.push("write-end");
        });
        return true;
      },
      cancel: () => events.push("cancel"),
    });

    const drain = drainPdfWork();
    await Promise.resolve();
    await Promise.resolve();
    expect(events).toEqual(["flush-view", "write-start"]);
    mutation.resolve();
    await expect(drain).resolves.toBe(true);

    retirePdfOwnership();
    expect(events).toEqual(["flush-view", "write-start", "write-end", "cancel"]);
    expect(currentPdfOwnership()).toBeNull();
  });

  it("gives a same-root refresh a new generation and rejects stale callbacks", async () => {
    const first = activatePdfOwnership("/graphs/A");
    retirePdfOwnership();
    const refreshed = activatePdfOwnership("/graphs/A");
    const write = vi.fn(async () => {});

    expect(refreshed.generation).toBeGreaterThan(first.generation);
    expect(isPdfOwnershipCurrent(first)).toBe(false);
    await expect(trackPdfMutation(first, write)).rejects.toThrow("stale PDF graph ownership");
    expect(write).not.toHaveBeenCalled();
  });

  it("reports a participant or mutation failure without invalidating the owner", async () => {
    const owner = activatePdfOwnership("/graphs/A");
    registerPdfParticipant(owner, {
      flush: async () => false,
      cancel: vi.fn(),
    });

    await expect(drainPdfWork()).resolves.toBe(false);
    expect(isPdfOwnershipCurrent(owner)).toBe(true);
  });
});
