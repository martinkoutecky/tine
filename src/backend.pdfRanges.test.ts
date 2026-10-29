import { afterEach, expect, it, vi } from "vitest";

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke, convertFileSrc: (path: string) => path }));
afterEach(() => { vi.unstubAllGlobals(); vi.resetModules(); invoke.mockReset(); });

it("pins metadata and every binary range to the original graph binding", async () => {
  vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
  invoke.mockImplementation(async (cmd: string) => cmd === "stat_asset" ? { length: 34_000_000, version: "snapshot" }
    : cmd === "read_asset_range" ? new Uint8Array([7, 8]).buffer : null);
  const { backend } = await import("./backend");
  const source = await backend().pdfAssetSource!("sub/book.pdf", 268_435_456, 17);
  expect(source.length).toBe(34_000_000);
  expect(await source.read(5, 7)).toEqual(new Uint8Array([7, 8]));
  expect(invoke).toHaveBeenCalledWith("stat_asset", { name: "sub/book.pdf", maxBytes: 268_435_456, bindingGeneration: 17 });
  expect(invoke).toHaveBeenCalledWith("read_asset_range", {
    name: "sub/book.pdf", maxBytes: 268_435_456, begin: 5, end: 7, version: "snapshot", bindingGeneration: 17,
  });
  await expect(backend().pdfAssetSource!("sub/book.pdf", 268_435_456, 0)).rejects.toThrow("missing-graph-binding");
});
