// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { clearRenderStalls, createStallBuffer, installRenderStallMonitor, renderStallSnapshot, safeScriptSource } from "./renderStalls";

afterEach(() => { vi.restoreAllMocks(); vi.unstubAllGlobals(); clearRenderStalls(); document.body.replaceChildren(); });

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
  it("samples typing across editor surfaces, stops when idle and discards background gaps", () => {
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
    next!(250);
    expect(renderStallSnapshot().stalls.some(stall => stall.kind === "frame" && stall.duration === 150)).toBe(true);
    const count = request.mock.calls.length;
    next!(2200);
    expect(request).toHaveBeenCalledTimes(count); // idle: no successor
    clearRenderStalls();
    editor.dispatchEvent(new Event("input", { bubbles: true }));
    document.dispatchEvent(new Event("visibilitychange"));
    expect(next).toBeUndefined();
    expect(renderStallSnapshot().stalls).toEqual([]);
    stop();
    editor.dispatchEvent(new Event("input", { bubbles: true }));
    expect(next).toBeUndefined();
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
