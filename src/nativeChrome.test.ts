import { afterEach, describe, expect, it, vi } from "vitest";

async function loadNativeChrome(active: boolean, saved: boolean) {
  vi.resetModules();
  globalThis.__TINE_NATIVE_FRAME__ = active;
  const getAppBool = vi.fn(async () => saved);
  const setAppBool = vi.fn(async () => {});
  vi.doMock("./backend", () => ({ backend: () => ({ getAppBool, setAppBool }) }));
  return { module: await import("./nativeChrome"), getAppBool, setAppBool };
}

// The user agent an iPadOS 13+ stock WKWebView actually serves. It names
// neither "iPad" nor any mobile token; it claims to be a Mac.
const IPAD_DESKTOP_CLASS_UA =
  "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15";

async function loadPlatform(platform: "android" | "ios" | "desktop" | undefined, ua: string) {
  vi.resetModules();
  if (platform === undefined) delete globalThis.__TINE_PLATFORM__;
  else globalThis.__TINE_PLATFORM__ = platform;
  vi.stubGlobal("navigator", { userAgent: ua, platform: /Mac/.test(ua) ? "MacIntel" : "" });
  vi.doMock("./backend", () => ({ backend: () => ({ getAppBool: vi.fn(), setAppBool: vi.fn() }) }));
  return await import("./nativeChrome");
}

describe("platform identity (GH #446)", () => {
  afterEach(() => {
    delete globalThis.__TINE_PLATFORM__;
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("believes the build, not the desktop-class user agent iPadOS serves", async () => {
    const m = await loadPlatform("ios", IPAD_DESKTOP_CLASS_UA);
    expect(m.platformKind).toBe("ios");
    expect(m.isMobilePlatform).toBe(true);
    expect(m.isMac).toBe(false);
    expect(m.osDrawsWindowControls()).toBe(true);
  });

  it("still reports a real Mac as a Mac desktop", async () => {
    const m = await loadPlatform("desktop", IPAD_DESKTOP_CLASS_UA);
    expect(m.platformKind).toBe("desktop");
    expect(m.isMobilePlatform).toBe(false);
    expect(m.isMac).toBe(true);
  });

  it("falls back to the user agent when Tauri never injected an identity", async () => {
    const m = await loadPlatform(
      undefined,
      "Mozilla/5.0 (Linux; Android 14) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120 Mobile Safari/537.36",
    );
    expect(m.platformKind).toBe("android");
    expect(m.isMobilePlatform).toBe(true);
  });

  it("keeps the split-pane shell on a tablet and the single-pane shell on a phone", async () => {
    const m = await loadPlatform("ios", IPAD_DESKTOP_CLASS_UA);
    expect(m.isTabletViewport(820, 1180)).toBe(true); // iPad portrait
    expect(m.isTabletViewport(1180, 820)).toBe(true); // iPad landscape
    expect(m.isTabletViewport(744, 1133)).toBe(true); // iPad mini portrait
    expect(m.isTabletViewport(430, 932)).toBe(false); // iPhone portrait
    expect(m.isTabletViewport(932, 430)).toBe(false); // iPhone landscape
  });

  it("never claims a single-pane shell on desktop", async () => {
    const m = await loadPlatform("desktop", "Mozilla/5.0 (X11; Linux x86_64)");
    expect(m.isSinglePaneShell()).toBe(false);
  });
});

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

  it("I-20: writes reach the backend in call order and the switch shows the newest choice", async () => {
    vi.resetModules();
    globalThis.__TINE_NATIVE_FRAME__ = false;
    const started: boolean[] = [];
    const finish: Array<() => void> = [];
    vi.doMock("./backend", () => ({
      backend: () => ({
        getAppBool: vi.fn(async () => false),
        setAppBool: vi.fn((_key: string, on: boolean) => new Promise<void>((resolve) => { started.push(on); finish.push(resolve); })),
      }),
    }));
    const module = await import("./nativeChrome");
    const first = module.setNativeFrame(true);
    const second = module.setNativeFrame(false);
    await Promise.resolve();
    await Promise.resolve();
    expect(started).toEqual([true]);
    finish[0]();
    await first;
    // The first call is no longer the newest choice, so it does not repaint.
    expect(module.nativeFrameEnabled()).toBe(false);
    for (let i = 0; i < 5; i += 1) await Promise.resolve();
    expect(started).toEqual([true, false]);
    finish[1]();
    await second;
    expect(module.nativeFrameEnabled()).toBe(false);
  });

  it("I-20: a startup read that lands after the user's choice does not overwrite it", async () => {
    vi.resetModules();
    globalThis.__TINE_NATIVE_FRAME__ = false;
    let land!: (saved: boolean) => void;
    vi.doMock("./backend", () => ({
      backend: () => ({
        getAppBool: vi.fn(() => new Promise<boolean>((resolve) => { land = resolve; })),
        setAppBool: vi.fn(async () => {}),
      }),
    }));
    const module = await import("./nativeChrome");
    const init = module.initNativeChrome();
    await module.setNativeFrame(true);
    land(false);
    await init;
    expect(module.nativeFrameEnabled()).toBe(true);
  });
});
