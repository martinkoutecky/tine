// Family 10 reload on focus (master d56219d73, b3d64addee39): returning to the
// window asks the backend for one full stat diff, holds new edits until that
// rescan's events are applied, coalesces and throttles, and never strands the
// editor when the fallback fails.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { deferEditorStartUntilFresh, freshnessPending } from "./freshnessBarrier";
import { refreshOnReturnToWindow, resetFocusRescanThrottle, subscribeWatcherFreshness, trackGraphChangeApplication } from "./reloadOnFocus";
import { setToasts, toasts } from "./toasts";

type Api = ReturnType<typeof backend>;
// The completion listener subscribes once per app lifetime, as in production.
let complete: ((sequence: number) => void) | null = null;
let sequence: number;
let rescans: number;
let round = 0;

beforeEach(() => {
  sequence = 100 * ++round; rescans = 0; setToasts([]);
  resetFocusRescanThrottle();
  const api = backend() as Api;
  api.onGraphRescanComplete = async (cb) => { complete = cb; return () => {}; };
  api.rescanGraphNow = async () => { rescans++; return ++sequence; };
});
afterEach(() => {
  const api = backend() as Api;
  delete api.onGraphRescanComplete;
  delete api.rescanGraphNow;
});

describe("reload on focus", () => {
  it("rescans once, holds new edits until the rescan's events are applied, then releases them", async () => {
    const refresh = refreshOnReturnToWindow(10_000);
    expect(freshnessPending()).toBe(true);
    const started = vi.fn();
    expect(deferEditorStartUntilFresh(started)).toBe(true);
    await vi.waitFor(() => expect(rescans).toBe(1));
    let applied!: () => void;
    trackGraphChangeApplication(new Promise<void>((resolve) => { applied = resolve; }));
    complete!(sequence);
    await Promise.resolve();
    expect(freshnessPending()).toBe(true); // the change is still being applied
    applied();
    await refresh;
    expect(freshnessPending()).toBe(false);
    expect(started).toHaveBeenCalledTimes(1);
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

  it("a failed rescan says so and releases the editor", async () => {
    (backend() as Api).rescanGraphNow = async () => { throw new Error("scan refused"); };
    await refreshOnReturnToWindow(40_000);
    expect(freshnessPending()).toBe(false);
    expect(toasts().map((t) => t.kind)).toEqual(["error"]);
    expect(toasts()[0].message).toContain("scan refused");
  });

  it("a refused OS watch is said out loud with its fallback, and its return too (I-9)", async () => {
    const api = backend() as Api;
    let report!: (status: { refused: boolean; message: string }) => void;
    api.onGraphWatchStatus = async (cb) => { report = cb; return () => {}; };
    const unsubscribe = await subscribeWatcherFreshness();
    report({ refused: true, message: "inotify watch limit reached" });
    expect(toasts()[0]).toMatchObject({ kind: "warn", sticky: true });
    expect(toasts()[0].message).toContain("inotify watch limit reached");
    expect(toasts()[0].message).toContain("every 3 seconds");
    report({ refused: false, message: "" });
    expect(toasts()[1].message).toBe("Live file notifications are back for this graph.");
    unsubscribe();
    delete api.onGraphWatchStatus;
  });
});
