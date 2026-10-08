import { afterEach, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { captureBinding } from "./binding";
import { importCaptureToOrigin, type AssetEditorToken } from "./assetLanding";
import { setGraphMeta } from "./graphSession";
import { setToasts, toasts } from "./toasts";

afterEach(() => {
  vi.restoreAllMocks();
  setGraphMeta(null);
  setToasts([]);
});

const token = (graphRoot: string | undefined): AssetEditorToken => ({
  binding: captureBinding(), graphRoot, textarea: {} as HTMLTextAreaElement,
  editingBlockId: null, editingBlockOwner: null, editingBlockSurface: null,
});

// og H1b: Rust rebinds the window before the frontend learns the new
// generation. A capture refused as stale in that gap is retried with the new
// binding, still naming the graph it was started in, and so is not lost.
it("retries a capture refused mid-switch with the new binding and the original graph", async () => {
  setGraphMeta({ root: "/graphs/A" } as never);
  const started = token("/graphs/A");
  let generation = 1;
  vi.spyOn(backend(), "graphBindingGeneration").mockImplementation(() => generation);
  const calls: unknown[][] = [];
  vi.spyOn(backend(), "importNativeCapture").mockImplementation(async (...args) => {
    calls.push(args);
    if (args[2] !== 2) { generation = 2; setGraphMeta({ root: "/graphs/B" } as never); throw new Error("stale-graph-binding"); }
    return "memo.m4a";
  });
  await expect(importCaptureToOrigin(started, "/cache/tine_memo_3.m4a", "memo.m4a")).resolves.toBeNull();
  expect(calls).toEqual([
    ["/cache/tine_memo_3.m4a", "memo.m4a", 1, "/graphs/A"],
    ["/cache/tine_memo_3.m4a", "memo.m4a", 2, "/graphs/A"],
  ]);
  expect(toasts().map((toast) => toast.message)).toEqual([
    "Saved memo.m4a to the assets/ of graph “A”; it was not inserted because the graph changed.",
  ]);
});

it("returns the stored name for insertion while the capture's graph is still open", async () => {
  setGraphMeta({ root: "/graphs/A" } as never);
  vi.spyOn(backend(), "importNativeCapture").mockResolvedValue("photo.jpg");
  await expect(importCaptureToOrigin(token("/graphs/A"), "/cache/tine_photo_1.jpg", "photo.jpg")).resolves.toBe("photo.jpg");
  expect(toasts()).toEqual([]);
});
