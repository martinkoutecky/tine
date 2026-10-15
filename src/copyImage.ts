// Copy an image (by src — a blob:/data:/http URL) to the OS clipboard.
// Preserve PNG/JPEG bytes; native clipboard conversion runs on the worker pool.
// Other browser image formats keep the canvas fallback. Shared by both buttons.

import { writeClipboardImage } from "./clipboard";
import { ASSET_INGRESS_MAX_BYTES } from "./clipboardImage";
import { dismissToast, pushToast } from "./toasts";
import { msg } from "./i18n";

export async function copyImageFromSrc(src: string): Promise<void> {
  const progress = pushToast(msg("image-copy-progress"), "info", { sticky: true });
  try {
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
