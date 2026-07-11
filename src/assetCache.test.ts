import { afterEach, describe, expect, it, vi } from "vitest";

const backendMock = vi.hoisted(() => ({ readAsset: vi.fn() }));
vi.mock("./backend", () => ({ backend: () => backendMock }));

import { __assetCacheStatsForTests, acquireAssetBlob, clearAssetBlobCache, seedAssetBlob } from "./assetCache";

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
