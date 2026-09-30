import { afterEach, beforeEach, expect, it, vi } from "vitest";

const api = vi.hoisted(() => ({ getBlockRefCounts: vi.fn(), resolveBlocks: vi.fn(), graphBindingGeneration: () => 1 }));
vi.mock("./backend", () => ({ backend: () => api }));
vi.mock("./warmCache", () => ({ waitForWarmCache: async () => true }));
vi.mock("./document", () => ({ blockExternalId: (id: string) => id, blockRef: vi.fn(), node: () => null, resolveGuideBlockRef: () => null }));
vi.mock("./debug", () => ({ dbg: vi.fn() }));

beforeEach(() => { vi.resetModules(); api.getBlockRefCounts.mockReset(); api.resolveBlocks.mockReset(); });
afterEach(() => { vi.restoreAllMocks(); });

it("keeps native last-good counts and reports a failed initial snapshot read", async () => {
  let reject!: (error: Error) => void;
  api.getBlockRefCounts.mockImplementation(() => new Promise((_, fail) => { reject = fail; }));
  const { blockRefCount } = await import("./blockRefCounts");
  const { applyGraphAnswers } = await import("./graphAnswers");
  const { toasts, setToasts } = await import("./toasts");
  setToasts([]);
  await vi.waitFor(() => expect(api.getBlockRefCounts).toHaveBeenCalledTimes(1));
  applyGraphAnswers({ rev: "2", inventoryChanged: false, blockRefCounts: { target: 3 } });
  expect(blockRefCount("target")).toBe(3);
  reject(new Error("io:PermissionDenied"));
  await vi.waitFor(() => expect(toasts().some((t) => t.kind === "error")).toBe(true));
  expect(blockRefCount("target")).toBe(3);
});

it("failed block resolution stays unknown, reports failure and retries in the same revision", async () => {
  api.resolveBlocks.mockRejectedValueOnce(new Error("io:PermissionDenied")).mockResolvedValueOnce([null]);
  const { resolveBlockBatched } = await import("./resolveBatch");
  const { toasts, setToasts } = await import("./toasts");
  setToasts([]);
  expect(await resolveBlockBatched("target")).toBeUndefined();
  expect(toasts().some((t) => t.kind === "error")).toBe(true);
  expect(await resolveBlockBatched("target")).toBeNull();
  expect(api.resolveBlocks).toHaveBeenCalledTimes(2);
});

it("reference-count owner construction failures report without escaping module loading", async () => {
  const owned = await import("./owned");
  vi.spyOn(owned, "latestOwner").mockImplementation(() => { throw new Error("owner lookup failed"); });
  const { toasts, setToasts } = await import("./toasts");
  setToasts([]);
  await import("./blockRefCounts");
  await vi.waitFor(() => expect(toasts().some((toast) => toast.kind === "error")).toBe(true));
  expect(api.getBlockRefCounts).not.toHaveBeenCalled();
});
