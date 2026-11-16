// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { clearRenderStalls, createStallBuffer, installRenderStallMonitor, renderStallSnapshot, safeScriptSource } from "./renderStalls";

afterEach(() => { vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals(); clearRenderStalls(); document.body.replaceChildren(); });

describe("renderer stall diagnostics", () => {
  it("retains a bounded worst-stall ring and never stores normal frames", () => {
    const ring = createStallBuffer(3);
    for (let i = 0; i < 1000; i++) ring.record({ kind: "frame", startTime: i, duration: 16 });
    expect(ring.snapshot()).toEqual([]);
    for (const duration of [110, 500, 200, 120, 700]) ring.record({ kind: "frame", startTime: duration, duration });
    expect(ring.snapshot().map(x => x.duration)).toEqual([700, 500, 200]);
    ring.clear();
    expect(ring.snapshot()).toEqual([]);
  });
  it("keeps only bundled script filenames, never paths, URLs or user function names", () => {
    expect(safeScriptSource("http://tauri.localhost/assets/index-ab12.js?secret" )).toBe("index-ab12.js");
    expect(safeScriptSource("https://example.com/private.js")).toBeUndefined();
    expect(safeScriptSource("file:///home/person/graph.js")).toBeUndefined();
  });
  it("measures key -> frame with one frame request per burst, never a per-frame loop", () => {
    let next: FrameRequestCallback | undefined;
    const request = vi.fn((callback: FrameRequestCallback) => { next = callback; return 1; });
    vi.stubGlobal("requestAnimationFrame", request);
    vi.stubGlobal("cancelAnimationFrame", vi.fn(() => { next = undefined; }));
    vi.spyOn(document, "hidden", "get").mockReturnValue(false);
    vi.spyOn(performance, "now").mockReturnValue(100);
    const stop = installRenderStallMonitor();
    expect(request).not.toHaveBeenCalled();
    const editor = document.createElement("textarea");
    document.body.append(editor);
    editor.dispatchEvent(new Event("keydown", { bubbles: true }));
    editor.dispatchEvent(new Event("input", { bubbles: true }));
    expect(request).toHaveBeenCalledTimes(1); // a burst shares one frame request
    next!(250);
    expect(renderStallSnapshot().stalls.some(stall => stall.kind === "key-frame" && stall.duration === 150)).toBe(true);
    expect(request).toHaveBeenCalledTimes(1); // the frame callback requests no successor
    clearRenderStalls();
    editor.dispatchEvent(new Event("input", { bubbles: true }));
    document.dispatchEvent(new Event("visibilitychange"));
    expect(next).toBeUndefined();
    expect(renderStallSnapshot().stalls).toEqual([]);
    stop();
    editor.dispatchEvent(new Event("input", { bubbles: true }));
    expect(next).toBeUndefined();
  });
  it("registers no PerformanceObserver until typing and disconnects when typing goes idle", () => {
    const observed: PerformanceObserverInit[] = [];
    let live = 0;
    class Observer {
      static supportedEntryTypes = ["long-animation-frame", "event"];
      constructor() { live++; }
      observe(options: PerformanceObserverInit) { observed.push(options); }
      disconnect() { live--; }
    }
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    vi.stubGlobal("PerformanceObserver", Observer);
    vi.stubGlobal("requestAnimationFrame", () => 1);
    vi.stubGlobal("cancelAnimationFrame", () => {});
    vi.spyOn(document, "hidden", "get").mockReturnValue(false);
    vi.spyOn(performance, "now").mockReturnValue(100);
    const stop = installRenderStallMonitor();
    expect(live).toBe(0); // scrolling and loading pay nothing (hosted bench)
    expect(renderStallSnapshot().supported).toEqual(["long-animation-frame", "event"]);
    const editor = document.createElement("textarea");
    document.body.append(editor);
    editor.dispatchEvent(new Event("keydown", { bubbles: true }));
    expect(live).toBe(2);
    expect(observed.every(options => (options as { buffered?: boolean }).buffered)).toBe(true);
    vi.mocked(performance.now).mockReturnValue(1500);
    editor.dispatchEvent(new Event("input", { bubbles: true }));
    vi.advanceTimersByTime(1999);
    expect(live).toBe(2); // the later key extends the window
    vi.mocked(performance.now).mockReturnValue(3500);
    vi.advanceTimersByTime(1500);
    expect(live).toBe(0); // two seconds without input
    editor.dispatchEvent(new Event("input", { bubbles: true }));
    expect(live).toBe(2);
    stop();
    expect(live).toBe(0);
  });
  it("exports bounded LoAF attribution and Event Timing where supported", () => {
    const callbacks: ((list: { getEntries: () => unknown[] }) => void)[] = [];
    class Observer {
      static supportedEntryTypes = ["long-animation-frame", "event"];
      constructor(callback: (list: { getEntries: () => unknown[] }) => void) { callbacks.push(callback); }
      observe() {}
      disconnect() {}
    }
    vi.stubGlobal("PerformanceObserver", Observer);
    vi.stubGlobal("requestAnimationFrame", () => 1);
    vi.stubGlobal("cancelAnimationFrame", () => {});
    vi.spyOn(document, "hidden", "get").mockReturnValue(false);
    vi.spyOn(performance, "now").mockReturnValue(100);
    const stop = installRenderStallMonitor();
    const editor = document.createElement("textarea");
    document.body.append(editor);
    editor.dispatchEvent(new Event("input", { bubbles: true }));
    callbacks[0]({ getEntries: () => [{ startTime: 100, duration: 250, blockingDuration: 200, scripts: Array.from({ length: 100 }, () => ({ sourceURL: "http://tauri.localhost/assets/index-ab12.js", sourceCharPosition: 42, duration: 180 })) }] });
    callbacks[1]({ getEntries: () => [{ startTime: 110, duration: 150, name: "private-key", target: editor }] });
    const snapshot = renderStallSnapshot();
    expect(snapshot.stalls[0].scripts).toHaveLength(8);
    expect(snapshot.stalls[0].scripts?.[0]).toEqual({ source: "index-ab12.js", position: 42, duration: 180 });
    expect(snapshot.stalls[1].kind).toBe("event");
    expect(JSON.stringify(snapshot)).not.toContain("private-key");
    stop();
  });
});
