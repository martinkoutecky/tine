import { afterEach, expect, it, vi } from "vitest";

// GH #343: uncaught errors and main-thread stalls reach the privacy-safe
// recorder as a fixed kind and numbers even when opt-in debug logging is off;
// the message itself never does.
const diagnosticFrontendEvent = vi.fn(async () => {});
const debugLog = vi.fn(async () => {});
vi.mock("./backend", () => ({
  backend: () => ({ diagnosticFrontendEvent, debugLog, debugInfo: async () => ({ enabled: false, path: "" }) }),
}));
vi.mock("./toasts", () => ({ pushToast: vi.fn(), pushToastUnique: vi.fn() }));

afterEach(() => { vi.useRealTimers(); vi.unstubAllGlobals(); diagnosticFrontendEvent.mockClear(); debugLog.mockClear(); });

function fakeWindow() {
  const listeners = new Map<string, (event: unknown) => void>();
  vi.stubGlobal("window", {
    addEventListener: (type: string, listener: (event: unknown) => void) => listeners.set(type, listener),
    setInterval: (fn: () => void, ms: number) => setInterval(fn, ms),
  });
  return listeners;
}

it("records uncaught errors and rejections as fixed kinds without their text, with debug logging off", async () => {
  const { initDebug, resetDebugForTests } = await import("./debug");
  resetDebugForTests();
  const listeners = fakeWindow();
  await initDebug();
  listeners.get("error")!({ message: "secret page title", filename: "/home/someone/x.js", lineno: 12, colno: 3 });
  listeners.get("unhandledrejection")!({ reason: "secret reason" });
  await vi.waitFor(() => expect(diagnosticFrontendEvent).toHaveBeenCalledTimes(2));
  expect(diagnosticFrontendEvent.mock.calls).toEqual([
    ["uncaught_error", { line: 12, column: 3 }],
    ["unhandled_rejection", undefined],
  ]);
  expect(JSON.stringify(diagnosticFrontendEvent.mock.calls)).not.toMatch(/secret|home/);
  expect(debugLog).not.toHaveBeenCalled();
});

it("records a main-thread stall of five seconds or more as a heartbeat delay", async () => {
  vi.useFakeTimers();
  let now = 0;
  vi.spyOn(performance, "now").mockImplementation(() => now);
  const { initDebug, resetDebugForTests, HEARTBEAT_REPORT_MS } = await import("./debug");
  resetDebugForTests();
  fakeWindow();
  await initDebug();
  now += 2_000;
  vi.advanceTimersByTime(2_000);
  expect(diagnosticFrontendEvent).not.toHaveBeenCalled();
  now += 2_000 + HEARTBEAT_REPORT_MS; // a blocked main thread: the tick lands late
  vi.advanceTimersByTime(2_000);
  await vi.waitFor(() => expect(diagnosticFrontendEvent).toHaveBeenCalledOnce());
  expect(diagnosticFrontendEvent.mock.calls[0]).toEqual(["heartbeat_delay", { delayMs: HEARTBEAT_REPORT_MS }]);
  vi.restoreAllMocks();
});
