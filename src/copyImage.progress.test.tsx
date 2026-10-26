import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { Toasts } from "./components/Toasts";
import { copyImageFromSrc } from "./copyImage";
import { backend } from "./backend";
import { setToasts } from "./toasts";

let dispose: (() => void) | undefined;
afterEach(() => {
  dispose?.();
  document.body.replaceChildren();
  setToasts([]);
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

it.each([false, true])("renders progress until the clipboard write settles (failure=%s)", async (fails) => {
  vi.stubGlobal("fetch", vi.fn().mockResolvedValue({
    ok: true,
    blob: async () => ({ type: "image/jpeg", size: 3, arrayBuffer: async () => new ArrayBuffer(3) }),
  }));
  let finish!: () => void;
  let reject!: (error: Error) => void;
  const write = vi.spyOn(backend(), "copyImageToClipboard").mockImplementation(() =>
    new Promise<void>((resolve, fail) => { finish = resolve; reject = fail; }));
  const host = document.createElement("div");
  document.body.append(host);
  dispose = render(() => <Toasts />, host);
  const pending = copyImageFromSrc("blob:photo");
  // Keep a failed pre-fix assertion from leaving a detached rejected promise.
  void pending.catch(() => {});
  expect(host.textContent).toContain("Copying image…");
  await vi.waitFor(() => expect(write).toHaveBeenCalled());
  expect(host.textContent).toContain("Copying image…");
  if (fails) {
    const result = expect(pending).rejects.toThrow("clipboard unavailable");
    reject(new Error("clipboard unavailable"));
    await result;
  } else {
    finish();
    await pending;
  }
  expect(host.textContent).not.toContain("Copying image…");
});

it("renders progress while the native asset file copy is pending", async () => {
  let finish!: (copied: boolean) => void;
  const copy = vi.spyOn(backend(), "copyAssetFileToClipboard").mockImplementation(() =>
    new Promise<boolean>((resolve) => { finish = resolve; }));
  const host = document.createElement("div");
  document.body.append(host);
  dispose = render(() => <Toasts />, host);
  const pending = copyImageFromSrc("", "large.tiff");
  expect(host.textContent).toContain("Copying image…");
  await vi.waitFor(() => expect(copy).toHaveBeenCalledWith("large.tiff"));
  finish(true);
  await pending;
  expect(host.textContent).not.toContain("Copying image…");
});
