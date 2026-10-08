// Family 10 reload on focus (master d56219d73, b3d64addee39): returning to the
// window asks the backend for one full stat diff and waits for its events to be
// applied, coalesces and throttles, and never blocks typing while it runs (K22,
// SPEC-storage §6.1: the save guard, not an input barrier, protects stale bytes).
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { REFRESH_NOTICE_DELAY_MS, refreshingFromDisk, refreshOnReturnToWindow, rescanGraphNowFromSettings, resetFocusRescanThrottle, subscribeWatcherFreshness, trackGraphChangeApplication } from "./reloadOnFocus";
import { setToasts, toasts } from "./toasts";
import { setGraphTransitioning } from "./ui";

type Api = ReturnType<typeof backend>;
// The completion listener subscribes once per app lifetime, as in production.
let complete: ((sequence: number) => void) | null = null;
let sequence: number;
let rescans: number;
let rebuilds: Array<boolean | undefined>;
let round = 0;

beforeEach(() => {
  sequence = 100 * ++round; rescans = 0; rebuilds = []; setToasts([]);
  resetFocusRescanThrottle();
  const api = backend() as Api;
  api.onGraphRescanComplete = async (cb) => { complete = cb; return () => {}; };
  api.rescanGraphNow = async (rebuild) => { rescans++; rebuilds.push(rebuild); return ++sequence; };
});
afterEach(() => {
  const api = backend() as Api;
  delete api.onGraphRescanComplete;
  delete api.rescanGraphNow;
});

