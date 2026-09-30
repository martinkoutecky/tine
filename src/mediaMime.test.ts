import { afterEach, expect, it, vi } from "vitest";
import * as wasm from "./render/wasm/lsdoc_wasm";
import fixtures from "../crates/tine-core/tests/fixtures/media-mime.json";
import { acquireAssetBlob, acquireLocalImageBlob, clearAssetBlobCache, seedAssetBlob } from "./assetCache";
import { backend } from "./backend";

afterEach(() => { clearAssetBlobCache(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

it("native and wasm share every asset MIME mapping, including case and unknown extensions", () => {
  for (const [ext, mime] of fixtures) {
    expect(wasm.mime_from_path(`nested/voice.${ext}`)).toBe(mime);
    expect(wasm.mime_from_path(`žluťoučký/VOICE.${ext.toUpperCase()}`)).toBe(mime);
  }
});

it("seeded, graph-read and opted-in local blobs ask the shared wasm MIME answerer", async () => {
  const blobs: Blob[] = [];
  vi.stubGlobal("URL", { createObjectURL: (blob: Blob) => { blobs.push(blob); return `blob:${blobs.length}`; }, revokeObjectURL: vi.fn() });
  const mime = vi.spyOn(wasm, "mime_from_path").mockReturnValue("fixture/shared-answer");
  vi.spyOn(backend(), "readAsset").mockResolvedValue(new Uint8Array([1]));
  vi.spyOn(backend(), "readLocalImage").mockResolvedValue(new Uint8Array([2]));
  seedAssetBlob("seed.png", new Uint8Array([3]));
  const graph = await acquireAssetBlob("nested/movie.mkv");
  const local = await acquireLocalImageBlob("/tmp/photo.JPG");
  expect(mime.mock.calls.map(([path]) => path)).toEqual(["seed.png", "nested/movie.mkv", "/tmp/photo.JPG"]);
  expect(blobs.map(blob => blob.type)).toEqual(Array(3).fill("fixture/shared-answer"));
  graph.release(); local.release();
});
