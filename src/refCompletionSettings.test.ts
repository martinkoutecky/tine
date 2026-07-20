import { afterEach, describe, expect, it, vi } from "vitest";

async function loadSettings(value: boolean, reject = false) {
  vi.resetModules();
  const getAppBool = reject
    ? vi.fn().mockRejectedValue(new Error("settings unavailable"))
    : vi.fn().mockResolvedValue(value);
  vi.doMock("./backend", () => ({ backend: () => ({ getAppBool }) }));
  return { module: await import("./refCompletionSettings"), getAppBool };
}

afterEach(() => vi.restoreAllMocks());

describe("reference completion settings initialization", () => {
  it("publishes the persisted policy only after its startup read completes", async () => {
    const { module, getAppBool } = await loadSettings(false);

    expect(module.refCompletionSettingsReady()).toBe(false);
    await module.initRefCompletionSettings();

    expect(getAppBool).toHaveBeenCalledWith("space_after_ref_completion", true);
    expect(module.spaceAfterRefCompletion()).toBe(false);
    expect(module.refCompletionSettingsReady()).toBe(true);
  });

  it("marks initialization complete when the backend falls back to Tine's default", async () => {
    const { module } = await loadSettings(true, true);

    await module.initRefCompletionSettings();

    expect(module.spaceAfterRefCompletion()).toBe(true);
    expect(module.refCompletionSettingsReady()).toBe(true);
  });
});
