import { afterEach, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { invalidateBinding } from "./binding";
import { acquireMediaBlobFallback } from "./mediaBlobFallback";

afterEach(() => vi.restoreAllMocks());

it("does not publish a media blob read from a retired graph", async () => {
  let complete!: (bytes: Uint8Array) => void;
  const read = vi.spyOn(backend(), "readAsset").mockImplementationOnce(() =>
    new Promise((resolve) => { complete = resolve; })
  );
  const create = vi.spyOn(URL, "createObjectURL").mockReturnValue("blob:old-graph");
  const pending = acquireMediaBlobFallback("clip.mp4", "video", "video/mp4");
  await vi.waitFor(() => expect(read).toHaveBeenCalledOnce());
  invalidateBinding();
  complete(new Uint8Array([1, 2, 3]));
  await expect(pending).rejects.toMatchObject({ name: "AbortError" });
  expect(create).not.toHaveBeenCalled();
});

it("does not start a queued media read after its graph retires", async () => {
  let complete!: (bytes: Uint8Array) => void;
  const read = vi.spyOn(backend(), "readAsset")
    .mockImplementationOnce(() => new Promise((resolve) => { complete = resolve; }))
    .mockResolvedValueOnce(new Uint8Array([4]));
  const create = vi.spyOn(URL, "createObjectURL").mockReturnValue("blob:queued");
  const first = acquireMediaBlobFallback("first.mp4", "video", "video/mp4");
  await vi.waitFor(() => expect(read).toHaveBeenCalledOnce());
  const queued = acquireMediaBlobFallback("second.mp4", "video", "video/mp4");
  invalidateBinding();
  complete(new Uint8Array([1]));
  try { (await first).release(); } catch { /* first read is retired */ }
  await expect(queued).rejects.toMatchObject({ name: "AbortError" });
  expect(read).toHaveBeenCalledOnce();
  expect(create).not.toHaveBeenCalled();
});
