// Clipboard image ingress: a bounded decode of a native clipboard image to PNG
// bytes, and the asset ingress byte limit the asset IPC shares.

export const CLIPBOARD_IMAGE_MAX_PIXELS = 32 * 1024 * 1024;
export const CLIPBOARD_IMAGE_MAX_RGBA_BYTES = 128 * 1024 * 1024;
export const ASSET_INGRESS_MAX_BYTES = 64 * 1024 * 1024;

type ClipboardImage = {
  size(): Promise<{ width: number; height: number }>;
  rgba(): Promise<Uint8Array>;
};

export async function clipboardImageToPng(img: ClipboardImage): Promise<Uint8Array | null> {
  // Dimensions are metadata: validate them before asking the native plugin to
  // materialize an attacker-controlled RGBA allocation on the WebView thread.
  const { width, height } = await img.size();
  if (!Number.isSafeInteger(width) || !Number.isSafeInteger(height) || width <= 0 || height <= 0) return null;
  const pixels = width * height;
  const rgbaBytes = pixels * 4;
  if (!Number.isSafeInteger(pixels) || pixels > CLIPBOARD_IMAGE_MAX_PIXELS
      || rgbaBytes > CLIPBOARD_IMAGE_MAX_RGBA_BYTES) {
    return null;
  }
  const rgba = await img.rgba();
  if (rgba.byteLength !== rgbaBytes) return null;
  const clamped = new Uint8ClampedArray(rgba.buffer as ArrayBuffer, rgba.byteOffset, rgba.byteLength);
  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  const ctx = canvas.getContext("2d");
  if (!ctx) return null;
  ctx.putImageData(new ImageData(clamped, width, height), 0, 0);
  const blob: Blob | null = await new Promise((resolve) => canvas.toBlob(resolve, "image/png"));
  if (!blob || blob.size > ASSET_INGRESS_MAX_BYTES) return null;
  const encoded = await blob.arrayBuffer();
  return encoded.byteLength <= ASSET_INGRESS_MAX_BYTES ? new Uint8Array(encoded) : null;
}
