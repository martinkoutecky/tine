// Copy an image (by src — a blob:/data:/http URL) to the OS clipboard.
// WebKitGTK's native "Copy Image" doesn't populate the OS clipboard, so we
// re-encode the pixels to PNG bytes via a canvas and hand them to the Rust
// clipboard writer (`copy_image_to_clipboard`). Shared by the lightbox (Toasts)
// and the inline-asset hover action bar (render/inline.tsx).

import { writeClipboardImage } from "./clipboard";

export async function copyImageFromSrc(src: string): Promise<void> {
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
