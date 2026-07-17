import { afterEach, describe, expect, it, vi } from "vitest";

async function loadNativeChrome(active: boolean, saved: boolean) {
  vi.resetModules();
  globalThis.__TINE_NATIVE_FRAME__ = active;
  const getAppBool = vi.fn(async () => saved);
  const setAppBool = vi.fn(async () => {});
  vi.doMock("./backend", () => ({ backend: () => ({ getAppBool, setAppBool }) }));
  return { module: await import("./nativeChrome"), getAppBool, setAppBool };
}

describe("native window frame preference", () => {
  afterEach(() => {
    delete globalThis.__TINE_NATIVE_FRAME__;
    vi.restoreAllMocks();
  });

  it("keeps the applied frame separate from a newly saved restart preference", async () => {
    const { module, setAppBool } = await loadNativeChrome(false, false);
    await module.initNativeChrome();

    expect(module.osDrawsWindowControls()).toBe(false);
    await module.setNativeFrame(true);

    expect(setAppBool).toHaveBeenCalledWith(module.KEY_NATIVE_FRAME, true);
    expect(module.nativeFrameEnabled()).toBe(true);
    expect(module.osDrawsWindowControls()).toBe(false);
  });

  it("hides custom controls immediately when Rust constructed a native frame", async () => {
    const { module } = await loadNativeChrome(true, true);

    expect(module.osDrawsWindowControls()).toBe(true);
    await module.initNativeChrome();
    expect(module.nativeFrameEnabled()).toBe(true);
  });

  it("does not change the switch when persisting the preference fails", async () => {
    vi.resetModules();
    globalThis.__TINE_NATIVE_FRAME__ = false;
    vi.doMock("./backend", () => ({
      backend: () => ({
        getAppBool: vi.fn(async () => false),
        setAppBool: vi.fn(async () => { throw new Error("disk full"); }),
      }),
    }));
    const module = await import("./nativeChrome");

    await expect(module.setNativeFrame(true)).rejects.toThrow("disk full");
    expect(module.nativeFrameEnabled()).toBe(false);
    expect(module.osDrawsWindowControls()).toBe(false);
  });
});
