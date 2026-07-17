import { afterEach, describe, expect, it, vi } from "vitest";
import {
  CLIPBOARD_IMAGE_MAX_PIXELS,
  clipboardImageToPng,
} from "./backend";

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("clipboard image ingress budget", () => {
  it("rejects oversized dimensions before materializing native RGBA", async () => {
    const rgba = vi.fn<() => Promise<Uint8Array>>();
    const result = await clipboardImageToPng({
      size: vi.fn().mockResolvedValue({ width: CLIPBOARD_IMAGE_MAX_PIXELS + 1, height: 1 }),
      rgba,
    });
    expect(result).toBeNull();
    expect(rgba).not.toHaveBeenCalled();
  });

  it("validates exact RGBA size and reuses its buffer for ImageData", async () => {
    const rgba = new Uint8Array(16);
    let received: Uint8ClampedArray | undefined;
    vi.stubGlobal("ImageData", class {
      constructor(data: Uint8ClampedArray) { received = data; }
    });
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue({ putImageData: vi.fn() } as any);
    vi.spyOn(HTMLCanvasElement.prototype, "toBlob").mockImplementation((cb) => cb({
      size: 4,
      arrayBuffer: async () => new Uint8Array([1, 2, 3, 4]).buffer,
    } as Blob));

    const result = await clipboardImageToPng({
      size: vi.fn().mockResolvedValue({ width: 2, height: 2 }),
      rgba: vi.fn().mockResolvedValue(rgba),
    });
    expect(received?.buffer).toBe(rgba.buffer);
    expect(result).toEqual(new Uint8Array([1, 2, 3, 4]));
  });
});
