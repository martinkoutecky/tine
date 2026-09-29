import { afterEach, describe, expect, it, vi } from "vitest";
import { appNow, localDayKey, setBackendClock } from "./journal";

// GH #607: a WebView whose bundled zone rules put the wall clock an hour ahead
// of the backend's must adopt the backend's calendar day.
describe("appNow follows the backend clock (GH #607)", () => {
  afterEach(() => {
    const now = Date.now();
    setBackendClock({ offset_minutes: -new Date(now).getTimezoneOffset(), unix_ms: now });
    vi.useRealTimers();
  });

  it("is the plain clock when the backend agrees with the WebView", () => {
    const now = Date.now();
    setBackendClock({ offset_minutes: -new Date(now).getTimezoneOffset(), unix_ms: now });
    expect(Math.abs(appNow().getTime() - Date.now())).toBeLessThan(1000);
  });

  it("reports the backend's day when the WebView's zone data is an hour ahead", () => {
    // The WebView reads 00:33 on the 28th; the backend's (current) rules put
    // the same instant at 23:33 on the 27th.
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2026, 8, 28, 0, 33));
    const now = Date.now();
    const browserOffset = -new Date(now).getTimezoneOffset();
    setBackendClock({ offset_minutes: browserOffset - 60, unix_ms: now });
    expect(localDayKey()).toBe(20260927);
    expect(appNow().getHours()).toBe(23);
  });

  it("keeps the WebView clock when the backend disagrees by more than stale rules can", () => {
    const now = Date.now();
    const browserOffset = -new Date(now).getTimezoneOffset();
    setBackendClock({ offset_minutes: browserOffset + 8 * 60, unix_ms: now });
    expect(Math.abs(appNow().getTime() - Date.now())).toBeLessThan(1000);
  });
});
