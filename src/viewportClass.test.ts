// D-20261018-W1: layout follows room and input, never the device's name.
import { afterEach, describe, expect, it, vi } from "vitest";
import { createRoot } from "solid-js";
import { isSinglePaneShell, isTabletViewport, tabStripFits } from "./viewportClass";
import { touchCapable, touchGestures, touchPrimary } from "./inputCapability";

function stubMedia(matches: Record<string, boolean>) {
  vi.stubGlobal("matchMedia", (query: string) => ({
    matches: matches[query] ?? false,
    addEventListener: () => {},
    removeEventListener: () => {},
  }));
}
function stubViewport(width: number, height: number) {
  vi.stubGlobal("innerWidth", width);
  vi.stubGlobal("innerHeight", height);
}

afterEach(() => {
  vi.unstubAllGlobals();
  delete globalThis.__TINE_E2E_TOUCH_GESTURES__;
});

describe("isTabletViewport", () => {
  it("is the shorter edge having room for two documents", () => {
    expect(isTabletViewport(820, 1180)).toBe(true); // iPad portrait
    expect(isTabletViewport(1180, 820)).toBe(true); // iPad landscape
    expect(isTabletViewport(744, 1133)).toBe(true); // iPad mini portrait
    expect(isTabletViewport(430, 932)).toBe(false); // iPhone portrait
    expect(isTabletViewport(932, 430)).toBe(false); // iPhone landscape
  });
});

describe("isSinglePaneShell", () => {
  it("is a touch-first screen too small for two documents", () => {
    stubMedia({ "(pointer: coarse)": true });
    stubViewport(390, 844);
    expect(isSinglePaneShell()).toBe(true);
    stubViewport(844, 390); // landscape phone
    expect(isSinglePaneShell()).toBe(true);
  });
  it("keeps splits on a tablet", () => {
    stubMedia({ "(pointer: coarse)": true });
    stubViewport(1180, 820);
    expect(isSinglePaneShell()).toBe(false);
  });
  it("keeps splits for a mouse user at any window size (a saved layout survives a narrow window)", () => {
    stubMedia({ "(pointer: coarse)": false });
    stubViewport(480, 600);
    expect(isSinglePaneShell()).toBe(false);
  });
  it("never claims a single-pane shell without a matchMedia (SSR, tests)", () => {
    vi.stubGlobal("matchMedia", undefined);
    stubViewport(390, 844);
    expect(isSinglePaneShell()).toBe(false);
  });
});

describe("tabStripFits", () => {
  it("follows the drawer-width classifier, not the platform", () => {
    // mobileDrawers computes its initial answer at module load from
    // matchMedia("(max-width: 639px)"); in this jsdom run it is wide.
    expect(tabStripFits()).toBe(true);
  });
});

describe("input capability", () => {
  it("asks the media features, per call", () => {
    stubMedia({ "(pointer: coarse)": true, "(any-pointer: coarse)": true });
    expect(touchPrimary()).toBe(true);
    expect(touchCapable()).toBe(true);
    stubMedia({ "(pointer: coarse)": false, "(any-pointer: coarse)": true });
    expect(touchPrimary()).toBe(false); // a touchscreen laptop
    expect(touchCapable()).toBe(true);
    expect(touchGestures()).toBe(true);
    stubMedia({});
    expect(touchGestures()).toBe(false);
  });
  it("lets only the E2E harness force touch gestures on a desktop process", () => {
    stubMedia({});
    globalThis.__TINE_E2E_TOUCH_GESTURES__ = "ios";
    expect(touchGestures()).toBe(true);
    expect(touchPrimary()).toBe(false);
  });
  it("re-runs a computation when the answer changes", async () => {
    vi.resetModules();
    let fire: () => void = () => {};
    let coarse = false;
    vi.stubGlobal("matchMedia", (query: string) => ({
      get matches() { return query === "(pointer: coarse)" && coarse; },
      addEventListener: (_: string, listener: () => void) => { fire = listener; },
      removeEventListener: () => {},
    }));
    const { touchPrimary: fresh } = await import("./inputCapability");
    const { createEffect } = await import("solid-js");
    const seen: boolean[] = [];
    const dispose = createRoot((d) => { createEffect(() => { seen.push(fresh()); }); return d; });
    expect(seen).toEqual([false]);
    coarse = true;
    fire();
    expect(seen).toEqual([false, true]);
    dispose();
  });
});
