import { afterEach, describe, expect, it, vi } from "vitest";

type Platform = "desktop" | "android" | "ios";

async function loadUpdate(opts: {
  tauri?: boolean;
  platform?: Platform;
  platformReject?: boolean;
  version?: string;
}) {
  vi.resetModules();
  const isTauriMock = vi.fn(() => opts.tauri ?? true);
  const platformKindMock = vi.fn(async (): Promise<Platform> => {
    if (opts.platformReject) throw new Error("platform unavailable");
    return opts.platform ?? "desktop";
  });
  const openExternalMock = vi.fn(async () => {});
  const pushToastMock = vi.fn(() => 1);
  const dismissToastMock = vi.fn();
  const getVersionMock = vi.fn(async () => opts.version ?? "0.5.3");
  const updaterCheckMock = vi.fn(async () => null);

  vi.doMock("./backend", () => ({
    isTauri: isTauriMock,
    backend: () => ({ openExternal: openExternalMock }),
  }));
  vi.doMock("./platform", () => ({ platformKind: platformKindMock }));
  vi.doMock("./ui", () => ({
    pushToast: pushToastMock,
    dismissToast: dismissToastMock,
  }));
  vi.doMock("@tauri-apps/api/app", () => ({ getVersion: getVersionMock }));
  vi.doMock("@tauri-apps/plugin-updater", () => ({ check: updaterCheckMock }));
  vi.doMock("@tauri-apps/plugin-process", () => ({ relaunch: vi.fn(async () => {}) }));

  const update = await import("./update");
  return {
    update,
    platformKindMock,
    getVersionMock,
    updaterCheckMock,
    openExternalMock,
    pushToastMock,
  };
}

function mockLatest(tag: string, ok = true) {
  const fetchMock = vi.fn(async () => ({
    ok,
    json: async () => ({ tag_name: tag }),
  }));
  vi.stubGlobal("fetch", fetchMock);
  return fetchMock;
}

describe("update checks", () => {
  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it.each(["android", "ios"] as const)("never checks or offers self-update on %s", async (platform) => {
    const fetchMock = mockLatest("v0.6.0");
    const { update, platformKindMock, getVersionMock, updaterCheckMock, openExternalMock, pushToastMock } =
      await loadUpdate({ platform });

    await update.checkForUpdate();
    await expect(update.checkForUpdateNow()).resolves.toEqual({ kind: "unavailable" });

    expect(platformKindMock).toHaveBeenCalled();
    expect(getVersionMock).not.toHaveBeenCalled();
    expect(fetchMock).not.toHaveBeenCalled();
    expect(updaterCheckMock).not.toHaveBeenCalled();
    expect(openExternalMock).not.toHaveBeenCalled();
    expect(pushToastMock).not.toHaveBeenCalled();
  });

  it("fails closed when native platform detection fails", async () => {
    const fetchMock = mockLatest("v0.6.0");
    const { update, getVersionMock, updaterCheckMock, pushToastMock } = await loadUpdate({
      platformReject: true,
    });

    await update.checkForUpdate();
    await expect(update.checkForUpdateNow()).resolves.toEqual({ kind: "unavailable" });

    expect(getVersionMock).not.toHaveBeenCalled();
    expect(fetchMock).not.toHaveBeenCalled();
    expect(updaterCheckMock).not.toHaveBeenCalled();
    expect(pushToastMock).not.toHaveBeenCalled();
  });

  it("keeps the startup update toast on desktop Tauri", async () => {
    mockLatest("v0.6.0");
    const { update, pushToastMock } = await loadUpdate({ platform: "desktop", version: "0.5.3" });

    await update.checkForUpdate();

    expect(pushToastMock).toHaveBeenCalledWith(
      "Tine 0.6.0 is available — you're on 0.5.3.",
      "info",
      expect.objectContaining({
        sticky: true,
        action: expect.objectContaining({ label: "Download" }),
      })
    );
  });

  it("keeps the manual current-version result on desktop Tauri", async () => {
    mockLatest("v0.5.3");
    const { update } = await loadUpdate({ platform: "desktop", version: "0.5.3" });

    await expect(update.checkForUpdateNow()).resolves.toEqual({ kind: "current", version: "0.5.3" });
  });

  it("keeps browser/dev checks inert without probing the native platform", async () => {
    const fetchMock = mockLatest("v0.6.0");
    const { update, platformKindMock } = await loadUpdate({ tauri: false });

    await update.checkForUpdate();
    await expect(update.checkForUpdateNow()).resolves.toEqual({ kind: "unavailable" });

    expect(platformKindMock).not.toHaveBeenCalled();
    expect(fetchMock).not.toHaveBeenCalled();
  });
});
