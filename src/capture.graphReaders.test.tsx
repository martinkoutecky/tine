import { afterAll, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { editorIn, maybeEditorIn } from "./tests/editorDriver";

vi.mock("@tauri-apps/api/event", () => ({
  listen: async () => () => {},
  emitTo: async () => {},
}));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    label: "capture", isVisible: async () => false,
    onFocusChanged: async () => () => {}, hide: async () => {},
    setSize: async () => {},
  }),
}));

afterAll(() => vi.restoreAllMocks());

it("cold capture edits and completes pages without general graph readers or error toasts", async () => {
  // A capture binding grants bounded completion, not a normal graph slot.
  // Reject exactly as native GraphContext does, even after binding succeeds.
  vi.spyOn(backend(), "bindCaptureGraph").mockResolvedValue();
  vi.spyOn(backend(), "graphBindingGeneration").mockReturnValue(17);
  vi.spyOn(backend(), "getCaptureEnterFiles").mockResolvedValue(false);
  const counts = vi.spyOn(backend(), "getBlockRefCounts").mockRejectedValue("no graph loaded for window capture");
  const inventory = vi.spyOn(backend(), "pageInventory").mockRejectedValue("no graph loaded for window capture");
  const warm = vi.spyOn(backend(), "warmDone").mockResolvedValue(true);
  const completion = vi.spyOn(backend(), "captureQuickSwitch").mockResolvedValue([
    { name: "Capture destination", kind: "page", path: "pages/Capture destination.md", date_key: null },
  ]);
  const generalCompletion = vi.spyOn(backend(), "quickSwitch");
  const root = document.createElement("div");
  root.id = "capture-root";
  document.body.append(root);
  await import("./capture");
  await vi.waitFor(() => expect(maybeEditorIn(root)).not.toBeNull());

  const { toasts } = await import("./toasts");
  const errors = () => toasts().filter(toast => toast.kind === "error").map(toast => toast.message);
  // Prove startup itself is quiet, independently of later edit-triggered reads.
  expect.soft(errors()).toEqual([]);
  expect.soft(counts).not.toHaveBeenCalled();
  expect.soft(warm).not.toHaveBeenCalled();

  editorIn(root).input("[[Capture");
  await vi.waitFor(() => expect(completion).toHaveBeenCalled());
  await vi.waitFor(() => expect(document.body.textContent).toContain("Capture destination"));
  expect(generalCompletion).not.toHaveBeenCalled();

  const { pageByName, setRaw, insertOutlineAfter, deleteBlock, node } = await import("./document");
  const { CAPTURE_SCRATCH_NAME } = await import("./captureSeed");
  const { endEdit } = await import("./editorController");
  const first = pageByName(CAPTURE_SCRATCH_NAME)!.roots[0];
  setRaw(first, "[[Capture destination]]");
  endEdit("blur"); // Render a page link: missing-page decoration uses pageIndex.
  expect(root.textContent).toContain("Capture destination");
  // Exercise the independent cold edit-notice path without a warm-cache load.
  warm.mockResolvedValue(false);
  const { bumpGraphEpoch } = await import("./graphSession");
  bumpGraphEpoch();
  const scratch = insertOutlineAfter(first, [{ raw: "temporary thought\nid:: 12345678-1234-4234-8234-123456789abc", children: [] }]);
  expect(scratch).not.toBeNull();
  deleteBlock(scratch!); // Scratch deletion must not ask for graph-wide counts.
  expect(node(scratch!)).toBeUndefined();
  expect(node(first)?.raw).toBe("[[Capture destination]]");

  // Drain the queued inventory refresh and edit-notice continuations.
  await new Promise(resolve => setTimeout(resolve, 0));
  expect.soft(errors()).toEqual([]);
  expect.soft(counts).not.toHaveBeenCalled();
  expect.soft(inventory).not.toHaveBeenCalled();
  expect.soft(warm).not.toHaveBeenCalled();
});
