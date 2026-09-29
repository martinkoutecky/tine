import { readFileSync } from "node:fs";
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
  updaterUpdate?: object;
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
    : vi.fn(async () => opts.updaterUpdate ?? null);
  const relaunchMock = vi.fn(async () => {});

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
  vi.doMock("@tauri-apps/plugin-process", () => ({ relaunch: relaunchMock }));

  const update = await import("./update");
  return {
    update,
    platformKindMock,
    getVersionMock,
    updaterCheckMock,
    relaunchMock,
    openExternalMock,
    pushToastMock,
    dismissToastMock,
    openSettingsMock,
    diagnosticFrontendEventMock,
  };
}

const MANIFEST = "https://github.com/martinkoutecky/tine/releases/download/og-preview/latest.json";

/** The og-preview channel manifest (latest.json): the build's version in `version`. */
function mockLatest(version: string, ok = true) {
  const fetchMock = vi.fn(async () => ({
    ok,
    json: async () => ({ version, notes: "", platforms: {} }),
  }));
  vi.stubGlobal("fetch", fetchMock);
  return fetchMock;
}

describe("update checks", () => {
  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it("reads only the og-preview channel, never the shipped Tine's releases/latest", async () => {
    const fetchMock = mockLatest("v0.6.0");
    const { update } = await loadUpdate({ platform: "desktop", version: "0.5.3" });

    await update.checkForUpdate();
    await update.checkForUpdateNow();

    expect(fetchMock.mock.calls.length).toBe(2);
    for (const call of fetchMock.mock.calls as unknown as [string][]) {
      expect(call[0]).toBe(MANIFEST);
    }
  });

  it("offers nothing and raises no toast when the og-preview release does not exist", async () => {
    const fetchMock = mockLatest("v9.9.9", false); // GitHub answers 404
    const { update, pushToastMock } = await loadUpdate({ platform: "desktop", version: "0.5.3" });

    await update.checkForUpdate();
    await expect(update.checkForUpdateNow()).resolves.toEqual({ kind: "unavailable" });

    expect(fetchMock).toHaveBeenCalled();
    expect(pushToastMock).not.toHaveBeenCalled();
  });

  it.each([
    ["a manifest with no version (only a tag/name)", async () => ({ tag_name: "v0.6.987", name: "Tine 0.6.987" })],
    ["a non-string version", async () => ({ version: 7 })],
    ["invalid JSON", async () => { throw new SyntaxError("Unexpected token <"); }],
  ])("stays silent on %s", async (_label, json) => {
    vi.stubGlobal("fetch", vi.fn(async () => ({ ok: true, json })));
    const { update, pushToastMock } = await loadUpdate({ platform: "desktop", version: "0.5.3" });

    await update.checkForUpdate();
    await expect(update.checkForUpdateNow()).resolves.toEqual({ kind: "unavailable" });

    expect(pushToastMock).not.toHaveBeenCalled();
  });

  it("reports current and stays quiet when the preview is not newer", async () => {
    mockLatest("v0.5.3");
    const { update, pushToastMock } = await loadUpdate({ platform: "desktop", version: "0.5.3" });

    await update.checkForUpdate();
    await expect(update.checkForUpdateNow()).resolves.toEqual({ kind: "current", version: "0.5.3" });

    expect(pushToastMock).not.toHaveBeenCalled();
  });

  it("points the manual releases fallback at the og-preview release page", async () => {
    mockLatest("v0.6.0");
    const { update, openExternalMock } = await loadUpdate({ platform: "desktop", version: "0.5.3" });

    update.openReleasesPage();

    expect(openExternalMock).toHaveBeenCalledWith("https://github.com/martinkoutecky/tine/releases/tag/og-preview");
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

  describe("installing flushes saves first (the window-close gate)", () => {
    async function installFlow(opts: { prepare: "accepted" | "rejected" | "in_flight"; installFails?: boolean }) {
      mockLatest("v0.6.0");
      const order: string[] = [];
      const updateObject = {
        version: "0.6.0",
        download: vi.fn(async () => { order.push("download"); }),
        install: vi.fn(async () => {
          order.push("install");
          if (opts.installFails) throw new Error("Failed to install package");
        }),
        downloadAndInstall: vi.fn(async () => { order.push("downloadAndInstall"); }),
      };
      const loaded = await loadUpdate({ platform: "desktop", version: "0.5.3", updaterUpdate: updateObject });
      const guard = {
        prepare: vi.fn(async () => { order.push("prepare"); return opts.prepare; }),
        reset: vi.fn(() => { order.push("reset"); }),
      };
      loaded.update.setUpdateExitGuard(guard);
      loaded.relaunchMock.mockImplementation(async () => { order.push("relaunch"); });
      await loaded.update.checkForUpdateNow();
      toastCalls(loaded.pushToastMock).find(([message]) => message.includes("0.6.0 is available"))?.[2]?.action?.run();
      return { ...loaded, order, updateObject, guard };
    }

    it("downloads, flushes through the shared exit gate, and only then installs and relaunches", async () => {
      const { order, updateObject } = await installFlow({ prepare: "accepted" });
      await vi.waitFor(() => expect(order).toEqual(["download", "prepare", "install", "relaunch"]));
      expect(updateObject.downloadAndInstall).not.toHaveBeenCalled();
    });

    it.each(["rejected", "in_flight"] as const)("does not install when the flush gate answers %s, and says why", async (prepare) => {
      const { order, updateObject, pushToastMock, openExternalMock } = await installFlow({ prepare });
      await vi.waitFor(() => expect(order).toEqual(["download", "prepare"]));
      await vi.waitFor(() => expect(toastCalls(pushToastMock).some(([m]) => m.includes("not installed"))).toBe(true));
      expect(updateObject.install).not.toHaveBeenCalled();
      expect(openExternalMock).not.toHaveBeenCalled();
    });

    it("App registers the window-close coordinator as the update's exit gate, and nothing installs without it", () => {
      expect(readFileSync("src/App.tsx", "utf8")).toContain("setUpdateExitGuard(safeClose);");
      const source = readFileSync("src/update.ts", "utf8");
      expect(source).not.toMatch(/update\.downloadAndInstall\(/);
      expect(source.match(/\.install\(\)/g)).toHaveLength(1);
    });

    it("releases the exit gate when the install itself fails, so later closes still save", async () => {
      const { order } = await installFlow({ prepare: "accepted", installFails: true });
      await vi.waitFor(() => expect(order).toEqual(["download", "prepare", "install", "reset"]));
    });
  });
});