describe("reload on focus", () => {
  it("rescans once and finishes only after the rescan's events are applied", async () => {
    const refresh = refreshOnReturnToWindow(10_000);
    let done = false;
    void refresh.then(() => { done = true; });
    await vi.waitFor(() => expect(rescans).toBe(1));
    let applied!: () => void;
    trackGraphChangeApplication(new Promise<void>((resolve) => { applied = resolve; }));
    complete!(sequence);
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(done).toBe(false); // the change is still being applied
    applied();
    await refresh;
    expect(done).toBe(true);
  });

  it("a rescan slower than the notice delay says so, and clears the status when it ends", async () => {
    const refresh = refreshOnReturnToWindow(16_000);
    await vi.waitFor(() => expect(rescans).toBe(1));
    expect(refreshingFromDisk()).toBe(false); // a fast rescan never flashes a notice
    expect(REFRESH_NOTICE_DELAY_MS).toBe(500);
    await new Promise((resolve) => setTimeout(resolve, REFRESH_NOTICE_DELAY_MS - 150));
    expect(refreshingFromDisk()).toBe(false); // still quiet well after the old 120 ms delay
    await vi.waitFor(() => expect(refreshingFromDisk()).toBe(true));
    complete!(sequence);
    await refresh;
    expect(refreshingFromDisk()).toBe(false);
  });

  it("records the phase split of a focus rescan as numbers only (GH #623)", async () => {
    const events: Array<[string, number]> = [];
    (backend() as Api).diagnosticTimingEvent = async (name, ms) => { events.push([name, ms]); };
    try {
      const refresh = refreshOnReturnToWindow(18_000);
      await vi.waitFor(() => expect(rescans).toBe(1));
      let applied!: () => void;
      trackGraphChangeApplication(new Promise<void>((resolve) => { applied = resolve; }));
      complete!(sequence);
      applied();
      await refresh;
      expect(events.map(([name]) => name)).toEqual(["focus.ipc", "focus.wait", "focus.apply", "focus.total"]);
      for (const [, ms] of events) expect(Number.isFinite(ms) && ms >= 0).toBe(true);
    } finally { delete (backend() as Api).diagnosticTimingEvent; }
  });

  it("records how long the notice was visible, only when it showed", async () => {
    const events: string[] = [];
    (backend() as Api).diagnosticTimingEvent = async (name) => { events.push(name); };
    try {
      const refresh = refreshOnReturnToWindow(19_000);
      await vi.waitFor(() => expect(rescans).toBe(1));
      await vi.waitFor(() => expect(refreshingFromDisk()).toBe(true));
      complete!(sequence);
      await refresh;
      expect(events).toContain("focus.banner");
      expect(events.indexOf("focus.total")).toBeGreaterThanOrEqual(0);
    } finally { delete (backend() as Api).diagnosticTimingEvent; }
  });

  it("does not record the focus phases for a Settings rebuild", async () => {
    const events: string[] = [];
    (backend() as Api).diagnosticTimingEvent = async (name) => { events.push(name); };
    try {
      const pending = rescanGraphNowFromSettings();
      await vi.waitFor(() => expect(rescans).toBe(1));
      complete!(sequence);
      await pending;
      expect(events.filter((name) => name !== "focus.banner")).toEqual([]);
    } finally { delete (backend() as Api).diagnosticTimingEvent; }
  });

  it("coalesces a focus during a rescan and throttles a quick second return", async () => {
    const first = refreshOnReturnToWindow(20_000);
    expect(refreshOnReturnToWindow(20_100)).toBe(first);
    await vi.waitFor(() => expect(rescans).toBe(1));
    complete!(sequence);
    await first;
    await refreshOnReturnToWindow(20_500);
    expect(rescans).toBe(1);
    const later = refreshOnReturnToWindow(30_000);
    await vi.waitFor(() => expect(rescans).toBe(2));
    complete!(sequence);
    await later;
  });

  it("a failed rescan says so and clears the status", async () => {
    (backend() as Api).rescanGraphNow = async () => { throw new Error("scan refused"); };
    await refreshOnReturnToWindow(40_000);
    expect(refreshingFromDisk()).toBe(false);
    expect(toasts().map((t) => t.kind)).toEqual(["error"]);
    expect(toasts()[0].message).toContain("scan refused");
  });

  it("a refused OS watch is said out loud with its fallback, and its return too (I-9)", async () => {
    const api = backend() as Api;
    let report!: (status: { refused: boolean; message: string }) => void;
    api.onGraphWatchStatus = async (cb) => { report = cb; return () => {}; };
    const unsubscribe = subscribeWatcherFreshness();
    report({ refused: true, message: "inotify watch limit reached" });
    expect(toasts()[0]).toMatchObject({ kind: "warn", sticky: true });
    expect(toasts()[0].message).toContain("inotify watch limit reached");
    expect(toasts()[0].message).toContain("every 3 seconds");
    report({ refused: false, message: "" });
    expect(toasts()[1].message).toBe("Live file notifications are back for this graph.");
    unsubscribe();
    delete api.onGraphWatchStatus;
  });

  // OG-TOAST T1 (Martin 2026-09-29): at launch the OS focus event reached the
  // fallback before the window's graph binding existed; the backend refused the
  // rescan (`missing-graph-binding` while load_graph's answer was in flight,
  // `no graph loaded for window main` on the Welcome screen) and a red toast
  // reported a transient startup state as a failure.
  it("a return to a window whose graph is not bound yet rescans nothing and reports nothing", async () => {
    const api = backend() as Api;
    const unbound = vi.spyOn(api, "graphBindingGeneration").mockReturnValue(0);
    api.rescanGraphNow = async () => { rescans++; throw new Error(api.graphBindingGeneration() ? "boom" : "missing-graph-binding"); };
    try {
      await refreshOnReturnToWindow(50_000);
      expect(rescans).toBe(0);
      expect(toasts()).toEqual([]);
      expect(refreshingFromDisk()).toBe(false);
    } finally { unbound.mockRestore(); }
  });

  it("a return during a graph load or restore waits for it instead of rescanning", async () => {
    setGraphTransitioning(true);
    try {
      await refreshOnReturnToWindow(60_000);
      expect(rescans).toBe(0);
      expect(toasts()).toEqual([]);
    } finally { setGraphTransitioning(false); }
    const after = refreshOnReturnToWindow(70_000);
    await vi.waitFor(() => expect(rescans).toBe(1));
    complete!(sequence);
    await after;
  });

  it("a rescan refused because the graph was switched meanwhile is stale, not a failure", async () => {
    const api = backend() as Api;
    let generation = 1;
    const spy = vi.spyOn(api, "graphBindingGeneration").mockImplementation(() => generation);
    api.rescanGraphNow = async () => { rescans++; generation = 2; throw new Error("stale-graph-binding"); };
    try {
      await refreshOnReturnToWindow(80_000);
      expect(rescans).toBe(1);
      expect(toasts()).toEqual([]);
      expect(refreshingFromDisk()).toBe(false);
    } finally { spy.mockRestore(); }
  });

  // GH #623: Settings > Help & diagnostics "Rescan graph" is one full stat diff
  // on demand: not throttled, answers when it finished, and a rescan already in
  // flight does not stand in for it.
  it("a Settings rescan ignores the focus throttle and answers when it finished", async () => {
    const focus = refreshOnReturnToWindow(Date.now());
    await vi.waitFor(() => expect(rescans).toBe(1));
    complete!(sequence);
    await focus;
    const before = Date.now();
    const settings = rescanGraphNowFromSettings();
    await vi.waitFor(() => expect(rescans).toBe(2)); // a focus return now would be throttled
    complete!(sequence);
    const finished = await settings;
    expect(finished).not.toBeNull();
    expect(finished!).toBeGreaterThanOrEqual(before);
  });

  // The Settings button is a forced rebuild (ignores stamps); the focus return
  // stays the cheap stat diff and never asks for one.
  it("only the Settings rescan asks for the forced rebuild", async () => {
    const focus = refreshOnReturnToWindow(Date.now());
    await vi.waitFor(() => expect(rescans).toBe(1));
    complete!(sequence);
    await focus;
    const settings = rescanGraphNowFromSettings();
    await vi.waitFor(() => expect(rescans).toBe(2));
    complete!(sequence);
    await settings;
    expect(rebuilds.map(Boolean)).toEqual([false, true]);
  });

  it("a Settings rescan waits for one in flight and then runs its own", async () => {
    const focus = refreshOnReturnToWindow(Date.now());
    await vi.waitFor(() => expect(rescans).toBe(1));
    const settings = rescanGraphNowFromSettings();
    complete!(sequence);
    await focus;
    await vi.waitFor(() => expect(rescans).toBe(2));
    complete!(sequence);
    expect(await settings).not.toBeNull();
  });

  it("a Settings rescan that could not run says so with null, never a time", async () => {
    delete (backend() as Api).rescanGraphNow;
    expect(await rescanGraphNowFromSettings()).toBeNull();
    expect(rescans).toBe(0);
  });
});
