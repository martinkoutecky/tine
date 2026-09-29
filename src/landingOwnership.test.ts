// og-G2 row #52 (master cdd0eda4b317, Harvest D): ported outcome tests (node pool).
import { afterEach, describe, expect, it, vi } from "vitest";
import { bumpGraphEpoch, setGraphMeta } from "./graphSession";
import { changeJournalTitleFormat, setGraphTransitioning } from "./ui";
import { invalidateBinding } from "./binding";
import { capturePluginGraphOwner, isPluginGraphOwnerCurrent } from "./plugins/ownership";
import { backend } from "./backend";
import { refreshOnReturnToWindow, resetFocusRescanThrottle } from "./reloadOnFocus";
import { setToasts, toasts } from "./toasts";

afterEach(() => {
  setGraphTransitioning(false);
  setGraphMeta(null);
  vi.restoreAllMocks();
});

describe("O4 plugin owner survives a render-epoch repaint (master ownership.test.ts / manager.test.ts)", () => {
  it("a display-only epoch bump (journal title format change) keeps the owner current", () => {
    setGraphMeta({ root: "/graph-a", journal_page_title_format: "MMM do, yyyy" } as never);
    vi.spyOn(backend(), "setJournalTitleFormat").mockImplementation(() => new Promise(() => {}) as never);
    const owner = capturePluginGraphOwner()!;
    expect(isPluginGraphOwnerCurrent(owner)).toBe(true);
    changeJournalTitleFormat("yyyy-MM-dd");
    expect(isPluginGraphOwnerCurrent(owner)).toBe(true);
  });
  it("a bare bumpGraphEpoch (as master's test) keeps the owner current", () => {
    setGraphMeta({ root: "/graph-a" } as never);
    const owner = capturePluginGraphOwner()!;
    bumpGraphEpoch();
    expect(isPluginGraphOwnerCurrent(owner)).toBe(true);
  });
  it("a graph rebind still retires the owner (control)", () => {
    setGraphMeta({ root: "/graph-a" } as never);
    const owner = capturePluginGraphOwner()!;
    setGraphMeta({ root: "/graph-b" } as never);
    expect(isPluginGraphOwnerCurrent(owner)).toBe(false);
  });
});

describe("O5 focus rescan is scoped to the graph binding (master reloadOnFocus.test.tsx)", () => {
  it("drops the old graph tail and runs one non-overlapping refresh for the new binding", async () => {
    setToasts([]);
    resetFocusRescanThrottle();
    const api = backend() as ReturnType<typeof backend>;
    let generation = 1;
    vi.spyOn(api, "graphBindingGeneration").mockImplementation(() => generation);
    let complete: ((s: number) => void) | null = null;
    api.onGraphRescanComplete = async (cb) => { complete = cb; return () => {}; };
    let finishOld!: (s: number) => void;
    let calls = 0;
    let inFlight = 0, maxInFlight = 0;
    api.rescanGraphNow = () => {
      calls++; inFlight++; maxInFlight = Math.max(maxInFlight, inFlight);
      if (calls === 1) return new Promise<number>((r) => { finishOld = (s) => { inFlight--; r(s); }; });
      inFlight--;
      return Promise.resolve(2_000_002);
    };
    try {
      const old = refreshOnReturnToWindow(100_000);
      await vi.waitFor(() => expect(calls).toBe(1));
      generation = 2; invalidateBinding();
      const replacement = refreshOnReturnToWindow(100_001);
      finishOld(2_000_001);
      await old;
      await vi.waitFor(() => expect(calls).toBe(2));
      complete!(2_000_002);
      await replacement;
      expect(maxInFlight).toBe(1);
      expect(toasts()).toEqual([]);
    } finally {
      delete (api as Partial<typeof api>).onGraphRescanComplete;
      delete (api as Partial<typeof api>).rescanGraphNow;
    }
  });
});
