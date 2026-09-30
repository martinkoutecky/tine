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
    ? vi.fn<() => Promise<unknown>>(async () => { throw opts.updaterReject; })
    : vi.fn<() => Promise<unknown>>(async () => opts.updaterUpdate ?? offerFromChannel(opts.version ?? "0.5.3"));
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

// The updater plugin's `check()` is the ONE answerer of what og-preview offers: it
// returns an Update only when the channel's version is newer than the running app,
// else null. `mockLatest` sets what the channel's manifest says; the mocked plugin
// applies the same newer-than rule.
let channelVersion: string | null = null;
const triple = (v: string) => (/(\d+)\.(\d+)\.(\d+)/.exec(v) ?? []).slice(1).map(Number);
function offerFromChannel(running: string) {
  if (!channelVersion) return null;
  const a = triple(channelVersion), b = triple(running);
  const newer = a[0] !== b[0] ? a[0] > b[0] : a[1] !== b[1] ? a[1] > b[1] : a[2] > b[2];
  return newer ? { version: channelVersion, close: vi.fn(async () => {}) } : null;
}
function mockLatest(version: string): void {
  channelVersion = version.replace(/^v/, "");
}

describe("update checks", () => {
  afterEach(() => {
    channelVersion = null;
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it("asks only the updater plugin (never fetch) and reports its offer", async () => {
    mockLatest("v0.6.0");
    const fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);
    const { update, updaterCheckMock } = await loadUpdate({ platform: "desktop", version: "0.5.3" });

    await update.checkForUpdate();
    await expect(update.checkForUpdateNow()).resolves.toMatchObject({ kind: "available", version: "0.6.0" });

    expect(updaterCheckMock).toHaveBeenCalledTimes(2);
    expect(fetchMock, "a webview fetch of a github.com release asset has no CORS: never fetch the channel").not.toHaveBeenCalled();
  });

  it("releases the plugin's handle after learning the version", async () => {
    const close = vi.fn(async () => {});
    const { update } = await loadUpdate({
      platform: "desktop", version: "0.5.3", updaterUpdate: { version: "0.6.0", close },
    });
    await update.checkForUpdate();
    expect(close).toHaveBeenCalledOnce();
  });

  it("is silent when nothing newer is offered (check() returns null)", async () => {
    const { update, pushToastMock } = await loadUpdate({ platform: "desktop", version: "0.5.3" });

    await update.checkForUpdate();
    await expect(update.checkForUpdateNow()).resolves.toEqual({ kind: "current", version: "0.5.3" });

    expect(pushToastMock).not.toHaveBeenCalled();
  });

  it.each([
    ["a missing manifest (404)", new Error("Could not fetch a valid release JSON from the remote")],
    ["an unreachable channel", new Error("error sending request for url")],
    ["an invalid manifest", new Error("failed to deserialize update response")],
  ])("stays silent, and About says unavailable, on %s", async (_label, failure) => {
    const { update, pushToastMock } = await loadUpdate({ platform: "desktop", version: "0.5.3", updaterReject: failure });

    await update.checkForUpdate();
    await expect(update.checkForUpdateNow()).resolves.toEqual({ kind: "unavailable" });

    expect(pushToastMock).not.toHaveBeenCalled();
  });

  it("stays silent when the offered version is not parsable", async () => {
    const { update, pushToastMock } = await loadUpdate({
      platform: "desktop", version: "0.5.3", updaterUpdate: { version: "nightly", close: async () => {} },
    });
    await update.checkForUpdate();
    await expect(update.checkForUpdateNow()).resolves.toEqual({ kind: "current", version: "0.5.3" });
    expect(pushToastMock).not.toHaveBeenCalled();
  });

  it("keeps macOS on the manual releases page while still learning the version from check()", async () => {
    vi.stubGlobal("navigator", { userAgent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0)" });
    mockLatest("v0.6.0");
    const { update, pushToastMock, updaterCheckMock, openExternalMock } = await loadUpdate({ platform: "desktop", version: "0.5.3" });

    await update.checkForUpdate();
    expect(updaterCheckMock).toHaveBeenCalledOnce();
    const offer = toastCalls(pushToastMock).find(([message]) => message.includes("0.6.0 is available"));
    offer?.[2]?.action?.run();
    await vi.waitFor(() => expect(openExternalMock).toHaveBeenCalledWith("https://github.com/martinkoutecky/tine/releases/tag/og-preview"));
    expect(updaterCheckMock, "manual mode never runs the in-place installer").toHaveBeenCalledOnce();
  });

  it("points the manual releases fallback at the og-preview release page", async () => {
    const { update, openExternalMock } = await loadUpdate({ platform: "desktop", version: "0.5.3" });

    update.openReleasesPage();

    expect(openExternalMock).toHaveBeenCalledWith("https://github.com/martinkoutecky/tine/releases/tag/og-preview");
  });

  it.each(["android", "ios"] as const)("never checks or offers self-update on %s", async (platform) => {
    mockLatest("v0.6.0");
    const { update, platformKindMock, getVersionMock, updaterCheckMock, openExternalMock, pushToastMock } =
      await loadUpdate({ platform });

    await update.checkForUpdate();
    await expect(update.checkForUpdateNow()).resolves.toEqual({ kind: "unavailable" });

    expect(platformKindMock).toHaveBeenCalled();
    expect(getVersionMock).not.toHaveBeenCalled();
    expect(updaterCheckMock).not.toHaveBeenCalled();
    expect(openExternalMock).not.toHaveBeenCalled();
    expect(pushToastMock).not.toHaveBeenCalled();
  });

  it("fails closed when native platform detection fails", async () => {
    mockLatest("v0.6.0");
    const { update, getVersionMock, updaterCheckMock, pushToastMock } = await loadUpdate({
      platformReject: true,
    });

    await update.checkForUpdate();
    await expect(update.checkForUpdateNow()).resolves.toEqual({ kind: "unavailable" });

    expect(getVersionMock).not.toHaveBeenCalled();
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
    expect(updaterCheckMock, "learning the version is one check(); nothing is downloaded").toHaveBeenCalledOnce();
    const offer = toastCalls(pushToastMock).find(([message]) => message.includes("0.6.0 is available"));
    expect(offer?.[2]).toMatchObject({ sticky: true, action: { label: "Install update" } });

    offer?.[2]?.action?.run();
    await vi.waitFor(() => expect(updaterCheckMock).toHaveBeenCalledTimes(2)); // the installer's own check()
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
    expect(updaterCheckMock, "two version checks (startup + About), never an install").toHaveBeenCalledTimes(2);
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
    const { update, pushToastMock, diagnosticFrontendEventMock, openExternalMock, openSettingsMock, updaterCheckMock } =
      await loadUpdate({ platform: "desktop", version: "0.5.3" });
    // The version check succeeds; the installer's own check() then fails.
    updaterCheckMock
      .mockResolvedValueOnce({ version: "0.6.0", close: async () => {} })
      .mockRejectedValueOnce(failure);

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
    mockLatest("v0.6.0");
    const { update, platformKindMock, updaterCheckMock } = await loadUpdate({ tauri: false });

    await update.checkForUpdate();
    await expect(update.checkForUpdateNow()).resolves.toEqual({ kind: "unavailable" });

    expect(platformKindMock).not.toHaveBeenCalled();
    expect(updaterCheckMock).not.toHaveBeenCalled();
  });

  describe("installing flushes saves first (the window-close gate)", () => {
    async function installFlow(opts: { prepare: "accepted" | "rejected" | "in_flight"; installFails?: boolean; downloadFails?: boolean }) {
      mockLatest("v0.6.0");
      const order: string[] = [];
      const updateObject = {
        version: "0.6.0",
        download: vi.fn(async () => { order.push("download"); if (opts.downloadFails) throw new Error("download failed"); }),
        close: vi.fn(async () => {}),
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

    it.each(["accepted", "rejected", "in_flight", "download", "install"] as const)("L17:67: closes the update resource on %s exit", async (exit) => {
      const { updateObject } = await installFlow({ prepare: exit === "download" || exit === "install" ? "accepted" : exit,
        downloadFails: exit === "download", installFails: exit === "install" });
      await vi.waitFor(() => expect(updateObject.close).toHaveBeenCalledTimes(2));
      // The check closes its handle; the install action acquires and closes another.
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
