import { afterEach, expect, it, vi } from "vitest";

afterEach(() => { vi.unstubAllGlobals(); vi.resetModules(); });

it("System follows OS changes while manual Light and Dark ignore them", async () => {
  let dark = false;
  const listeners = new Set<() => void>();
  const media = {
    get matches() { return dark; },
    addEventListener: (_: string, listener: () => void) => listeners.add(listener),
    removeEventListener: (_: string, listener: () => void) => listeners.delete(listener),
  };
  vi.stubGlobal("window", { matchMedia: () => media });
  vi.stubGlobal("document", { querySelector: () => null, documentElement: { setAttribute: vi.fn() } });
  vi.stubGlobal("localStorage", { getItem: () => null, setItem: vi.fn() });
  const { appearancePreference, setAppearancePreference, theme } = await import("./ui");
  setAppearancePreference("system");
  expect(appearancePreference()).toBe("system");
  expect(theme()).toBe("light");
  dark = true;
  listeners.forEach((listener) => listener());
  expect(theme()).toBe("dark");
  setAppearancePreference("light");
  expect(listeners.size).toBe(0);
  expect(theme()).toBe("light");
});
