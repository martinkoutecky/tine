import { beforeEach, describe, expect, it, vi } from "vitest";

const backendMock = vi.hoisted(() => ({ getPage: vi.fn() }));
vi.mock("../backend", () => ({ backend: () => backendMock }));

import { loadSingle, pageByName, resetStore } from "../store";
import type { PageDto, PageKind, RefGroup } from "../types";
import { bumpGraphEpoch, setGraphMeta } from "../ui";
import {
  hydrateVisibleQueryPages,
  queryHydrationCircuitStatus,
  retryQueryHydrationCircuit,
} from "./queryHydration";

function group(page: string, kind: PageKind, id: string): RefGroup {
  return {
    page,
    kind,
    blocks: [{ id, raw: `${kind} row`, collapsed: false, children: [] }],
  };
}

function page(name: string, kind: PageKind): PageDto {
  return { name, kind, title: name, pre_block: null, blocks: [] };
}

beforeEach(() => {
  resetStore();
  backendMock.getPage.mockReset();
  backendMock.getPage.mockImplementation(async (name: string, kind: PageKind) => ({
    name,
    kind,
    title: name,
    pre_block: null,
    blocks: [],
  }));
  setGraphMeta({ root: "/graph" } as any);
});

describe("query sheet hydration identity", () => {
  it("hydrates the visible block's kind when a page and journal share a name", async () => {
    const groups = [group("Twin", "page", "page-block"), group("Twin", "journal", "journal-block")];

    await hydrateVisibleQueryPages([{ id: "page-block", page: "Twin" }], groups);

    expect(backendMock.getPage).toHaveBeenCalledTimes(1);
    expect(backendMock.getPage).toHaveBeenCalledWith("Twin", "page");
  });

  it("keeps simultaneously visible page/journal twins DTO-only", async () => {
    const groups = [group("Twin", "page", "page-block"), group("Twin", "journal", "journal-block")];

    await hydrateVisibleQueryPages(
      [
        { id: "page-block", page: "Twin" },
        { id: "journal-block", page: "Twin" },
      ],
      groups,
    );

    expect(backendMock.getPage).not.toHaveBeenCalled();
  });

  it("serializes same-name twin claims across invocations and preserves a post-await occupant", async () => {
    let finishPage!: (dto: PageDto) => void;
    backendMock.getPage.mockImplementation(
      (name: string, kind: PageKind) => kind === "page"
        ? new Promise<PageDto>((resolve) => { finishPage = resolve; })
        : Promise.resolve(page(name, kind)),
    );
    const groups = [group("Twin", "page", "page-block"), group("Twin", "journal", "journal-block")];

    const pageHydration = hydrateVisibleQueryPages([{ id: "page-block", page: "Twin" }], groups);
    const journalHydration = hydrateVisibleQueryPages([{ id: "journal-block", page: "Twin" }], groups);
    expect(backendMock.getPage).toHaveBeenCalledTimes(1);
    expect(backendMock.getPage).toHaveBeenCalledWith("Twin", "page");

    // Another surface loads the twin while the claimed request is in flight.
    loadSingle(page("Twin", "journal"));
    finishPage(page("Twin", "page"));
    await Promise.all([pageHydration, journalHydration]);

    expect(pageByName("Twin")?.kind).toBe("journal");
    expect(backendMock.getPage).toHaveBeenCalledTimes(1);
  });

  it("cancels old-graph queued reads before IPC so new hydration is not starved", async () => {
    const releases: ((dto: PageDto) => void)[] = [];
    backendMock.getPage.mockImplementation((name: string, kind: PageKind) => {
      if (name.startsWith("Old")) {
        return new Promise<PageDto>((resolve) => releases.push(resolve));
      }
      return Promise.resolve(page(name, kind));
    });
    const oldGroups = Array.from({ length: 6 }, (_, i) => group(`Old${i}`, "page", `old-${i}`));
    const oldRows = oldGroups.map((g) => ({ id: g.blocks[0].id, page: g.page }));

    const oldHydration = hydrateVisibleQueryPages(oldRows, oldGroups);
    expect(backendMock.getPage).toHaveBeenCalledTimes(4);

    setGraphMeta({ root: "/graph-b" } as any);
    bumpGraphEpoch();
    const freshGroup = group("Fresh", "page", "fresh");
    const freshHydration = hydrateVisibleQueryPages([{ id: "fresh", page: "Fresh" }], [freshGroup]);
    // Old active reads may still resolve later, but they no longer consume the
    // new graph's limiter slots; Fresh reaches IPC before any old release.
    expect(backendMock.getPage.mock.calls.map(([name]) => name)).toContain("Fresh");

    releases.forEach((resolve, i) => resolve(page(`Old${i}`, "page")));
    await Promise.all([oldHydration, freshHydration]);

    const requestedNames = backendMock.getPage.mock.calls.map(([name]) => name);
    expect(requestedNames).toContain("Fresh");
    expect(requestedNames).not.toContain("Old4");
    expect(requestedNames).not.toContain("Old5");
    expect(pageByName("Fresh")?.kind).toBe("page");
    expect(pageByName("Old0")).toBeUndefined();
  });

  it("bounds physical IPC concurrency across repeated graph switches", async () => {
    let active = 0;
    let peak = 0;
    const pending = new Map<string, () => void>();
    backendMock.getPage.mockImplementation((name: string, kind: PageKind) => {
      active++;
      peak = Math.max(peak, active);
      return new Promise<PageDto>((resolve) => {
        pending.set(name, () => {
          pending.delete(name);
          active--;
          resolve(page(name, kind));
        });
      });
    });
    const batch = (prefix: string, count: number) => {
      const groups = Array.from({ length: count }, (_, i) => group(`${prefix}${i}`, "page", `${prefix}-${i}`));
      return hydrateVisibleQueryPages(
        groups.map((g) => ({ id: g.blocks[0].id, page: g.page })),
        groups,
      );
    };

    setGraphMeta({ root: "/rapid-a" } as any);
    bumpGraphEpoch();
    const a = batch("A", 6); // A0..A3 start; A4/A5 remain queued.
    setGraphMeta({ root: "/rapid-b" } as any);
    bumpGraphEpoch();
    const b = batch("B", 6); // B0..B3 start; total physical reads reaches 8.
    setGraphMeta({ root: "/rapid-c" } as any);
    bumpGraphEpoch();
    const c = batch("C", 2); // Cannot start until one of the physical eight settles.

    let names = backendMock.getPage.mock.calls.map(([name]) => name);
    expect(names).toEqual(expect.arrayContaining(["A0", "A1", "A2", "A3", "B0", "B1", "B2", "B3"]));
    expect(names).not.toEqual(expect.arrayContaining(["A4", "A5", "B4", "B5", "C0", "C1"]));
    expect(active).toBe(8);
    expect(peak).toBe(8);

    // Release the physical old work. Current-scope C jobs take the freed global
    // slots, but the observed peak never exceeds the hard cap.
    [...pending.values()].forEach((release) => release());
    await vi.waitFor(() => {
      names = backendMock.getPage.mock.calls.map(([name]) => name);
      expect(names).toEqual(expect.arrayContaining(["C0", "C1"]));
    });
    [...pending.values()].forEach((release) => release());
    await Promise.all([a, b, c]);

    names = backendMock.getPage.mock.calls.map(([name]) => name);
    expect(names).not.toEqual(expect.arrayContaining(["A4", "A5", "B4", "B5"]));
    expect(peak).toBeLessThanOrEqual(8);
    expect(active).toBe(0);
  });

  it("bounds the short-grace recovery tranche before stale leases expire", async () => {
    vi.useFakeTimers();
    try {
      let active = 0;
      let peak = 0;
      const pending = new Map<string, () => void>();
      backendMock.getPage.mockImplementation((name: string, kind: PageKind) => {
        active++;
        peak = Math.max(peak, active);
        return new Promise<PageDto>((resolve) => {
          pending.set(name, () => {
            pending.delete(name);
            active--;
            resolve(page(name, kind));
          });
        });
      });
      const batch = (root: string, prefix: string, count: number) => {
        setGraphMeta({ root } as any);
        bumpGraphEpoch();
        const groups = Array.from({ length: count }, (_, i) => group(`${prefix}${i}`, "page", `${prefix}-${i}`));
        return hydrateVisibleQueryPages(
          groups.map((g) => ({ id: g.blocks[0].id, page: g.page })),
          groups,
        );
      };

      const a = batch("/hung-a", "HA", 6); // 4 running, 2 stale-queued.
      const b = batch("/hung-b", "HB", 6); // 4 more running, A's queued work cancelled.
      const c = batch("/hung-c", "HC", 4); // blocked at the normal physical ceiling of 8.
      expect(active).toBe(8);
      expect(backendMock.getPage.mock.calls.map(([name]) => name)).not.toContain("HC0");

      await vi.advanceTimersByTimeAsync(1_000);
      expect(backendMock.getPage.mock.calls.map(([name]) => name)).toEqual(
        expect.arrayContaining(["HC0", "HC1", "HC2", "HC3"]),
      );
      expect(active).toBe(12);
      expect(peak).toBe(12);

      // Before the longer stale lease expires, a fourth switch cannot leak a
      // thirteenth invocation. Its latest request remains queued.
      const d = batch("/hung-d", "HD", 1);
      await vi.advanceTimersByTimeAsync(500);
      expect(backendMock.getPage.mock.calls.map(([name]) => name)).not.toContain("HD0");
      expect(active).toBe(12);

      // Settlement of any physical call is the explicit retry path: the newest
      // queued graph starts automatically without exceeding the absolute cap.
      pending.get("HA0")!();
      await vi.waitFor(() => {
        expect(backendMock.getPage.mock.calls.map(([name]) => name)).toContain("HD0");
      });
      expect(peak).toBeLessThanOrEqual(12);
      expect(active).toBe(12);

      [...pending.values()].forEach((release) => release());
      await Promise.all([a, b, c, d]);
      expect(active).toBe(0);
      const names = backendMock.getPage.mock.calls.map(([name]) => name);
      expect(names).not.toEqual(expect.arrayContaining(["HA4", "HA5", "HB4", "HB5"]));
    } finally {
      vi.useRealTimers();
    }
  });

  it("expires hung stale leases, bounds quarantine growth, and retries after circuit backoff", async () => {
    vi.useFakeTimers();
    try {
      let active = 0;
      let peak = 0;
      const pending = new Map<string, () => void>();
      backendMock.getPage.mockImplementation((name: string, kind: PageKind) => {
        active++;
        peak = Math.max(peak, active);
        return new Promise<PageDto>((resolve) => {
          pending.set(name, () => {
            pending.delete(name);
            active--;
            resolve(page(name, kind));
          });
        });
      });
      const batch = (root: string, prefix: string, count: number) => {
        setGraphMeta({ root } as any);
        bumpGraphEpoch();
        const groups = Array.from({ length: count }, (_, i) => group(`${prefix}${i}`, "page", `${prefix}-${i}`));
        return hydrateVisibleQueryPages(
          groups.map((g) => ({ id: g.blocks[0].id, page: g.page })),
          groups,
        );
      };

      const a = batch("/lease-a", "LA", 4);
      const b = batch("/lease-b", "LB", 4);
      const c = batch("/lease-c", "LC", 4);
      await vi.advanceTimersByTimeAsync(1_000); // opens the normal 8→12 recovery tranche
      expect(active).toBe(12);

      // With twelve old calls hung, the next healthy graph receives a bounded
      // opportunity when the oldest stale leases expire instead of starving.
      const d = batch("/lease-d", "LD", 4);
      await vi.advanceTimersByTimeAsync(2_500);
      expect(backendMock.getPage.mock.calls.map(([name]) => name)).toEqual(
        expect.arrayContaining(["LD0", "LD1", "LD2", "LD3"]),
      );
      expect(active).toBe(16);
      expect(peak).toBe(16);

      // A fifth all-hung switch opens the observable circuit and cannot start a
      // seventeenth call while the backoff is running.
      const e = batch("/lease-e", "LE", 1);
      await vi.advanceTimersByTimeAsync(500);
      expect(backendMock.getPage.mock.calls.map(([name]) => name)).not.toContain("LE0");
      expect(queryHydrationCircuitStatus()).toMatchObject({
        state: "open",
        underlying: 16,
        queued: 1,
      });
      expect(retryQueryHydrationCircuit()).toBe(false);

      // Real capacity recovery observes the 1s circuit backoff, then retries the
      // latest graph automatically and still never exceeds sixteen calls.
      pending.get("LA0")!();
      await Promise.resolve();
      expect(backendMock.getPage.mock.calls.map(([name]) => name)).not.toContain("LE0");
      await vi.advanceTimersByTimeAsync(499);
      expect(backendMock.getPage.mock.calls.map(([name]) => name)).not.toContain("LE0");
      await vi.advanceTimersByTimeAsync(1);
      expect(backendMock.getPage.mock.calls.map(([name]) => name)).toContain("LE0");
      expect(peak).toBeLessThanOrEqual(16);

      [...pending.values()].forEach((release) => release());
      await Promise.all([a, b, c, d, e]);
      expect(active).toBe(0);
      expect(queryHydrationCircuitStatus().underlying).toBe(0);
    } finally {
      vi.useRealTimers();
    }
  });
});
