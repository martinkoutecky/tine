import { afterEach, describe, expect, it } from "vitest";
import { clearTransientLayersForTest, dismissTopTransient, registerTransientLayer } from "./transientLayers";
import { installMobileDrawerMode, isMobileDrawerViewport, mobileDrawerMatches, mobileDrawerMode } from "./mobileDrawers";

afterEach(clearTransientLayersForTest);

describe("mobile drawer policy", () => {
  it("uses only the exact 639/640 viewport boundary", () => {
    expect(isMobileDrawerViewport(639)).toBe(true);
    expect(isMobileDrawerViewport(640)).toBe(false);
    expect(isMobileDrawerViewport(1200)).toBe(false);
    expect(mobileDrawerMatches(true)).toBe(true);
    expect(mobileDrawerMatches(false)).toBe(false);
  });

  it("reacts to the width-only media query and removes its listener", () => {
    const descriptor = Object.getOwnPropertyDescriptor(globalThis, "matchMedia");
    let matches = true;
    let listener: (() => void) | undefined;
    const query = {
      get matches() { return matches; },
      media: "(max-width: 639px)",
      addEventListener(type: string, value: () => void) {
        if (type === "change") listener = value;
      },
      removeEventListener(type: string, value: () => void) {
        if (type === "change" && listener === value) listener = undefined;
      },
    } as unknown as MediaQueryList;
    Object.defineProperty(globalThis, "matchMedia", {
      configurable: true,
      value: (media: string) => {
        expect(media).toBe("(max-width: 639px)");
        return query;
      },
    });

    try {
      const dispose = installMobileDrawerMode();
      expect(mobileDrawerMode()).toBe(true);
      matches = false;
      listener?.();
      expect(mobileDrawerMode()).toBe(false);
      dispose();
      expect(listener).toBeUndefined();
    } finally {
      if (descriptor) Object.defineProperty(globalThis, "matchMedia", descriptor);
      else delete (globalThis as { matchMedia?: typeof matchMedia }).matchMedia;
    }
  });

  it("dismisses only one newest transient and does not fall through a stale layer", () => {
    const calls: string[] = [];
    registerTransientLayer({ id: "drawer-menu", dismiss: () => { calls.push("menu"); return true; } });
    registerTransientLayer({ id: "stale", dismiss: () => { calls.push("stale"); return false; } });
    expect(dismissTopTransient("escape")).toBe(true);
    expect(calls).toEqual(["stale"]);
    expect(dismissTopTransient("escape")).toBe(true);
    expect(calls).toEqual(["stale", "menu"]);
  });

  it("reactivating a layer makes it topmost deterministically", () => {
    const calls: string[] = [];
    const unregA = registerTransientLayer({ id: "a", dismiss: () => { calls.push("a"); return true; } });
    registerTransientLayer({ id: "b", dismiss: () => { calls.push("b"); return true; } });
    unregA();
    registerTransientLayer({ id: "a", dismiss: () => { calls.push("a"); return true; } });
    dismissTopTransient("back");
    expect(calls).toEqual(["a"]);
  });

  it("does not let an old same-id disposer remove its replacement", () => {
    const calls: string[] = [];
    const stale = registerTransientLayer({ id: "shared", dismiss: () => { calls.push("old"); return true; } });
    registerTransientLayer({ id: "shared", dismiss: () => { calls.push("new"); return true; } });
    stale();
    expect(dismissTopTransient("escape")).toBe(true);
    expect(calls).toEqual(["new"]);
  });
});
