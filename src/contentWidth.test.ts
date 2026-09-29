// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { backend } from "./backend";
import {
  MAX_CONTENT_WIDTH,
  MIN_CONTENT_WIDTH,
  applyContentWidths,
  changeStandardContentWidth,
  changeWideContentWidth,
  initContentWidths,
  normalizeContentWidth,
  resetStandardContentWidth,
  standardContentWidth,
  wideContentWidth,
} from "./contentWidth";

const write = () => vi.spyOn(backend(), "setAppString").mockResolvedValue();

afterEach(() => {
  vi.restoreAllMocks();
  write();
  resetStandardContentWidth();
  changeWideContentWidth(null);
  vi.restoreAllMocks();
  document.documentElement.style.removeProperty("--tine-main-content-max-width");
  document.documentElement.style.removeProperty("--tine-wide-content-max-width");
});

describe("device-local content widths", () => {
  it("clamps explicit widths and persists the values", () => {
    const saved = write();
    changeStandardContentWidth(42);
    changeWideContentWidth(100_000);

    expect(saved).toHaveBeenCalledWith("content_width_standard", String(MIN_CONTENT_WIDTH));
    expect(saved).toHaveBeenCalledWith("content_width_wide", String(MAX_CONTENT_WIDTH));
    expect(document.documentElement.style.getPropertyValue("--tine-main-content-max-width")).toBe(`${MIN_CONTENT_WIDTH}px`);
    expect(document.documentElement.style.getPropertyValue("--tine-wide-content-max-width")).toBe(`${MAX_CONTENT_WIDTH}px`);
  });

  it("uses the theme and fill-pane defaults after reset", () => {
    const saved = write();
    changeStandardContentWidth(960);
    changeWideContentWidth(1440);
    resetStandardContentWidth();
    changeWideContentWidth(null);
    applyContentWidths();

    expect(saved).toHaveBeenLastCalledWith("content_width_wide", "");
    expect(saved).toHaveBeenCalledWith("content_width_standard", "");
    expect(document.documentElement.style.getPropertyValue("--tine-main-content-max-width")).toBe("");
    expect(document.documentElement.style.getPropertyValue("--tine-wide-content-max-width")).toBe("");
  });

  it("rounds finite values and rejects non-finite input at the public boundary", () => {
    const saved = write();
    expect(normalizeContentWidth(915.6)).toBe(916);
    changeStandardContentWidth(960);
    changeStandardContentWidth(Number.NaN);
    expect(standardContentWidth()).toBe(960);
    expect(saved).toHaveBeenCalledTimes(1);
  });

  it("hydrates remembered overrides at startup and ignores an out-of-range stored value", async () => {
    vi.spyOn(backend(), "getAppString").mockImplementation(async (key) =>
      key === "content_width_standard" ? "960" : "99999999");
    await initContentWidths();
    expect(standardContentWidth()).toBe(960);
    expect(wideContentWidth()).toBeNull();
    expect(document.documentElement.style.getPropertyValue("--tine-main-content-max-width")).toBe("960px");
  });

  it("a change made while the read is pending wins over the delayed read", async () => {
    write();
    const pending: Array<(value: string) => void> = [];
    vi.spyOn(backend(), "getAppString").mockImplementation(() => new Promise((resolve) => { pending.push(resolve); }));
    const started = initContentWidths();
    changeStandardContentWidth(1000);
    for (const release of pending) release("700");
    await started;
    expect(standardContentWidth()).toBe(1000);
  });
});
