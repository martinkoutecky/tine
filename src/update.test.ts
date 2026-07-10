import { afterEach, describe, expect, it, vi } from "vitest";

async function loadUpdate(opts: {
  tauri?: boolean;
  mobile?: boolean;
  mobileReject?: boolean;
  version?: string;
}) {
  vi.resetModules();
  const isTauriMock = vi.fn(() => opts.tauri ?? true);
  const isMobileMock = vi.fn(async () => {
    if (opts.mobileReject) throw new Error("platform unavailable");
    return opts.mobile ?? false;
  });
  const openExternalMock = vi.fn(async () => {});
  const pushToastMock = vi.fn(() => 1);
  const dismissToastMock = vi.fn();
  const getVersionMock = vi.fn(async () => opts.version ?? "0.5.0");
  const checkMock = vi.fn(async () => null);
  const relaunchMock = vi.fn(async () => {});

  vi.doMock("./backend", () => ({
    isTauri: isTauriMock,
    backend: () => ({ openExternal: openExternalMock }),
  }));
  vi.doMock("./platform", () => ({ isMobile: isMobileMock }));
  vi.doMock("./ui", () => ({
    pushToast: pushToastMock,
    dismissToast: dismissToastMock,
  }));
  vi.doMock("@tauri-apps/api/app", () => ({ getVersion: getVersionMock }));
  vi.doMock("@tauri-apps/plugin-updater", () => ({ check: checkMock }));
  vi.doMock("@tauri-apps/plugin-process", () => ({ relaunch: relaunchMock }));

  const update = await import("./update");
  return {
    update,
    isMobileMock,
    getVersionMock,
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

async function expectMobileUnavailable() {
  const fetchMock = mockLatest("v0.6.0");
  const { update, isMobileMock, getVersionMock, pushToastMock } = await loadUpdate({ mobile: true });

  await update.checkForUpdate();
  expect(await update.checkForUpdateNow()).toEqual({ kind: "unavailable" });

  expect(isMobileMock).toHaveBeenCalled();
  expect(getVersionMock).not.toHaveBeenCalled();
  expect(fetchMock).not.toHaveBeenCalled();
  expect(pushToastMock).not.toHaveBeenCalled();
}

describe("update checks", () => {
  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it("skips startup and manual checks on Android", async () => {
    await expectMobileUnavailable();
  });

  it("skips startup and manual checks on iOS", async () => {
    await expectMobileUnavailable();
  });

  it("keeps the startup toast path on desktop Tauri", async () => {
    mockLatest("v0.6.0");
    const { update, pushToastMock } = await loadUpdate({ mobile: false, version: "0.5.0" });

    await update.checkForUpdate();

    expect(pushToastMock).toHaveBeenCalledWith(
      "Tine 0.6.0 is available — you're on 0.5.0.",
      "info",
      expect.objectContaining({
        sticky: true,
        action: expect.objectContaining({ label: "Download" }),
      })
    );
  });

  it("keeps the manual current-version result on desktop Tauri", async () => {
    mockLatest("v0.5.0");
    const { update } = await loadUpdate({ mobile: false, version: "0.5.0" });

    await expect(update.checkForUpdateNow()).resolves.toEqual({ kind: "current", version: "0.5.0" });
  });

  it("preserves browser/dev unavailability without fetching", async () => {
    const fetchMock = mockLatest("v0.6.0");
    const { update, isMobileMock } = await loadUpdate({ tauri: false, mobile: false });

    await update.checkForUpdate();
    await expect(update.checkForUpdateNow()).resolves.toEqual({ kind: "unavailable" });

    expect(isMobileMock).not.toHaveBeenCalled();
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("falls back to desktop behavior if platform detection fails", async () => {
    mockLatest("v0.5.0");
    const { update, isMobileMock } = await loadUpdate({
      mobileReject: true,
      version: "0.5.0",
    });

    await expect(update.checkForUpdateNow()).resolves.toEqual({ kind: "current", version: "0.5.0" });
    expect(isMobileMock).toHaveBeenCalled();
  });
});
