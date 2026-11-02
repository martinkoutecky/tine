// Graph assets copy as native files on desktop. Other sources and mobile keep
// the bounded image-byte route, with canvas fallback for other browser formats.
// Shared by inline and lightbox copy buttons.

import { writeClipboardAssetFile, writeClipboardImage } from "./clipboard";
import { assetNameForBlobUrl } from "./assetCache";
import { ASSET_INGRESS_MAX_BYTES, CLIPBOARD_IMAGE_MAX_PIXELS } from "./clipboardImage";
import { dismissToast, pushToast } from "./toasts";
import { msg } from "./i18n";

const TOO_MANY_BYTES = "image exceeds 64 MiB clipboard limit";

export async function copyImageFromSrc(src: string, assetName = assetNameForBlobUrl(src)): Promise<void> {
  const progress = pushToast(msg("image-copy-progress"), "info", { sticky: true });
  try {
    if (assetName !== undefined) {
      if (await writeClipboardAssetFile(assetName)) return;
    }
    const response = await fetch(src);
    if (!response.ok) throw new Error("image fetch failed");
    // A declared length refuses before the body is read; the blob's own size
    // covers a response that declares none.
    if (Number(response.headers?.get("content-length")) > ASSET_INGRESS_MAX_BYTES) throw new Error(TOO_MANY_BYTES);
    const source = await response.blob();
    if (source.size > ASSET_INGRESS_MAX_BYTES) throw new Error(TOO_MANY_BYTES);
    if (source.type === "image/png" || source.type === "image/jpeg") {
      await writeClipboardImage(new Uint8Array(await source.arrayBuffer()));
      return;
    }
    await copyViaCanvas(src);
  } finally {
    dismissToast(progress);
  }
}

async function copyViaCanvas(src: string): Promise<void> {
  const img = new Image();
  img.src = src;
  await img.decode();
  // The same admission as clipboardImageToPng: an SVG's intrinsic size can far
  // exceed what is on screen, and its canvas costs 4 bytes per pixel.
  if (img.naturalWidth * img.naturalHeight > CLIPBOARD_IMAGE_MAX_PIXELS) throw new Error("image is too large to copy");
  const canvas = document.createElement("canvas");
  canvas.width = img.naturalWidth;
  canvas.height = img.naturalHeight;
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("no 2d context");
  ctx.drawImage(img, 0, 0);
  const blob: Blob | null = await new Promise((r) => canvas.toBlob(r, "image/png"));
  if (!blob) throw new Error("encode failed");
  if (blob.size > ASSET_INGRESS_MAX_BYTES) throw new Error(TOO_MANY_BYTES);
  await writeClipboardImage(new Uint8Array(await blob.arrayBuffer()));
}
