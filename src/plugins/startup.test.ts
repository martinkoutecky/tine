import { afterEach, describe, expect, it, vi } from "vitest";
import { backend } from "../backend";
import { revokedThemeVersions } from "../themes/manager";
import { PluginRuntime } from "./runtime";
import { pluginManager } from "./manager";
import {
  loadVerifiedCachedRegistry,
  refreshCommunityRegistry,
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
  contributions: { commands: [{ id: "forbidden", label: "Forbidden cached command" }] },
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("community extension startup revocations", () => {
  it("verifies cached revocations before plugin activation while the live fetch stalls", async () => {
    const api = backend();
    vi.spyOn(api, "getAppString").mockImplementation(async (key, fallback) => {
      if (key === "plugin-registry-index") return cachedIndex;
      if (key === "plugin-registry-signature") return "valid-signature";
      if (key === "theme.packages.v1") return "[]";
      return fallback;
    });
    vi.spyOn(api, "setAppString").mockResolvedValue();
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
    expect(startup.initialRevocations).toEqual(new Set([revokedKey]));
    expect(verify).toHaveBeenCalledWith(cachedIndex, "valid-signature");
  });

  it("rejects invalid and absent cached registries without inventing revocations", async () => {
    const api = backend();
    const get = vi.spyOn(api, "getAppString");
    const verify = vi.spyOn(api, "verifyPluginRegistry");

    get.mockImplementation(async (key, fallback) => key === "plugin-registry-index"
      ? "not-json"
      : key === "plugin-registry-signature" ? "invalid-signature" : fallback);
    verify.mockRejectedValue(new Error("signature did not verify"));
    expect(await loadVerifiedCachedRegistry(100)).toBeNull();
    expect(seedCachedCommunityRegistry(null)).toEqual(new Set());
    expect(revokedThemeVersions()).toEqual(new Set());

    get.mockResolvedValue("");
    verify.mockClear();
    expect(await loadVerifiedCachedRegistry(100)).toBeNull();
    expect(verify).not.toHaveBeenCalled();
  });

  it("keeps a verified cached state when a streamed live body reaches its abort deadline", async () => {
    const api = backend();
    vi.spyOn(api, "getAppString").mockImplementation(async (key, fallback) => {
      if (key === "plugin-registry-index") return cachedIndex;
      if (key === "plugin-registry-signature") return "valid-signature";
      return fallback;
    });
    vi.spyOn(api, "verifyPluginRegistry").mockResolvedValue();
    const cached = await loadVerifiedCachedRegistry(100);
    expect(cached).not.toBeNull();
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
    vi.spyOn(api, "setAppString").mockResolvedValue();
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

  it("does not let an older verified refresh overwrite the newer cached registry", async () => {
    const api = backend();
    vi.spyOn(api, "verifyPluginRegistry").mockResolvedValue();
    const apply = vi.spyOn(pluginManager, "applyRevocations").mockResolvedValue();
    const stored = new Map<string, string>();
    vi.spyOn(api, "setAppString").mockImplementation(async (key, value) => {
      stored.set(key, value);
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
    await vi.waitFor(() => expect(stored.get("plugin-registry-signature")).toBe("new-signature"));

    expect(stored.get("plugin-registry-index")).toBe(newIndex);
    const newerRevocations = new Set(["page.tine.newer-cache@1.0.0"]);
    expect(apply).toHaveBeenCalledTimes(1);
    expect(apply).toHaveBeenCalledWith(newerRevocations);
    expect(revokedThemeVersions()).toEqual(newerRevocations);
  });
});
