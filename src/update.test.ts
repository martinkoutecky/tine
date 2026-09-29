import { afterEach, describe, expect, it, vi } from "vitest";

type Platform = "desktop" | "android" | "ios";
type ToastCall = [string, string, { sticky?: boolean; action?: { label: string; run: () => void } }?];

function toastCalls(mock: { mock: { calls: unknown[][] } }): ToastCall[] {
  return mock.mock.calls as unknown as ToastCall[];
}

async function loadUpdate(opts: {
  tauri?: boolean;
  platform?: Platform;
  platformReject?: boolean;
  version?: string;
  architecture?: string;
  updaterReject?: Error;
}) {
  vi.resetModules();
  const isTauriMock = vi.fn(() => opts.tauri ?? true);
  const platformKindMock = vi.fn(async (): Promise<Platform> => {
    if (opts.platformReject) throw new Error("platform unavailable");
    return opts.platform ?? "desktop";
  });
  const openExternalMock = vi.fn(async () => {});
  let nextToastId = 40;
  const pushToastMock = vi.fn(() => ++nextToastId);
  const dismissToastMock = vi.fn();
  const openSettingsMock = vi.fn();
  const diagnosticFrontendEventMock = vi.fn(async () => {});
  const appArchitectureMock = vi.fn(async () => opts.architecture ?? "x86_64");
  const getVersionMock = vi.fn(async () => opts.version ?? "0.5.3");
  const updaterCheckMock = opts.updaterReject
    ? vi.fn(async () => { throw opts.updaterReject; })
    : vi.fn(async () => null);

  vi.doMock("./backend", () => ({
    isTauri: isTauriMock,
    backend: () => ({
      openExternal: openExternalMock,
      appArchitecture: appArchitectureMock,
      diagnosticFrontendEvent: diagnosticFrontendEventMock,
    }),
  }));
  vi.doMock("./platform", () => ({ platformKind: platformKindMock }));
  vi.doMock("./toasts", () => ({
    pushToast: pushToastMock,
    pushToastUnique: pushToastMock,
    dismissToast: dismissToastMock,
  }));
  vi.doMock("./ui", () => ({ openSettings: openSettingsMock }));
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
    dismissToastMock,
    openSettingsMock,
    diagnosticFrontendEventMock,
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
        action: expect.objectContaining({ label: "Install update" }),
      })
    );
  });

  it("keeps one visible offer when the startup and manual checks find the same update", async () => {
    mockLatest("v0.6.0");
    const { update, pushToastMock, dismissToastMock } = await loadUpdate({ platform: "desktop", version: "0.5.3" });

    await update.checkForUpdate();
    await expect(update.checkForUpdateNow()).resolves.toEqual({
      kind: "available",
      version: "0.6.0",
      current: "0.5.3",
    });

    expect(pushToastMock).toHaveBeenCalledTimes(2);
    expect(dismissToastMock).toHaveBeenCalledOnce();
    expect(dismissToastMock).toHaveBeenCalledWith(41);
  });

  it("lets only the newest of two concurrent offers publish the sticky toast", async () => {
    mockLatest("v0.6.0");
    const { update, pushToastMock } = await loadUpdate({ platform: "desktop", version: "0.5.3" });

    await Promise.all([update.offerUpdate("0.6.0", "0.5.3"), update.offerUpdate("0.6.0", "0.5.3")]);

    expect(pushToastMock).toHaveBeenCalledTimes(1);
  });

  it("checks without installing: only the Install update action runs the updater (GH #241)", async () => {
    mockLatest("v0.6.0");
    const { update, pushToastMock, updaterCheckMock } = await loadUpdate({ platform: "desktop", version: "0.5.3" });

    await expect(update.checkForUpdateNow()).resolves.toMatchObject({ kind: "available" });
    expect(updaterCheckMock).not.toHaveBeenCalled();
    const offer = toastCalls(pushToastMock).find(([message]) => message.includes("0.6.0 is available"));
    expect(offer?.[2]).toMatchObject({ sticky: true, action: { label: "Install update" } });

    offer?.[2]?.action?.run();
    await vi.waitFor(() => expect(updaterCheckMock).toHaveBeenCalledOnce());
  });

  it("keeps the manual current-version result on desktop Tauri", async () => {
    mockLatest("v0.5.3");
    const { update } = await loadUpdate({ platform: "desktop", version: "0.5.3" });

    await expect(update.checkForUpdateNow()).resolves.toEqual({ kind: "current", version: "0.5.3" });
  });

  it("offers a manual download, not a native install, to the x86 build and records the policy, not a failure (GH #594)", async () => {
    mockLatest("v0.6.0");
    const { update, pushToastMock, updaterCheckMock, openExternalMock, diagnosticFrontendEventMock } =
      await loadUpdate({ platform: "desktop", architecture: "x86", version: "0.5.3" });

    await update.checkForUpdate();
    const offer = toastCalls(pushToastMock).find(([message]) => message.includes("32-bit Windows"));
    expect(offer?.[2]).toMatchObject({ sticky: true, action: { label: "Download manually" } });
    expect(toastCalls(pushToastMock).some(([, , options]) => options?.action?.label === "Install update")).toBe(false);
    expect(diagnosticFrontendEventMock).toHaveBeenCalledWith("updater_manual_only", undefined);
    expect(diagnosticFrontendEventMock).not.toHaveBeenCalledWith("updater_failure", expect.anything());

    await expect(update.checkForUpdateNow()).resolves.toMatchObject({ kind: "available" });
    await vi.waitFor(() => expect(pushToastMock).toHaveBeenCalledTimes(2));
    expect(updaterCheckMock).not.toHaveBeenCalled();
    expect(toastCalls(pushToastMock)[1][0]).toContain("32-bit Windows");

    offer?.[2]?.action?.run();
    expect(openExternalMock).toHaveBeenCalledOnce();
  });

  it("records a fixed stage/cause for a failed install and points the user at Diagnostics (GH #343)", async () => {
    mockLatest("v0.6.0");
    const failure = new Error(
      "error sending request for url https://reporter:hunter2@example.test/latest.json?token=secret",
      { cause: new Error("api_key=key123 at C:/Users/Reporter/secret.txt (/home/reporter/private/file)") },
    );
    const { update, pushToastMock, diagnosticFrontendEventMock, openExternalMock, openSettingsMock } =
      await loadUpdate({ platform: "desktop", updaterReject: failure });

    await update.checkForUpdateNow();
    toastCalls(pushToastMock).find(([message]) => message.includes("0.6.0 is available"))?.[2]?.action?.run();
    await vi.waitFor(() => expect(diagnosticFrontendEventMock).toHaveBeenCalledWith(
      "updater_failure", { updaterStage: "manifest_fetch", updaterCause: "network" },
    ));
    expect(JSON.stringify(diagnosticFrontendEventMock.mock.calls)).not.toMatch(/example|hunter2|key123|Reporter|reporter/);
    const failureToast = toastCalls(pushToastMock).find(([message]) => message.includes("manifest fetch"));
    expect(failureToast?.[1]).toBe("error");
    failureToast?.[2]?.action?.run();
    expect(openSettingsMock).toHaveBeenCalledWith("diagnostics");
    expect(openExternalMock).toHaveBeenCalled(); // releases page still opens as the safe fallback

    const chain = update.safeUpdaterErrorChain(failure);
    expect(chain).toContain("<url>");
    expect(chain).not.toMatch(/hunter2|key123|token=secret|C:[\\/]Users|example\.test|\/home\/reporter/);
  });

  it.each([
    ["check", "error sending request for url https://example.test/latest.json", "manifest_fetch", "network"],
    ["check", "failed to deserialize update response", "manifest_parse", "invalid_manifest"],
    ["check", "missing field `version` at line 1 column 17", "manifest_parse", "invalid_manifest"],
    ["check", "None of the fallback platforms were found", "target_selection", "unsupported_target"],
    ["check", "connection failed because the target machine actively refused it", "manifest_fetch", "network"],
    ["apply", "download failed: connection reset", "download", "network"],
    ["apply", "minisign signature verification failed", "signature_verification", "invalid_signature"],
    ["apply", "Failed to install package", "install", "install_failed"],
    ["relaunch", "process restart refused", "relaunch", "relaunch_failed"],
  ] as const)("classifies %s failures without retaining their free-form text", async (phase, message, stage, cause) => {
    const { update } = await loadUpdate({ platform: "desktop" });
    expect(update.classifyUpdaterFailure(phase, new Error(message))).toEqual({ stage, cause });
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
