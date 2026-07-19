import { afterEach, describe, expect, it, vi } from "vitest";
import { backend } from "../backend";
import { doc, isDirty, resetStore, setDoc } from "../store";
import { bumpGraphEpoch, setGraphMeta, setGraphTransitioning } from "../ui";
import type { GraphMeta } from "../types";
import { installedPlugins, PluginManager } from "./manager";
import { bindPluginBlockSnapshot, capturePluginGraphOwner } from "./ownership";
import type { PluginEvent, PluginResponse } from "./protocol";
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

function graphMeta(root: string): GraphMeta {
  return {
    root, journals_dir: "journals", pages_dir: "pages", preferred_workflow: "now",
    shortcuts: {}, start_of_week: 6, block_hidden_properties: [], default_journal_template: null,
    favorites: [], journal_page_title_format: "MMM do, yyyy", journal_file_name_format: "yyyy_MM_dd",
    preferred_format: "md", macros: {}, enable_timetracking: true, show_brackets: true, logbook_with_second_support: true,
    logbook_enabled_in_timestamped_blocks: false, logbook_enabled_in_all_blocks: false, guide_announced: true,
  };
}

function commandRecord() {
  const value = {
    ...manifest("page.tine.graph-owner", "Graph owner"),
    capabilities: ["commands.register", "slash-commands.register", "block-decorations.register", "graph.write.block"],
    contributions: {
      commands: [{ id: "write", title: "Write", description: "Write focused block." }],
      slashCommands: [{ id: "insert", title: "Insert" }],
      blockDecorations: [{ id: "badge", kind: "badge" }],
    },
  };
  return { ...record(value.id, value.name), manifest_json: JSON.stringify(value) };
}

function sharedDoc(raw = "same raw") {
  setDoc({
    byId: { "shared-id": { id: "shared-id", raw, collapsed: false, parent: null, page: "Shared", children: [] } },
    pages: [{ name: "Shared", kind: "page", title: "Shared", preBlock: null, roots: ["shared-id"], format: "md", readOnly: false, guide: false }],
    feed: ["Shared"], loaded: true,
  });
}

