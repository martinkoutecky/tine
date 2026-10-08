import { afterEach, describe, expect, it, vi } from "vitest";
import { setToasts, toasts } from "./toasts";

const backendMock = vi.hoisted(() => ({ readAsset: vi.fn() } as { readAsset: ReturnType<typeof vi.fn>; graphBindingGeneration?: () => number }));
vi.mock("./backend", () => ({ backend: () => backendMock }));

import { __assetCacheStatsForTests, acquireAssetBlob, assetVersion, refreshAsset, clearAssetBlobCache, seedAssetBlob } from "./assetCache";

afterEach(() => {
  clearAssetBlobCache();
  vi.restoreAllMocks();
});

describe("asset blob cache bounds", () => {
  it("evicts least-recently-used image blobs beyond the entry cap", () => {
    let next = 0;
    vi.stubGlobal("URL", {
      ...URL,
      createObjectURL: () => `blob:test-${next++}`,
      revokeObjectURL: vi.fn(),
    });
    for (let i = 0; i < 160; i++) seedAssetBlob(`image-${i}.png`, new Uint8Array([i]));
    expect(__assetCacheStatsForTests()).toEqual({ entries: 128, bytes: 128 });
  });

  it("L09:47: asset versions end with the graph cache", () => {
    refreshAsset("old-graph/image.png");
    expect(assetVersion("old-graph/image.png")).toBe(1);
    clearAssetBlobCache();
    expect(assetVersion("old-graph/image.png")).toBe(0);
    refreshAsset("new-graph/image.png");
    expect(assetVersion("new-graph/image.png")).toBe(1);
  });

  it("runs at most two distinct image reads concurrently", async () => {
    vi.stubGlobal("URL", {
      createObjectURL: vi.fn((_: Blob) => `blob:test-${Math.random()}`),
      revokeObjectURL: vi.fn(),
    });
    let active = 0;
    let peak = 0;
    backendMock.readAsset.mockImplementation(async () => {
      active++;
      peak = Math.max(peak, active);
      await new Promise((resolve) => setTimeout(resolve, 5));
      active--;
      return new Uint8Array([1]);
    });
    const leases = await Promise.all(Array.from({ length: 10 }, (_, i) => acquireAssetBlob(`queued-${i}.png`)));
    expect(peak).toBe(2);
    leases.forEach((lease) => lease.release());
  });

  it("does not revoke any of 129 requested image URLs before consumers release them", async () => {
    const revokeObjectURL = vi.fn();
    let next = 0;
    vi.stubGlobal("URL", {
      createObjectURL: vi.fn(() => `blob:held-${next++}`),
      revokeObjectURL,
    });
    backendMock.readAsset.mockResolvedValue(new Uint8Array([1]));
    const leases = await Promise.all(
      Array.from({ length: 129 }, (_, i) => acquireAssetBlob(`held-${i}.png`))
    );
    expect(leases.map((lease, i) => lease.url ? null : i).filter((i) => i !== null)).toEqual([]);
    expect(revokeObjectURL).not.toHaveBeenCalled();
    leases.forEach((lease) => lease.release());
  });

  it("reuses a live lease after its entry leaves the retained LRU", async () => {
    let next = 0;
    vi.stubGlobal("URL", {
      createObjectURL: vi.fn(() => `blob:live-${next++}`),
      revokeObjectURL: vi.fn(),
    });
    backendMock.readAsset.mockResolvedValue(new Uint8Array([1]));
    const leases = await Promise.all(
      Array.from({ length: 129 }, (_, i) => acquireAssetBlob(`live-${i}.png`))
    );
    const again = await acquireAssetBlob("live-0.png");
    expect(again.url).toBe(leases[0].url);
    expect(backendMock.readAsset.mock.calls.filter(([path]) => path === "live-0.png")).toHaveLength(1);
    again.release();
    leases.forEach((lease) => lease.release());
  });
});

describe("I-9: an asset read failure is not swallowed", () => {
  afterEach(() => setToasts([]));
  it("shows a sticky error when a graph asset cannot be read", async () => {
    backendMock.graphBindingGeneration = vi.fn(() => 1);
    backendMock.readAsset.mockRejectedValueOnce("io:PermissionDenied");
    const lease = await acquireAssetBlob("locked.png");
    expect(lease.url).toBe("");
    expect(toasts().some((t) => t.kind === "error" && t.sticky)).toBe(true);
    delete backendMock.graphBindingGeneration;
  });
  it("stays quiet for a missing asset: the broken-image placeholder is its visible form", async () => {
    backendMock.graphBindingGeneration = vi.fn(() => 1);
    backendMock.readAsset.mockRejectedValueOnce("not-found");
    const lease = await acquireAssetBlob("gone.png");
    expect(lease.url).toBe("");
    expect(toasts()).toEqual([]);
    delete backendMock.graphBindingGeneration;
  });
});
