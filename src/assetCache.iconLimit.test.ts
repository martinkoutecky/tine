import { afterEach, beforeEach, expect, it, vi } from "vitest";
const mock = vi.hoisted(() => ({ readAsset: vi.fn() }));
vi.mock("./backend", () => ({ backend: () => mock }));
import { acquireAssetBlob, acquireIconAssetBlob, clearAssetBlobCache } from "./assetCache";

// GH #346 review r2: an icon read (1 MiB limit) that starts first must not
// refuse a concurrent block image of the same file its 32 MiB allowance.
const png = Uint8Array.from(atob("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aFOkAAAAASUVORK5CYII="), c => c.charCodeAt(0));
const paddedPng = (size: number) => { const bytes = new Uint8Array(size); bytes.set(png); return bytes; };
const twoMiB = paddedPng(2 * 1024 * 1024);

beforeEach(() => {
  mock.readAsset.mockReset();
  let serial = 0;
  vi.stubGlobal("URL", { createObjectURL: vi.fn(() => `blob:limit-${++serial}`), revokeObjectURL: vi.fn() });
});
afterEach(() => { clearAssetBlobCache(); vi.unstubAllGlobals(); });

it("a block image still loads when an icon read of the same file is refused first (backend refusal)", async () => {
  mock.readAsset.mockImplementation(async (_name: string, maxBytes: number) => {
    if (twoMiB.byteLength > maxBytes) throw "asset-too-large";
    return twoMiB;
  });
  const [icon, block] = await Promise.all([acquireIconAssetBlob("shared.png"), acquireAssetBlob("shared.png")]);
  expect(icon.url).toBe("");
  expect(block.url).not.toBe("");
  icon.release(); block.release();
});

it("a block image still loads when an icon read of the same file is refused first (length check)", async () => {
  mock.readAsset.mockResolvedValue(twoMiB);
  const [icon, block] = await Promise.all([acquireIconAssetBlob("shared.png"), acquireAssetBlob("shared.png")]);
  expect(icon.url).toBe("");
  expect(block.url).not.toBe("");
  icon.release(); block.release();
});