afterEach(() => {
  vi.restoreAllMocks();
  setGraphTransitioning(false);
  setGraphMeta(null);
  resetStore();
});

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

  it("serializes a held enable behind a winning live revocation", async () => {
    const api = backend();
    const id = "page.tine.held-enable-race";
    vi.spyOn(api, "appPlatform").mockResolvedValue("desktop");
    vi.spyOn(api, "listInstalledPlugins").mockResolvedValue([{
      ...record(id, "Held enable race"),
      selected: false,
      enabled: false,
    }]);
    vi.spyOn(api, "getAppString").mockResolvedValue("{}");
    const readEntry = vi.spyOn(api, "readPluginEntry");
    const createRuntime = vi.spyOn(PluginRuntime, "create");
    let finishTrue!: () => void;
    const writes: boolean[] = [];
    const setEnabled = vi.spyOn(api, "setPluginEnabled").mockImplementation(async (_id, _version, enabled) => {
      writes.push(enabled);
      if (enabled) await new Promise<void>((resolve) => { finishTrue = resolve; });
    });

    const manager = new PluginManager();
    await manager.initialize(new Set(), true);
    const enabling = manager.enable(id, "1.0.0");
    await vi.waitFor(() => expect(setEnabled).toHaveBeenCalledWith(id, "1.0.0", true));

    const revoking = manager.applyRevocations(new Set([`${id}@1.0.0`]));
    finishTrue();
    await enabling;
    await revoking;

    expect(writes).toEqual([true, false]);
    expect(installedPlugins().find((item) => item.manifest.id === id)).toMatchObject({
      enabled: false,
      running: false,
      error: "This version was revoked by the registry.",
    });
    expect(readEntry).not.toHaveBeenCalled();
    expect(createRuntime).not.toHaveBeenCalled();
  });

  it("does not let hold release commit a stale running state after concurrent disable", async () => {
    const api = backend();
    const id = "page.tine.release-disable-race";
    vi.spyOn(api, "appPlatform").mockResolvedValue("desktop");
    vi.spyOn(api, "listInstalledPlugins").mockResolvedValue([record(id, "Release disable race")]);
    vi.spyOn(api, "getAppString").mockResolvedValue("{}");
    vi.spyOn(api, "readPluginEntry").mockResolvedValue(new Uint8Array([0, 97, 115, 109]));
    let finishRuntime!: (runtime: PluginRuntime) => void;
    const runtime = { invoke: vi.fn().mockResolvedValue({ effects: [] }), dispose: vi.fn() };
    const createRuntime = vi.spyOn(PluginRuntime, "create").mockReturnValue(new Promise((resolve) => {
      finishRuntime = resolve;
    }));
    let finishDisable!: () => void;
    const setEnabled = vi.spyOn(api, "setPluginEnabled").mockImplementation(async (_id, _version, enabled) => {
      if (!enabled) await new Promise<void>((resolve) => { finishDisable = resolve; });
    });

    const manager = new PluginManager();
    await manager.initialize(new Set(), true);
    const releasing = manager.setActivationHold(false);
    await vi.waitFor(() => expect(createRuntime).toHaveBeenCalledTimes(1));

    const disabling = manager.disable(id);
    await vi.waitFor(() => expect(setEnabled).toHaveBeenCalledWith(id, "1.0.0", false));
    finishRuntime(runtime as unknown as PluginRuntime);
    await releasing;
    finishDisable();
    await disabling;

    expect(runtime.invoke).not.toHaveBeenCalled();
    expect(runtime.dispose).toHaveBeenCalledTimes(1);
    expect(installedPlugins().find((item) => item.manifest.id === id)).toMatchObject({
      enabled: false,
      running: false,
      error: undefined,
    });
  });

  it("makes a successful held enable the selected durable intent so disable can clear it", async () => {
    const api = backend();
    const id = "page.tine.held-enable-disable";
    vi.spyOn(api, "appPlatform").mockResolvedValue("desktop");
    vi.spyOn(api, "listInstalledPlugins").mockResolvedValue([{
      ...record(id, "Held enable disable"),
      selected: false,
      enabled: false,
    }]);
    vi.spyOn(api, "getAppString").mockResolvedValue("{}");
    const writes: boolean[] = [];
    vi.spyOn(api, "setPluginEnabled").mockImplementation(async (_id, _version, enabled) => {
      writes.push(enabled);
    });

    const manager = new PluginManager();
    await manager.initialize(new Set(), true);
    await manager.enable(id, "1.0.0");
    expect(installedPlugins().find((item) => item.manifest.id === id)).toMatchObject({
      selected: true,
      enabled: true,
      running: false,
    });

    await manager.disable(id);

    expect(writes).toEqual([true, false]);
    expect(installedPlugins().find((item) => item.manifest.id === id)).toMatchObject({
      selected: true,
      enabled: false,
      running: false,
      error: undefined,
    });
  });

  it("starts exactly once when a non-revoking hold release passes a pending held enable", async () => {
    const api = backend();
    const id = "page.tine.held-enable-release";
    vi.spyOn(api, "appPlatform").mockResolvedValue("desktop");
    vi.spyOn(api, "listInstalledPlugins").mockResolvedValue([{
      ...record(id, "Held enable release"),
      selected: false,
      enabled: false,
    }]);
    vi.spyOn(api, "getAppString").mockResolvedValue("{}");
    let finishTrue!: () => void;
    const writes: boolean[] = [];
    const setEnabled = vi.spyOn(api, "setPluginEnabled").mockImplementation(async (_id, _version, enabled) => {
      writes.push(enabled);
      if (enabled) await new Promise<void>((resolve) => { finishTrue = resolve; });
    });
    const readEntry = vi.spyOn(api, "readPluginEntry").mockResolvedValue(new Uint8Array([0, 97, 115, 109]));
    const runtime = { invoke: vi.fn().mockResolvedValue({ effects: [] }), dispose: vi.fn() };
    const createRuntime = vi.spyOn(PluginRuntime, "create").mockResolvedValue(runtime as unknown as PluginRuntime);

    const manager = new PluginManager();
    await manager.initialize(new Set(), true);
    const enabling = manager.enable(id, "1.0.0");
    await vi.waitFor(() => expect(setEnabled).toHaveBeenCalledWith(id, "1.0.0", true));

    await manager.setActivationHold(false);
    expect(readEntry).not.toHaveBeenCalled();
    finishTrue();
    await enabling;

    expect(writes).toEqual([true]);
    expect(readEntry).toHaveBeenCalledTimes(1);
    expect(createRuntime).toHaveBeenCalledTimes(1);
    expect(runtime.invoke).toHaveBeenCalledTimes(1);
    expect(installedPlugins().find((item) => item.manifest.id === id)).toMatchObject({
      selected: true,
      enabled: true,
      running: true,
      error: undefined,
    });
  });
});

