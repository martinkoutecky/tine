import { afterEach, describe, expect, it, vi } from "vitest";
import { backend } from "../backend";
import { revokedThemeVersions } from "../themes/manager";
import { PluginRuntime } from "./runtime";
import { pluginManager } from "./manager";
import {
  loadVerifiedCachedRegistry,
  refreshCommunityRegistry,
  registryPersistenceError,
  registryState,
  seedCachedCommunityRegistry,
} from "./registry";
import { startCommunityExtensions } from "./startup";

const revokedId = "page.tine.cached-revoked";
const revokedVersion = "1.0.0";
const revokedKey = `${revokedId}@${revokedVersion}`;
const cachedIndex = JSON.stringify({
  schemaVersion: 1,
  generatedAt: "2026-07-17T00:00:00Z",
  plugins: [],
  themes: [],
  revocations: [{
    id: revokedId,
    version: revokedVersion,
    severity: "high",
    reason: "Startup revocation regression fixture.",
    revokedAt: "2026-07-17T00:00:00Z",
  }],
});
const cachedManifest = JSON.stringify({
  schemaVersion: 1,
  id: revokedId,
  name: "Cached revoked plugin",
  version: revokedVersion,
  apiVersion: "0.2",
  description: "Must never instantiate at startup.",
  author: "Tine",
  license: "MIT",
  source: "https://example.invalid/cached-revoked",
  entry: "plugin.wasm",
  platforms: ["desktop"],
  capabilities: ["commands.register"],
  contributions: { commands: [{ id: "forbidden", title: "Forbidden cached command" }] },
});

