import { readFileSync } from "node:fs";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { InlineText } from "./render/inline";
import { initParser } from "./render/parse";
import { backend, type AssetChangedBatch } from "./backend";
import { __assetCacheStatsForTests, assetVersion, clearAssetBlobCache, seedAssetBlob } from "./assetCache";
import { subscribeAssetChanges } from "./assetRefresh";

// og-J2 (master d017d1afc, 2f54a8d5e): an image replaced on disk by an editor,
// a synchronizer or another window refreshes in place. The native watcher emits
// `asset-changed` with assets-relative paths; this window drops the cached blob
// and the mounted <img> re-reads the new bytes.

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  clearAssetBlobCache();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

function stubBlobUrls(): void {
  let next = 0;
  vi.stubGlobal("URL", {
    ...URL,
    createObjectURL: vi.fn(() => `blob:image-${++next}`),
    revokeObjectURL: vi.fn(),
  });
}

/** Subscribe as the app does and hand back the native event emitter. */
async function subscribeToNative(): Promise<{ emit: (batch: AssetChangedBatch) => void; stop: () => void }> {
  let deliver: (batch: AssetChangedBatch) => void = () => {};
  vi.spyOn(backend(), "onAssetChanged").mockImplementation(async (cb) => {
    deliver = cb;
    return () => {};
  });
  const stop = subscribeAssetChanges();
  await vi.waitFor(() => expect(backend().onAssetChanged).toHaveBeenCalled());
  return { emit: (batch) => deliver(batch), stop };
}

function mount(raw: string): { host: HTMLElement; dispose: () => void } {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const dispose = render(() => <InlineText text={raw} format="md" />, host);
  return { host, dispose: () => { dispose(); host.remove(); } };
}

describe("external asset refresh", () => {
  it("re-reads and re-renders a displayed image when the watcher reports its replacement", async () => {
    stubBlobUrls();
    const read = vi.spyOn(backend(), "readAsset")
      .mockResolvedValueOnce(new Uint8Array([1, 1, 1]))
      .mockResolvedValue(new Uint8Array([2, 2, 2, 2]));
    const { emit, stop } = await subscribeToNative();
    const { host, dispose } = mount("![pic](../assets/sub/pic.png)");
    try {
      await vi.waitFor(() => expect(host.querySelector("img.inline-image")?.getAttribute("src")).toBe("blob:image-1"));
      expect(read).toHaveBeenCalledTimes(1);

      emit({ paths: ["sub/pic.png"], binding_generation: backend().graphBindingGeneration() });

      await vi.waitFor(() => expect(host.querySelector("img.inline-image")?.getAttribute("src")).toBe("blob:image-2"));
      expect(read).toHaveBeenCalledTimes(2);
    } finally {
      stop();
      dispose();
    }
  });

  it("shows the missing placeholder when the watcher reports the file deleted", async () => {
    stubBlobUrls();
    vi.spyOn(backend(), "readAsset")
      .mockResolvedValueOnce(new Uint8Array([1]))
      .mockRejectedValue(new Error("asset is gone"));
    const { emit, stop } = await subscribeToNative();
    const { host, dispose } = mount("![gone](../assets/gone.png)");
    try {
      await vi.waitFor(() => expect(host.querySelector("img.inline-image")).not.toBeNull());
      emit({ paths: ["gone.png"], binding_generation: backend().graphBindingGeneration() });
      await vi.waitFor(() => expect(host.querySelector(".inline-image-missing")).not.toBeNull());
      expect(host.querySelector("img.inline-image")).toBeNull();
    } finally {
      stop();
      dispose();
    }
  });

  it("drops a batch from another graph binding and never bumps an unrelated asset", async () => {
    const { emit, stop } = await subscribeToNative();
    try {
      const before = assetVersion("other-graph.png");
      const generation = backend().graphBindingGeneration();
      emit({ paths: ["other-graph.png"], binding_generation: generation + 1 });
      expect(assetVersion("other-graph.png")).toBe(before);
      emit({ paths: ["mine.png"], binding_generation: generation });
      expect(assetVersion("mine.png")).toBe(1);
      expect(assetVersion("other-graph.png")).toBe(before);
    } finally {
      stop();
    }
  });

  it("invalidates but never hot-swaps an open PDF, audio or video asset", async () => {
    const { emit, stop } = await subscribeToNative();
    try {
      const generation = backend().graphBindingGeneration();
      const deferred = ["paper.pdf", "talk.mp3", "clip.MP4"];
      const before = deferred.map(assetVersion);
      stubBlobUrls();
      for (const name of deferred) seedAssetBlob(name, new Uint8Array([1, 2]));
      expect(__assetCacheStatsForTests().entries).toBe(3);
      emit({ paths: [...deferred, "photo.jpeg", "photo.jpeg"], binding_generation: generation });
      expect(deferred.map(assetVersion)).toEqual(before);
      expect(__assetCacheStatsForTests().entries, "stale bytes must not be served on the next open").toBe(0);
      expect(assetVersion("photo.jpeg")).toBe(1);
    } finally {
      stop();
    }
  });

  it("stops listening when the window's subscription is released", async () => {
    const release = vi.fn();
    vi.spyOn(backend(), "onAssetChanged").mockResolvedValue(release);
    const stop = subscribeAssetChanges();
    await vi.waitFor(() => expect(backend().onAssetChanged).toHaveBeenCalled());
    await Promise.resolve();
    stop();
    expect(release).toHaveBeenCalledTimes(1);
  });

  it("is subscribed once by the app shell and released with it", () => {
    const shell = readFileSync("src/App.tsx", "utf8");
    expect(shell.match(/onCleanup\(subscribeAssetChanges\(\)\)/g) ?? []).toHaveLength(1);
  });
});
