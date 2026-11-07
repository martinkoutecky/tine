import { afterEach, expect, it, vi } from "vitest";

afterEach(() => { vi.unstubAllGlobals(); vi.resetModules(); });

it.each([
  ["Linux x86_64", "desktop", "mod+q"],
  ["Win32", "desktop", "mod+q"],
  ["MacIntel", "desktop", ""],
  ["MacIntel", "ios", ""],
  ["Linux armv8", "android", ""],
])("Quit's default on %s / %s is %s", async (platform, host, binding) => {
  vi.resetModules();
  vi.stubGlobal("navigator", { platform, userAgent: "" });
  vi.stubGlobal("__TINE_PLATFORM__", host);
  const { commandDefaults } = await import("./keybindings");
  expect(commandDefaults().find(command => command.id === "app/quit")?.binding).toBe(binding);
});