describe("plugin invocation ownership", () => {
  it("drops a delayed graph-A write when graph B has the same UUID and raw bytes", async () => {
    const api = backend();
    vi.spyOn(api, "appPlatform").mockResolvedValue("desktop");
    vi.spyOn(api, "listInstalledPlugins").mockResolvedValue([commandRecord()]);
    vi.spyOn(api, "readPluginEntry").mockResolvedValue(new Uint8Array([0, 97, 115, 109]));
    vi.spyOn(api, "getAppString").mockResolvedValue("{}");
    vi.spyOn(api, "setPluginEnabled").mockResolvedValue();

    let resolveCommand!: (response: PluginResponse) => void;
    const pending = new Promise<PluginResponse>((resolve) => { resolveCommand = resolve; });
    const runtime = {
      invoke: vi.fn((event: PluginEvent) => event.kind === "activate"
        ? Promise.resolve({ protocolVersion: 2 as const, effects: [] })
        : pending),
      dispose: vi.fn(),
    };
    vi.spyOn(PluginRuntime, "create").mockResolvedValue(runtime as unknown as PluginRuntime);

    const manager = new PluginManager();
    setGraphMeta(graphMeta("/graph-a"));
    sharedDoc();
    await manager.initialize();
    const owned = bindPluginBlockSnapshot({ id: "shared-id", raw: "same raw", parentId: null, depth: 0, format: "md" })!;
    const invocation = manager.invokeCommand("page.tine.graph-owner", "write", owned);
    await vi.waitFor(() => expect(runtime.invoke).toHaveBeenCalledTimes(2));

    setGraphTransitioning(true);
    resetStore();
    setGraphMeta(graphMeta("/graph-b"));
    bumpGraphEpoch();
    sharedDoc();
    setGraphTransitioning(false);
    resolveCommand({ protocolVersion: 2, effects: [{ kind: "replace-block-text", blockId: "shared-id", expectedRaw: "same raw", raw: "A result" }] });
    await invocation;

    expect(doc.byId["shared-id"].raw).toBe("same raw");
    expect(isDirty("Shared")).toBe(false);
    expect(runtime.dispose).not.toHaveBeenCalled();
  });

  it("rejects stale palette/decorate owners before worker invocation and permits the current owner", async () => {
    const api = backend();
    vi.spyOn(api, "appPlatform").mockResolvedValue("desktop");
    vi.spyOn(api, "listInstalledPlugins").mockResolvedValue([commandRecord()]);
    vi.spyOn(api, "readPluginEntry").mockResolvedValue(new Uint8Array([0, 97, 115, 109]));
    vi.spyOn(api, "getAppString").mockResolvedValue("{}");
    vi.spyOn(api, "setPluginEnabled").mockResolvedValue();
    const runtime = {
      invoke: vi.fn().mockResolvedValue({ protocolVersion: 2 as const, effects: [] }),
      dispose: vi.fn(),
    };
    vi.spyOn(PluginRuntime, "create").mockResolvedValue(runtime as unknown as PluginRuntime);
    setGraphMeta(graphMeta("/graph-a"));
    sharedDoc();
    const manager = new PluginManager();
    await manager.initialize();
    const stale = bindPluginBlockSnapshot({ id: "shared-id", raw: "same raw", parentId: null, depth: 0 })!;
    bumpGraphEpoch();

    await manager.invokeCommand("page.tine.graph-owner", "write", stale);
    await manager.decorateBlocks("page.tine.graph-owner", "badge", { owner: stale.owner, blocks: [stale.block] });
    expect(runtime.invoke).toHaveBeenCalledTimes(1);

    const current = bindPluginBlockSnapshot({ id: "shared-id", raw: "same raw", parentId: null, depth: 0 })!;
    await manager.invokeCommand("page.tine.graph-owner", "write", current);
    await manager.decorateBlocks("page.tine.graph-owner", "badge", { owner: capturePluginGraphOwner()!, blocks: [current.block] });
    expect(runtime.invoke).toHaveBeenCalledTimes(3);
  });

  it("does not disable or dispose a successor when an old runtime fails late", async () => {
    const api = backend();
    vi.spyOn(api, "appPlatform").mockResolvedValue("desktop");
    vi.spyOn(api, "listInstalledPlugins").mockResolvedValue([commandRecord()]);
    vi.spyOn(api, "readPluginEntry").mockResolvedValue(new Uint8Array([0, 97, 115, 109]));
    vi.spyOn(api, "getAppString").mockResolvedValue("{}");
    const persist = vi.spyOn(api, "setPluginEnabled").mockResolvedValue();
    let rejectCommand!: (error: Error) => void;
    const pending = new Promise<PluginResponse>((_resolve, reject) => { rejectCommand = reject; });
    const oldRuntime = {
      invoke: vi.fn((event: PluginEvent) => event.kind === "activate"
        ? Promise.resolve({ protocolVersion: 2 as const, effects: [] })
        : pending),
      dispose: vi.fn(),
    };
    vi.spyOn(PluginRuntime, "create").mockResolvedValue(oldRuntime as unknown as PluginRuntime);
    setGraphMeta(graphMeta("/graph-a"));
    sharedDoc();
    const manager = new PluginManager();
    await manager.initialize();
    const owned = bindPluginBlockSnapshot({ id: "shared-id", raw: "same raw", parentId: null, depth: 0 })!;
    const invocation = manager.invokeCommand("page.tine.graph-owner", "write", owned);
    await vi.waitFor(() => expect(oldRuntime.invoke).toHaveBeenCalledTimes(2));

    const successor = { manifest: JSON.parse(commandRecord().manifest_json), runtime: { invoke: vi.fn(), dispose: vi.fn() } };
    (manager as unknown as { active: Map<string, unknown> }).active.set("page.tine.graph-owner", successor);
    rejectCommand(new Error("old runtime failed"));
    await expect(invocation).rejects.toThrow("old runtime failed");

    expect(oldRuntime.dispose).not.toHaveBeenCalled();
    expect(successor.runtime.dispose).not.toHaveBeenCalled();
    expect(persist).not.toHaveBeenCalled();
  });

  it("does not disable an active runtime when a new start attempt fails before registration", async () => {
    const api = backend();
    vi.spyOn(api, "appPlatform").mockResolvedValue("desktop");
    vi.spyOn(api, "listInstalledPlugins").mockResolvedValue([commandRecord()]);
    const readEntry = vi.spyOn(api, "readPluginEntry").mockResolvedValue(new Uint8Array([0, 97, 115, 109]));
    vi.spyOn(api, "getAppString").mockResolvedValue("{}");
    const persist = vi.spyOn(api, "setPluginEnabled").mockResolvedValue();
    const activeRuntime = { invoke: vi.fn().mockResolvedValue({ protocolVersion: 2 as const, effects: [] }), dispose: vi.fn() };
    vi.spyOn(PluginRuntime, "create").mockResolvedValue(activeRuntime as unknown as PluginRuntime);
    const manager = new PluginManager();
    await manager.initialize();
    readEntry.mockRejectedValueOnce(new Error("new start could not read bytes"));

    await expect(manager.enable("page.tine.graph-owner", "1.0.0")).rejects.toThrow("new start could not read bytes");

    expect(activeRuntime.dispose).not.toHaveBeenCalled();
    expect(persist).not.toHaveBeenCalled();
    expect(installedPlugins().find((item) => item.manifest.id === "page.tine.graph-owner")).toMatchObject({
      enabled: true,
      running: true,
      error: undefined,
    });
  });
});
