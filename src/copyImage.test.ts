import { afterEach, expect, it, vi } from "vitest";
import { readFileSync } from "node:fs";
import { copyImageFromSrc } from "./copyImage";
import { backend } from "./backend";
import { toasts, setToasts } from "./toasts";
import { seedAssetBlob, clearAssetBlobCache } from "./assetCache";

afterEach(() => { vi.restoreAllMocks(); vi.unstubAllGlobals(); setToasts([]); });

it("copies a graph asset as a file without fetching image bytes", async () => {
  vi.stubGlobal("URL", { createObjectURL: () => "blob:graph-photo", revokeObjectURL: vi.fn() });
  const src = seedAssetBlob("original.jpg", new Uint8Array([1, 2, 3]));
  const fileCopy = vi.spyOn(backend(), "copyAssetFileToClipboard").mockResolvedValue(true);
  const fetch = vi.fn().mockRejectedValue(new Error("must not fetch local assets"));
  vi.stubGlobal("fetch", fetch);
  try {
    await copyImageFromSrc(src);
    expect(fileCopy).toHaveBeenCalledWith("original.jpg");
    expect(fetch).not.toHaveBeenCalled();
  } finally { clearAssetBlobCache(); }
});

it("offers an unloaded large asset without reading its bytes", async () => {
  const fileCopy = vi.spyOn(backend(), "copyAssetFileToClipboard").mockResolvedValue(true);
  const fetch = vi.fn();
  vi.stubGlobal("fetch", fetch);
  await copyImageFromSrc("", "large.tiff");
  expect(fileCopy).toHaveBeenCalledWith("large.tiff");
  expect(fetch).not.toHaveBeenCalled();
});

it("keeps the image-byte fallback on mobile", async () => {
  vi.spyOn(backend(), "copyAssetFileToClipboard").mockResolvedValue(false);
  const bytes = new Uint8Array([1]);
  vi.stubGlobal("fetch", vi.fn().mockResolvedValue({ ok: true, blob: async () => ({ type: "image/png", size: 1, arrayBuffer: async () => bytes.buffer }) }));
  const write = vi.spyOn(backend(), "copyImageToClipboard").mockResolvedValue();
  await copyImageFromSrc("blob:mobile", "photo.png");
  expect(write).toHaveBeenCalledWith(bytes);
});

it.each(["image/png", "image/jpeg"])("copies original %s bytes without a canvas round trip", async (type) => {
  const bytes = new Uint8Array([1, 2, 3]);
  vi.stubGlobal("fetch", vi.fn().mockResolvedValue({ ok: true, blob: async () => ({ type, size: 3, arrayBuffer: async () => bytes.buffer }) }));
  const write = vi.spyOn(backend(), "copyImageToClipboard").mockResolvedValue();
  await copyImageFromSrc("blob:photo");
  expect(write).toHaveBeenCalledWith(bytes);
  expect(toasts()).toEqual([]);
});

it("converts other browser formats to PNG once before native copy", async () => {
  vi.stubGlobal("fetch", vi.fn().mockResolvedValue({ ok: true, blob: async () => ({ type: "image/webp", size: 3 }) }));
  const decode = vi.fn().mockResolvedValue(undefined);
  vi.stubGlobal("Image", class { src = ""; naturalWidth = 2; naturalHeight = 3; decode = decode; });
  const drawImage = vi.fn();
  const bytes = new Uint8Array([4, 5, 6]);
  const toBlob = vi.fn((done: (blob: unknown) => void) => done({ arrayBuffer: async () => bytes.buffer }));
  const canvas = { width: 0, height: 0, getContext: () => ({ drawImage }), toBlob };
  vi.stubGlobal("document", { createElement: vi.fn().mockReturnValue(canvas) });
  const write = vi.spyOn(backend(), "copyImageToClipboard").mockResolvedValue();
  await copyImageFromSrc("blob:webp");
  expect(decode).toHaveBeenCalledOnce();
  expect(drawImage).toHaveBeenCalledOnce();
  expect(toBlob).toHaveBeenCalledOnce();
  expect(toBlob).toHaveBeenCalledWith(expect.any(Function), "image/png");
  expect(write).toHaveBeenCalledWith(bytes);
});

it("shows progress before fetching and removes it on failure", async () => {
  vi.stubGlobal("fetch", vi.fn().mockImplementation(async () => {
    expect(toasts().some((toast) => toast.message === "Copying image…")).toBe(true);
    throw new Error("offline");
  }));
  await expect(copyImageFromSrc("https://example.com/photo")).rejects.toThrow("offline");
  expect(toasts()).toEqual([]);
});

it("rejects an oversized source before reading or sending its bytes", async () => {
  const arrayBuffer = vi.fn();
  vi.stubGlobal("fetch", vi.fn().mockResolvedValue({ ok: true, blob: async () => ({ type: "image/png", size: 64 * 1024 * 1024 + 1, arrayBuffer }) }));
  const write = vi.spyOn(backend(), "copyImageToClipboard").mockResolvedValue();
  await expect(copyImageFromSrc("blob:large")).rejects.toThrow("64 MiB");
  expect(arrayBuffer).not.toHaveBeenCalled();
  expect(write).not.toHaveBeenCalled();
  expect(toasts()).toEqual([]);
});

it("uses bounded raw IPC and keeps native conversion off the UI thread", () => {
  const config = JSON.parse(readFileSync("src-tauri/tauri.conf.json", "utf8"));
  const connect = config.app.security.csp["connect-src"].split(" ");
  expect(connect).toContain("blob:");
  expect(connect).toContain("data:");
  const backend = readFileSync("src/backend.ts", "utf8");
  const method = backend.slice(backend.indexOf("  async copyImageToClipboard("), backend.indexOf("  readHighlights("));
  expect(method).toContain('await this.appPlatform() === "android"');
  expect(method).toContain('bytesB64: bytesToBase64(bytes)');
  expect(method).toContain('invoke<void>("copy_image_to_clipboard", bytes)');
  const native = readFileSync("src-tauri/src/platform.rs", "utf8");
  expect(native).toContain("tauri::ipc::InvokeBody::Raw");
  expect(native).toContain("spawn_blocking(move ||");
  expect(native).toContain("64 * 1024 * 1024");
  const command = native.slice(native.indexOf("pub(crate) async fn copy_image_to_clipboard"), native.indexOf("fn decode_clipboard_image"));
  expect(command).not.toContain("Image::from_bytes");
  expect(command).toContain("Image::new_owned(rgba.into_raw(), width, height)");
});
