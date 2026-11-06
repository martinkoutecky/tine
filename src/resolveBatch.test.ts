import { afterEach, expect, it, vi } from "vitest";
import { createComputed, createRoot } from "solid-js";
import { backend } from "./backend";
import * as changeKeys from "./changeKeys";
import { bumpGraphEpoch } from "./graphSession";
import { resolveBlockBatched, resolutionCacheStats } from "./resolveBatch";

const disposers: (() => void)[] = [];
const settle = async () => { for (let i = 0; i < 15; i++) await Promise.resolve(); };
afterEach(() => { while (disposers.length) disposers.pop()!(); bumpGraphEpoch(); vi.restoreAllMocks(); });

it.each([8, 32])("GH #886: K answer arrivals do only K membership checks, K=%i", async K => {
  vi.spyOn(backend(), "resolveBlocks").mockImplementation(async ids => ids.map(() => null));
  const probe = vi.spyOn(changeKeys, "changeKeyHasConsumers");
  const dispose = createRoot(d => {
    for (let i = 0; i < K; i++) createComputed(() => changeKeys.changed(`block:ref${i}`));
    return d;
  });
  disposers.push(dispose);
  probe.mockClear();
  for (let i = 0; i < K; i++) await resolveBlockBatched(`ref${i}`);
  expect(probe.mock.calls.length).toBe(K);
  expect(resolutionCacheStats().entries).toBe(K);
});

it.each([8, 32])("GH #886: releasing K keys does only K membership checks, K=%i", async K => {
  vi.spyOn(backend(), "resolveBlocks").mockImplementation(async ids => ids.map(() => null));
  const dispose = createRoot(d => {
    for (let i = 0; i < K; i++) createComputed(() => changeKeys.changed(`block:ref${i}`));
    return d;
  });
  disposers.push(dispose);
  await Promise.all(Array.from({ length: K }, (_, i) => resolveBlockBatched(`ref${i}`)));
  const probe = vi.spyOn(changeKeys, "changeKeyHasConsumers");
  dispose(); disposers.pop(); await settle();
  expect(probe.mock.calls.length).toBe(K);
});

it("GH #886: inactive eviction follows last access even when readers release in reverse order", async () => {
  vi.spyOn(backend(), "resolveBlocks").mockImplementation(async ids => ids.map(() => null));
  const readers = Array.from({ length: 514 }, (_, i) => createRoot(d => {
    createComputed(() => changeKeys.changed(`block:lru${i}`)); return d;
  }));
  disposers.push(...readers);
  const answers = await Promise.all(readers.map((_, i) => resolveBlockBatched(`lru${i}`)));
  expect(answers).toHaveLength(514);
  const touched = resolveBlockBatched("lru0");
  for (const dispose of readers.reverse()) dispose();
  disposers.length = 0; await settle();
  expect(resolutionCacheStats().entries).toBe(512);
  expect(resolveBlockBatched("lru0")).toBe(touched);
  const read = vi.mocked(backend().resolveBlocks); read.mockClear();
  await resolveBlockBatched("lru1");
  expect(read).toHaveBeenCalledExactlyOnceWith(["lru1"]);
});

it("GH #886: acquiring a cached inactive answer protects it during later admission", async () => {
  const read = vi.spyOn(backend(), "resolveBlocks").mockImplementation(async ids => ids.map(() => null));
  const pinned = resolveBlockBatched("pinned"); await pinned;
  disposers.push(createRoot(d => { createComputed(() => changeKeys.changed("block:pinned")); return d; }));
  await Promise.all(Array.from({ length: 513 }, (_, i) => resolveBlockBatched(`extra${i}`)));
  expect(resolutionCacheStats().entries).toBe(513);
  expect(resolveBlockBatched("pinned")).toBe(pinned);
  expect(read).toHaveBeenCalledTimes(2);
});

it("GH #886: an answer whose key an imperative reader kept still becomes evictable", async () => {
  const read = vi.spyOn(backend(), "resolveBlocks").mockImplementation(async ids => ids.map(() => null));
  const dispose = createRoot(d => { createComputed(() => changeKeys.changed("block:kept")); return d; });
  await resolveBlockBatched("kept");
  dispose(); changeKeys.changed("block:kept"); await settle();
  await Promise.all(Array.from({ length: 512 }, (_, i) => resolveBlockBatched(`fill${i}`)));
  expect(resolutionCacheStats().entries).toBe(512);
  read.mockClear();
  await resolveBlockBatched("kept");
  expect(read).toHaveBeenCalledExactlyOnceWith(["kept"]);
});