function installedRecord(id: string, enabled = true) {
  return {
    id,
    version: "1.0.0",
    manifest_json: JSON.stringify({
      schemaVersion: 1,
      id,
      name: id,
      version: "1.0.0",
      apiVersion: "0.2",
      description: "Registry startup test plugin.",
      author: "Tine",
      license: "MIT",
      source: `https://example.invalid/${id}`,
      entry: "plugin.wasm",
      platforms: ["desktop"],
      capabilities: [],
    }),
    sha256: "mock",
    selected: enabled,
    enabled,
  };
}

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("community extension startup revocations", () => {
  it("verifies cached revocations before plugin activation while the live fetch stalls", async () => {
    const api = backend();
    vi.spyOn(api, "loadPluginRegistryCache").mockResolvedValue({
      kind: "envelope",
      envelope: { schemaVersion: 1, indexJson: cachedIndex, signature: "valid-signature" },
    });
    vi.spyOn(api, "getAppString").mockResolvedValue("[]");
    vi.spyOn(api, "storePluginRegistryCache").mockResolvedValue();
    const verify = vi.spyOn(api, "verifyPluginRegistry").mockResolvedValue();
    vi.spyOn(api, "appPlatform").mockResolvedValue("desktop");
    vi.spyOn(api, "listInstalledPlugins").mockResolvedValue([{
      id: revokedId,
      version: revokedVersion,
      manifest_json: cachedManifest,
      sha256: "mock",
      selected: true,
      enabled: true,
    }]);
    const persistEnabled = vi.spyOn(api, "setPluginEnabled").mockResolvedValue();
    const readEntry = vi.spyOn(api, "readPluginEntry").mockResolvedValue(new Uint8Array([0, 97, 115, 109]));
    const createRuntime = vi.spyOn(PluginRuntime, "create");
    const liveSignals: AbortSignal[] = [];
    vi.stubGlobal("fetch", vi.fn((_url: string | URL | Request, init?: RequestInit) => {
      const liveSignal = init?.signal;
      if (liveSignal) liveSignals.push(liveSignal);
      return new Promise<Response>((_resolve, reject) => {
        liveSignal?.addEventListener("abort", () => reject(liveSignal.reason), { once: true });
      });
    }));

    const startup = await startCommunityExtensions({ networkTimeoutMs: 20, cacheTimeoutMs: 100 });
    await expect(startup.pluginInitialization).resolves.toBeUndefined();
    await expect(startup.liveRefresh).resolves.toBeUndefined();

    await vi.waitFor(() => expect(liveSignals.every((signal) => signal.aborted)).toBe(true));
    expect(readEntry).not.toHaveBeenCalled();
    expect(createRuntime).not.toHaveBeenCalled();
    expect(persistEnabled).toHaveBeenCalledWith(revokedId, revokedVersion, false);
    expect(startup.initialRevocations).toEqual(new Set([revokedKey]));
    expect(verify).toHaveBeenCalledWith(cachedIndex, "valid-signature");
  });

  it("rejects invalid and absent cached registries without inventing revocations", async () => {
    const api = backend();
    const load = vi.spyOn(api, "loadPluginRegistryCache");
    const verify = vi.spyOn(api, "verifyPluginRegistry");

    load.mockResolvedValue({
      kind: "envelope",
      envelope: { schemaVersion: 1, indexJson: "not-json", signature: "invalid-signature" },
    });
    verify.mockRejectedValue(new Error("signature did not verify"));
    const unsafe = await loadVerifiedCachedRegistry(100);
    expect(unsafe).toMatchObject({ kind: "unsafe" });
    expect(seedCachedCommunityRegistry(unsafe)).toEqual(new Set());
    expect(revokedThemeVersions()).toEqual(new Set());

    load.mockResolvedValue({ kind: "absent" });
    verify.mockClear();
    expect(await loadVerifiedCachedRegistry(100)).toEqual({ kind: "absent" });
    expect(verify).not.toHaveBeenCalled();
  });

  it("uses a verified legacy pair immediately and migrates it through one guarded atomic call", async () => {
    const api = backend();
    vi.spyOn(api, "loadPluginRegistryCache").mockResolvedValue({
      kind: "legacy",
      indexJson: cachedIndex,
      signature: " valid-signature ",
    });
    vi.spyOn(api, "verifyPluginRegistry").mockResolvedValue();
    const store = vi.spyOn(api, "storePluginRegistryCache").mockRejectedValue(new Error("disk full"));

    const loaded = await loadVerifiedCachedRegistry(100);

    expect(loaded).toMatchObject({ kind: "verified", source: "legacy" });
    expect(store).toHaveBeenCalledTimes(1);
    expect(store).toHaveBeenCalledWith(cachedIndex, " valid-signature ", {
      indexJson: cachedIndex,
      signature: " valid-signature ",
    });
    expect(registryPersistenceError()).toContain("disk full");
  });

  it("finishes guarded legacy migration before any live refresh can publish", async () => {
    const api = backend();
    vi.spyOn(api, "loadPluginRegistryCache").mockResolvedValue({
      kind: "legacy",
      indexJson: cachedIndex,
      signature: "valid-signature",
    });
    vi.spyOn(api, "verifyPluginRegistry").mockResolvedValue();
    let finishMigration!: () => void;
    const store = vi.spyOn(api, "storePluginRegistryCache").mockImplementationOnce(() => new Promise<void>((resolve) => {
      finishMigration = resolve;
    })).mockResolvedValue(undefined);
    vi.spyOn(api, "appPlatform").mockResolvedValue("desktop");
    vi.spyOn(api, "listInstalledPlugins").mockResolvedValue([]);
    vi.spyOn(api, "getAppString").mockImplementation(async (key, fallback) => key === "theme.packages.v1" ? "[]" : fallback);
    const fetch = vi.fn(async (url: string | URL | Request) =>
      new Response(String(url).endsWith(".sig") ? "live-signature" : cachedIndex)
    );
    vi.stubGlobal("fetch", fetch);

    const starting = startCommunityExtensions({ cacheTimeoutMs: 100, networkTimeoutMs: 100 });
    await vi.waitFor(() => expect(store).toHaveBeenCalledTimes(1));
    expect(fetch).not.toHaveBeenCalled();
    finishMigration();
    const startup = await starting;
    await startup.liveRefresh;
    expect(fetch).toHaveBeenCalledTimes(2);
  });

  it("classifies torn, malformed, unreadable, and timed-out cache loads as unsafe", async () => {
    const api = backend();
    const load = vi.spyOn(api, "loadPluginRegistryCache");
    load.mockResolvedValueOnce({ kind: "unsafe", reason: "legacy registry cache is torn" });
    await expect(loadVerifiedCachedRegistry(100)).resolves.toEqual({
      kind: "unsafe",
      reason: "legacy registry cache is torn",
    });
    load.mockRejectedValueOnce(new Error("settings unreadable"));
    await expect(loadVerifiedCachedRegistry(100)).resolves.toEqual({ kind: "unsafe", reason: "settings unreadable" });
    load.mockImplementationOnce(() => new Promise(() => {}));
    await expect(loadVerifiedCachedRegistry(5)).resolves.toMatchObject({ kind: "unsafe" });
  });

  it("holds every persisted guest on an unsafe cache without clearing user intent", async () => {
    const api = backend();
    vi.spyOn(api, "loadPluginRegistryCache").mockResolvedValue({ kind: "unsafe", reason: "legacy cache torn" });
    vi.spyOn(api, "appPlatform").mockResolvedValue("desktop");
    vi.spyOn(api, "listInstalledPlugins").mockResolvedValue([installedRecord("page.tine.unsafe-held")]);
    vi.spyOn(api, "getAppString").mockImplementation(async (key, fallback) => key === "theme.packages.v1" ? "[]" : fallback);
    const persistEnabled = vi.spyOn(api, "setPluginEnabled").mockResolvedValue();
    const readEntry = vi.spyOn(api, "readPluginEntry");
    const createRuntime = vi.spyOn(PluginRuntime, "create");
    vi.stubGlobal("fetch", vi.fn((_url: string | URL | Request, init?: RequestInit) => new Promise<Response>((_resolve, reject) => {
      init?.signal?.addEventListener("abort", () => reject(init.signal?.reason), { once: true });
    })));

    const startup = await startCommunityExtensions({ cacheTimeoutMs: 100, networkTimeoutMs: 10 });
    await startup.pluginInitialization;
    await startup.liveRefresh;

    expect(readEntry).not.toHaveBeenCalled();
    expect(createRuntime).not.toHaveBeenCalled();
    expect(persistEnabled).not.toHaveBeenCalled();
    expect(registryState()).toBe("unsafe");
  });

  it("distinguishes truly absent cache and retains local-package startup", async () => {
    const api = backend();
    vi.spyOn(api, "loadPluginRegistryCache").mockResolvedValue({ kind: "absent" });
    vi.spyOn(api, "appPlatform").mockResolvedValue("desktop");
    vi.spyOn(api, "listInstalledPlugins").mockResolvedValue([installedRecord("page.tine.absent-local")]);
    vi.spyOn(api, "getAppString").mockImplementation(async (key, fallback) => key === "theme.packages.v1" ? "[]" : fallback);
    vi.spyOn(api, "setPluginEnabled").mockResolvedValue();
    const readEntry = vi.spyOn(api, "readPluginEntry").mockResolvedValue(new Uint8Array([0, 97, 115, 109]));
    const runtime = { invoke: vi.fn().mockResolvedValue({ effects: [] }), dispose: vi.fn() };
    vi.spyOn(PluginRuntime, "create").mockResolvedValue(runtime as unknown as PluginRuntime);
    vi.stubGlobal("fetch", vi.fn((_url: string | URL | Request, init?: RequestInit) => new Promise<Response>((_resolve, reject) => {
      init?.signal?.addEventListener("abort", () => reject(init.signal?.reason), { once: true });
    })));

    const startup = await startCommunityExtensions({ cacheTimeoutMs: 100, networkTimeoutMs: 10 });
    await startup.pluginInitialization;
    await startup.liveRefresh;

    expect(readEntry).toHaveBeenCalledWith("page.tine.absent-local", "1.0.0");
    expect(runtime.invoke).toHaveBeenCalledTimes(1);
    await pluginManager.disable("page.tine.absent-local");
  });

  it("releases an unsafe-start hold only after live revocations durably block revoked guests", async () => {
    const api = backend();
    const revoked = "page.tine.live-held-revoked";
    const allowed = "page.tine.live-held-allowed";
    vi.spyOn(api, "loadPluginRegistryCache").mockResolvedValue({ kind: "unsafe", reason: "torn" });
    vi.spyOn(api, "appPlatform").mockResolvedValue("desktop");
    vi.spyOn(api, "listInstalledPlugins").mockResolvedValue([installedRecord(revoked), installedRecord(allowed)]);
    vi.spyOn(api, "getAppString").mockImplementation(async (key, fallback) => key === "theme.packages.v1" ? "[]" : fallback);
    const setEnabled = vi.spyOn(api, "setPluginEnabled").mockResolvedValue();
    const readEntry = vi.spyOn(api, "readPluginEntry").mockResolvedValue(new Uint8Array([0, 97, 115, 109]));
    const runtime = { invoke: vi.fn().mockResolvedValue({ effects: [] }), dispose: vi.fn() };
    vi.spyOn(PluginRuntime, "create").mockResolvedValue(runtime as unknown as PluginRuntime);
    vi.spyOn(api, "verifyPluginRegistry").mockResolvedValue();
    vi.spyOn(api, "storePluginRegistryCache").mockResolvedValue();
    const liveIndex = JSON.stringify({
      schemaVersion: 1,
      generatedAt: "2026-07-17T00:03:00Z",
      plugins: [],
      themes: [],
      revocations: [{ id: revoked, version: "1.0.0", severity: "high", reason: "revoked", revokedAt: "2026-07-17T00:03:00Z" }],
    });
    vi.stubGlobal("fetch", vi.fn(async (url: string | URL | Request) =>
      new Response(String(url).endsWith(".sig") ? "live-signature" : liveIndex)
    ));

    const startup = await startCommunityExtensions({ cacheTimeoutMs: 100, networkTimeoutMs: 100 });
    await startup.pluginInitialization;
    await startup.liveRefresh;

    expect(setEnabled).toHaveBeenCalledWith(revoked, "1.0.0", false);
    expect(readEntry).toHaveBeenCalledTimes(1);
    expect(readEntry).toHaveBeenCalledWith(allowed, "1.0.0");
    expect(runtime.invoke).toHaveBeenCalledTimes(1);
    await pluginManager.disable(allowed);
  });

  it("keeps a verified cached state when a streamed live body reaches its abort deadline", async () => {
    const api = backend();
    vi.spyOn(api, "loadPluginRegistryCache").mockResolvedValue({
      kind: "envelope",
      envelope: { schemaVersion: 1, indexJson: cachedIndex, signature: "valid-signature" },
    });
    vi.spyOn(api, "verifyPluginRegistry").mockResolvedValue();
    const cached = await loadVerifiedCachedRegistry(100);
    expect(cached.kind).toBe("verified");
    seedCachedCommunityRegistry(cached);

    const signals: AbortSignal[] = [];
    vi.stubGlobal("fetch", vi.fn(async (_url: string | URL | Request, init?: RequestInit) => {
      if (init?.signal) signals.push(init.signal);
      return new Response(new ReadableStream<Uint8Array>({
        start(controller) { controller.enqueue(new TextEncoder().encode("{")); }
      }));
    }));
    await refreshCommunityRegistry({ timeoutMs: 20 });

    await vi.waitFor(() => expect(signals.every((signal) => signal.aborted)).toBe(true));
    expect(registryState()).toBe("offline");
    expect(revokedThemeVersions()).toEqual(new Set([revokedKey]));
  });

  it("applies a newer verified live revocation to the running plugin manager", async () => {
    const api = backend();
    vi.spyOn(api, "verifyPluginRegistry").mockResolvedValue();
    vi.spyOn(api, "storePluginRegistryCache").mockResolvedValue();
    const apply = vi.spyOn(pluginManager, "applyRevocations").mockResolvedValue();
    const liveKey = "page.tine.live-revoked@2.0.0";
    const liveIndex = JSON.stringify({
      schemaVersion: 1,
      generatedAt: "2026-07-17T00:01:00Z",
      plugins: [],
      themes: [],
      revocations: [{
        id: "page.tine.live-revoked",
        version: "2.0.0",
        severity: "high",
        reason: "Newer verified live revocation.",
        revokedAt: "2026-07-17T00:01:00Z",
      }],
    });
    vi.stubGlobal("fetch", vi.fn(async (url: string | URL | Request) =>
      new Response(String(url).endsWith(".sig") ? "live-signature" : liveIndex)
    ));

    await refreshCommunityRegistry({ timeoutMs: 100 });

    expect(apply).toHaveBeenCalledWith(new Set([liveKey]));
    expect(registryState()).toBe("ready");
    expect(revokedThemeVersions()).toEqual(new Set([liveKey]));
  });

  it("keeps a verified live revocation effective and reports an atomic cache write failure", async () => {
    const api = backend();
    vi.spyOn(api, "verifyPluginRegistry").mockResolvedValue();
    vi.spyOn(api, "storePluginRegistryCache").mockRejectedValue(new Error("read-only settings"));
    const apply = vi.spyOn(pluginManager, "applyRevocations").mockResolvedValue();
    const liveIndex = JSON.stringify({
      schemaVersion: 1,
      generatedAt: "2026-07-17T00:04:00Z",
      plugins: [],
      themes: [],
      revocations: [{
        id: "page.tine.non-durable-live",
        version: "1.0.0",
        severity: "high",
        reason: "Remain safe in memory.",
        revokedAt: "2026-07-17T00:04:00Z",
      }],
    });
    vi.stubGlobal("fetch", vi.fn(async (url: string | URL | Request) =>
      new Response(String(url).endsWith(".sig") ? "live-signature" : liveIndex)
    ));

    await refreshCommunityRegistry({ timeoutMs: 100 });

    expect(apply).toHaveBeenCalledWith(new Set(["page.tine.non-durable-live@1.0.0"]));
    expect(registryState()).toBe("ready");
    expect(registryPersistenceError()).toContain("read-only settings");
  });

  it("does not let an older verified refresh overwrite the newer cached registry", async () => {
    const api = backend();
    vi.spyOn(api, "verifyPluginRegistry").mockResolvedValue();
    const apply = vi.spyOn(pluginManager, "applyRevocations").mockResolvedValue();
    const stored: Array<{ indexJson: string; signature: string }> = [];
    vi.spyOn(api, "storePluginRegistryCache").mockImplementation(async (indexJson, signature) => {
      stored.push({ indexJson, signature });
    });
    const oldIndex = JSON.stringify({
      schemaVersion: 1,
      generatedAt: "2026-07-17T00:01:00Z",
      plugins: [],
      themes: [],
      revocations: [],
    });
    const newIndex = JSON.stringify({
      schemaVersion: 1,
      generatedAt: "2026-07-17T00:02:00Z",
      plugins: [],
      themes: [],
      revocations: [{
        id: "page.tine.newer-cache",
        version: "1.0.0",
        severity: "high",
        reason: "The newer response must remain restart-durable.",
        revokedAt: "2026-07-17T00:02:00Z",
      }],
    });
    const pending = Array.from({ length: 4 }, () => {
      let resolve!: (response: Response) => void;
      const promise = new Promise<Response>((done) => { resolve = done; });
      return { promise, resolve };
    });
    let request = 0;
    vi.stubGlobal("fetch", vi.fn(() => pending[request++].promise));

    const older = refreshCommunityRegistry({ timeoutMs: 1_000 });
    const newer = refreshCommunityRegistry({ timeoutMs: 1_000 });
    pending[2].resolve(new Response(newIndex));
    pending[3].resolve(new Response("new-signature"));
    await newer;
    pending[0].resolve(new Response(oldIndex));
    pending[1].resolve(new Response("old-signature"));
    await older;
    await vi.waitFor(() => expect(stored.at(-1)?.signature).toBe("new-signature"));

    expect(stored).toEqual([{ indexJson: newIndex, signature: "new-signature" }]);
    const newerRevocations = new Set(["page.tine.newer-cache@1.0.0"]);
    expect(apply).toHaveBeenCalledTimes(1);
    expect(apply).toHaveBeenCalledWith(newerRevocations);
    expect(revokedThemeVersions()).toEqual(newerRevocations);
  });
});
