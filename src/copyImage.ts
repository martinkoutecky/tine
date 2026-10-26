// Graph assets copy as native files on desktop. Other sources and mobile keep
// the bounded image-byte route, with canvas fallback for other browser formats.
// Shared by inline and lightbox copy buttons.

import { writeClipboardAssetFile, writeClipboardImage } from "./clipboard";
import { assetNameForBlobUrl } from "./assetCache";
import { ASSET_INGRESS_MAX_BYTES } from "./clipboardImage";
import { dismissToast, pushToast } from "./toasts";
import { msg } from "./i18n";

export async function copyImageFromSrc(src: string, assetName = assetNameForBlobUrl(src)): Promise<void> {
  const progress = pushToast(msg("image-copy-progress"), "info", { sticky: true });
  try {
    if (assetName !== undefined) {
      if (await writeClipboardAssetFile(assetName)) return;
    }
    const response = await fetch(src);
    if (!response.ok) throw new Error("image fetch failed");
    const source = await response.blob();
    if (source.size > ASSET_INGRESS_MAX_BYTES) throw new Error("image exceeds 64 MiB clipboard limit");
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
  const canvas = document.createElement("canvas");
  canvas.width = img.naturalWidth;
  canvas.height = img.naturalHeight;
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("no 2d context");
  ctx.drawImage(img, 0, 0);
  const blob: Blob | null = await new Promise((r) => canvas.toBlob(r, "image/png"));
  if (!blob) throw new Error("encode failed");
  await writeClipboardImage(new Uint8Array(await blob.arrayBuffer()));
}
