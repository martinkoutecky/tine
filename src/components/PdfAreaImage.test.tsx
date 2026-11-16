import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { PdfAreaImage } from "./PdfAreaImage";
import { clearAssetBlobCache, refreshAsset } from "../assetCache";
import { setToasts, toasts } from "../toasts";

let dispose: (() => void) | undefined;
afterEach(() => {
  dispose?.();
  clearAssetBlobCache();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  document.body.replaceChildren();
  setToasts([]);
});

it("a missing crop stays a quiet placeholder and loads when sync delivers it", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  setToasts([]);
  const read = vi.spyOn(backend(), "readAsset").mockRejectedValueOnce(new Error("not-found"))
    .mockResolvedValue(new Uint8Array([137, 80, 78, 71]));
  vi.stubGlobal("URL", class extends URL {
    static createObjectURL() { return "blob:arrived-area"; }
    static revokeObjectURL() {}
  });
  dispose = render(() => <PdfAreaImage page="hls__book%20one" highlightId="area-id" hlPage={3} stamp="123" />, host);
  const path = "book%20one/3_area-id_123.png";
  await vi.waitFor(() => expect(read).toHaveBeenCalledWith(path, 32 * 1024 * 1024));
  expect(host.querySelector(".inline-image-missing")).toBeTruthy();
  expect(host.querySelector("img")).toBeNull();
  expect(toasts()).toEqual([]);
  refreshAsset(path);
  await vi.waitFor(() => expect(host.querySelector("img")?.getAttribute("src")).toBe("blob:arrived-area"));
  expect(toasts()).toEqual([]);
});
