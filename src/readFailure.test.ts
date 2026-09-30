import { afterEach, beforeEach, expect, it, vi } from "vitest";

const api = vi.hoisted(() => ({ getBlockRefCounts: vi.fn(), resolveBlocks: vi.fn(), graphBindingGeneration: () => 1 }));
vi.mock("./backend", () => ({ backend: () => api }));
vi.mock("./warmCache", () => ({ waitForWarmCache: async () => true }));
vi.mock("./document", () => ({ blockExternalId: (id: string) => id, blockRef: vi.fn(), node: () => null, resolveGuideBlockRef: () => null }));
vi.mock("./debug", () => ({ dbg: vi.fn() }));

beforeEach(() => { vi.resetModules(); api.getBlockRefCounts.mockReset(); api.resolveBlocks.mockReset(); });
afterEach(() => { vi.restoreAllMocks(); });

it("keeps last-good reference counts and reports a failed refresh", async () => {
  api.getBlockRefCounts.mockResolvedValue({ target: 3 });
  const { blockRefCount } = await import("./blockRefCounts");
  const { toasts, setToasts } = await import("./toasts");
  const { bumpDataRev } = await import("./graphSession");
  setToasts([]);
  await vi.waitFor(() => expect(blockRefCount("target")).toBe(3));
  api.getBlockRefCounts.mockRejectedValue(new Error("io:PermissionDenied"));
  bumpDataRev();
  await vi.waitFor(() => expect(api.getBlockRefCounts).toHaveBeenCalledTimes(2));
  await new Promise((resolve) => setTimeout(resolve, 0));
  expect(blockRefCount("target")).toBe(3);
  expect(toasts().some((t) => t.kind === "error")).toBe(true);
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
