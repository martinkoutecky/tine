import { afterEach, expect, it, vi } from "vitest";

// GH #343: uncaught errors and main-thread stalls reach the privacy-safe
// recorder as a fixed kind and numbers even when opt-in debug logging is off;
// the message itself never does.
const diagnosticFrontendEvent = vi.fn(async () => {});
const debugLog = vi.fn(async () => {});
let previousExitUnclean = false;
let debugEnabled = false;
vi.mock("./backend", () => ({
  backend: () => ({ diagnosticFrontendEvent, debugLog, debugInfo: async () => ({ enabled: debugEnabled, path: "/log", previousExitUnclean }) }),
}));
const pushToast = vi.fn();
vi.mock("./toasts", () => ({ pushToast, pushToastUnique: vi.fn() }));
const openSettings = vi.fn();
vi.mock("./ui", () => ({ openSettings }));

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

// og ADR 0058 (master 271885b2): the previous run's session marker survived,
// so it ended without an orderly exit; the user is pointed at the report.
it("offers the diagnostic report once when the previous run did not close cleanly", async () => {
  const { initDebug, resetDebugForTests } = await import("./debug");
  for (const unclean of [false, true]) {
    resetDebugForTests();
    pushToast.mockClear();
    previousExitUnclean = unclean;
    fakeWindow();
    await initDebug();
    const warned = pushToast.mock.calls.filter(([text]) => String(text).includes("did not close cleanly"));
    expect(warned).toHaveLength(unclean ? 1 : 0);
    if (unclean) {
      expect(warned[0][2]).toMatchObject({ sticky: true, action: { label: "Diagnostics" } });
      warned[0][2].action.run();
      await vi.waitFor(() => expect(openSettings).toHaveBeenCalledWith("diagnostics"));
    }
  }
  previousExitUnclean = false;
});

// GH #446 (master 82b64dcb): the boot line names the identity the build
// injected next to the UA, because an iPad's UA says Mac.
it("logs the build-injected platform beside the user agent in the boot line", async () => {
  vi.resetModules();
  vi.stubGlobal("__TINE_PLATFORM__", "ios");
  vi.stubGlobal("navigator", { userAgent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15" });
  debugEnabled = true;
  try {
    const { initDebug, resetDebugForTests } = await import("./debug");
    resetDebugForTests();
    fakeWindow();
    await initDebug();
    await vi.waitFor(() => expect(debugLog).toHaveBeenCalledWith(expect.stringContaining("frontend booted")));
    expect(debugLog).toHaveBeenCalledWith(
      "frontend booted (platform=ios ua=Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15)",
    );
  } finally {
    debugEnabled = false;
  }
});
