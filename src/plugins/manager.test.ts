import { afterEach, describe, expect, it, vi } from "vitest";
import { backend } from "../backend";
import { installedPlugins, PluginManager } from "./manager";
import { PluginRuntime } from "./runtime";

const manifest = (id: string, name: string) => ({
  schemaVersion: 1 as const,
  id,
  name,
  version: "1.0.0",
  apiVersion: "0.2" as const,
  description: `${name} test plugin.`,
  author: "Tine",
  license: "MIT",
  source: `https://example.invalid/${id}`,
  entry: "plugin.wasm",
  platforms: ["desktop" as const],
  capabilities: [],
});

const record = (id: string, name: string) => ({
  id,
  version: "1.0.0",
  manifest_json: JSON.stringify(manifest(id, name)),
  sha256: "mock",
  selected: true,
  enabled: true,
});

afterEach(() => vi.restoreAllMocks());

describe("installed plugin lifecycle", () => {
  it("uninstalls an incompatible stored manifest through its real package identity", async () => {
    const manifest = {
      schemaVersion: 1,
      id: "page.tine.legacy-uninstall-test",
      name: "Legacy uninstall test",
      version: "0.1.0",
      apiVersion: "0.1",
      description: "An intentionally incompatible installed package.",
      author: "Tine",
      license: "MIT",
      source: "https://example.invalid/legacy",
      entry: "plugin.wasm",
      platforms: ["desktop"],
      capabilities: [],
    };
    await backend().installPlugin(JSON.stringify(manifest), new Uint8Array([0, 97, 115, 109]));
    const manager = new PluginManager();
    await manager.initialize();
    const incompatible = installedPlugins().find((plugin) => plugin.error?.includes("apiVersion must be 0.2"));

    expect(incompatible?.manifest.id).toMatch(/^invalid\./);
    await expect(
      manager.uninstall(incompatible!.manifest.id, incompatible!.manifest.version)
    ).resolves.toBeUndefined();
    await manager.initialize();
    expect(installedPlugins().some((plugin) => plugin.error?.includes("apiVersion must be 0.2"))).toBe(false);
  });

  it("isolates a failed persisted activation while a live revocation disposes an earlier plugin", async () => {
    const api = backend();
    vi.spyOn(api, "appPlatform").mockResolvedValue("desktop");
    vi.spyOn(api, "listInstalledPlugins").mockResolvedValue([
      record("page.tine.startup-a", "Startup A"),
      record("page.tine.startup-b", "Startup B"),
    ]);
    vi.spyOn(api, "readPluginEntry").mockResolvedValue(new Uint8Array([0, 97, 115, 109]));
    vi.spyOn(api, "getAppString").mockResolvedValue("{}");
    const setEnabled = vi.spyOn(api, "setPluginEnabled").mockResolvedValue();

    let failB!: (error: Error) => void;
    const bActivation = new Promise<never>((_resolve, reject) => { failB = reject; });
    const runtimeA = { invoke: vi.fn().mockResolvedValue({ effects: [] }), dispose: vi.fn() };
    const runtimeB = { invoke: vi.fn().mockReturnValue(bActivation), dispose: vi.fn() };
    vi.spyOn(PluginRuntime, "create")
      .mockResolvedValueOnce(runtimeA as unknown as PluginRuntime)
      .mockResolvedValueOnce(runtimeB as unknown as PluginRuntime);

    const manager = new PluginManager();
    const initializing = manager.initialize(new Set());
    await vi.waitFor(() => expect(runtimeB.invoke).toHaveBeenCalled());

    const revokingA = manager.applyRevocations(new Set(["page.tine.startup-a@1.0.0"]));
    failB(new Error("B activation failed"));
    await expect(initializing).resolves.toBeUndefined();
    await revokingA;

    expect(runtimeA.dispose).toHaveBeenCalledTimes(1);
    expect(runtimeB.dispose).toHaveBeenCalledTimes(1);
    expect(installedPlugins().find((item) => item.manifest.id === "page.tine.startup-a")).toMatchObject({
      enabled: false,
      running: false,
      error: "This version was revoked by the registry.",
    });
    expect(installedPlugins().find((item) => item.manifest.id === "page.tine.startup-b")).toMatchObject({
      enabled: false,
      running: false,
      error: "B activation failed",
    });
    expect(setEnabled.mock.calls.filter(([id, version, enabled]) =>
      id === "page.tine.startup-a" && version === "1.0.0" && enabled === false
    )).toHaveLength(1);
  });

  it("does not overwrite a newer live revocation after the platform startup await", async () => {
    const api = backend();
    let resolvePlatform!: (platform: "desktop") => void;
    vi.spyOn(api, "appPlatform").mockReturnValue(new Promise((resolve) => { resolvePlatform = resolve; }));
    vi.spyOn(api, "listInstalledPlugins").mockResolvedValue([
      record("page.tine.startup-race", "Startup race"),
    ]);
    const readEntry = vi.spyOn(api, "readPluginEntry").mockResolvedValue(new Uint8Array([0, 97, 115, 109]));
    const createRuntime = vi.spyOn(PluginRuntime, "create");

    const manager = new PluginManager();
    const initializing = manager.initialize(new Set());
    await manager.applyRevocations(new Set(["page.tine.startup-race@1.0.0"]));
    resolvePlatform("desktop");
    await initializing;

    expect(readEntry).not.toHaveBeenCalled();
    expect(createRuntime).not.toHaveBeenCalled();
    expect(installedPlugins().find((item) => item.manifest.id === "page.tine.startup-race")).toMatchObject({
      enabled: false,
      error: "This version was revoked by the registry.",
    });
  });

  it("holds persisted and manual activation until registry verification releases it", async () => {
    const api = backend();
    vi.spyOn(api, "appPlatform").mockResolvedValue("desktop");
    vi.spyOn(api, "listInstalledPlugins").mockResolvedValue([
      record("page.tine.held", "Held plugin"),
    ]);
    const readEntry = vi.spyOn(api, "readPluginEntry").mockResolvedValue(new Uint8Array([0, 97, 115, 109]));
    vi.spyOn(api, "getAppString").mockResolvedValue("{}");
    const setEnabled = vi.spyOn(api, "setPluginEnabled").mockResolvedValue();
    const runtime = { invoke: vi.fn().mockResolvedValue({ effects: [] }), dispose: vi.fn() };
    vi.spyOn(PluginRuntime, "create").mockResolvedValue(runtime as unknown as PluginRuntime);

    const manager = new PluginManager();
    await manager.initialize(new Set(), true);
    expect(readEntry).not.toHaveBeenCalled();

    await manager.enable("page.tine.held", "1.0.0");
    expect(setEnabled).toHaveBeenCalledWith("page.tine.held", "1.0.0", true);
    expect(readEntry).not.toHaveBeenCalled();

    await manager.setActivationHold(false);
    expect(readEntry).toHaveBeenCalledTimes(1);
    expect(runtime.invoke).toHaveBeenCalledTimes(1);
  });

  it("blocks guest reads when durable revocation disable fails and retries on the next verified pass", async () => {
    const api = backend();
    vi.spyOn(api, "appPlatform").mockResolvedValue("desktop");
    vi.spyOn(api, "listInstalledPlugins").mockResolvedValue([
      record("page.tine.retry-disable", "Retry disable"),
    ]);
    const readEntry = vi.spyOn(api, "readPluginEntry").mockResolvedValue(new Uint8Array([0, 97, 115, 109]));
    const createRuntime = vi.spyOn(PluginRuntime, "create");
    vi.spyOn(api, "getAppString").mockResolvedValue("{}");
    const setEnabled = vi.spyOn(api, "setPluginEnabled")
      .mockRejectedValueOnce(new Error("disk full"))
      .mockResolvedValue(undefined);
    const revoked = new Set(["page.tine.retry-disable@1.0.0"]);

    const manager = new PluginManager();
    await manager.initialize(revoked);
    expect(readEntry).not.toHaveBeenCalled();
    expect(createRuntime).not.toHaveBeenCalled();
    expect(installedPlugins().find((item) => item.manifest.id === "page.tine.retry-disable")?.error).toContain("disk full");

    await manager.applyRevocations(revoked);
    expect(setEnabled).toHaveBeenCalledTimes(2);
    expect(installedPlugins().find((item) => item.manifest.id === "page.tine.retry-disable")).toMatchObject({
      enabled: false,
      running: false,
      error: "This version was revoked by the registry.",
    });
    expect(readEntry).not.toHaveBeenCalled();
    expect(createRuntime).not.toHaveBeenCalled();
  });
});
