// UI-CAPTURE-NO-GRAPH-READ-TOASTS: the Quick Capture WebView is granted no
// graph read (src-tauri state.rs: capture has no graph slot). The CodeMirror
// editor renders a completed `[[page]]`, `#tag` or `((block))` inside the
// editor through the read renderer, which asked the graph for page icons and
// block resolutions, and a pasted image for its bytes; each refusal showed an
// error toast in the capture window.
import { afterAll, expect, it, vi } from "vitest";
import { backend } from "./backend";
import type { GraphMeta } from "./types";
import { editorIn, maybeEditorIn } from "./tests/editorDriver";

vi.unmock("./editor/cm6Setting");
vi.mock("@tauri-apps/api/event", () => ({ listen: async () => () => {}, emitTo: async () => {}, emit: async () => {} }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({
  label: "capture", isVisible: async () => false, onFocusChanged: async () => () => {},
  hide: async () => {}, setSize: async () => {},
}) }));

// Capture owns its root for the life of this WebView; drain its layout timers.
afterAll(() => new Promise<void>(resolve => setTimeout(resolve, 300)));

const REFUSED = "no graph loaded for window capture";
const UUID = "12345678-1234-4234-8234-123456789abc";

it("UI-CAPTURE-NO-GRAPH-READ-TOASTS: references rendered in the capture editor ask no graph read and raise no toast", async () => {
  if (!Range.prototype.getClientRects) Range.prototype.getClientRects = () => [] as unknown as DOMRectList;
  if (!Range.prototype.getBoundingClientRect) Range.prototype.getBoundingClientRect = () => new DOMRect();
  // The production default: the CodeMirror editor.
  vi.spyOn(backend(), "getAppBool").mockImplementation(async (_key, fallback) => fallback);
  vi.spyOn(backend(), "bindCaptureGraph").mockResolvedValue({ root: "/capture-graph", preferred_workflow: "now" } as GraphMeta);
  vi.spyOn(backend(), "graphBindingGeneration").mockReturnValue(17);
  vi.spyOn(backend(), "getCaptureEnterFiles").mockResolvedValue(false);
  vi.spyOn(backend(), "captureTarget").mockResolvedValue("main");
  // Reject exactly as the native graph slot does for the capture WebView.
  const icons = vi.spyOn(backend(), "pageIcons").mockRejectedValue(REFUSED);
  const blocks = vi.spyOn(backend(), "resolveBlocks").mockRejectedValue(REFUSED);
  const asset = vi.spyOn(backend(), "readAsset").mockRejectedValue(REFUSED);
  const root = document.createElement("div"); root.id = "capture-root"; document.body.append(root);
  await (await import("./components/EditorGate")).preloadCm6Editor();
  await import("./capture");
  await vi.waitFor(() => expect(maybeEditorIn(root)?.kind).toBe("cm6"));
  const { toasts } = await import("./toasts");
  const errors = () => toasts().filter(toast => toast.kind === "error").map(toast => toast.message);

  // What completing a page link, a tag and a block reference, and pasting an
  // image, leave; the caret after the last space keeps every one rendered.
  editorIn(root).input(`[[Fuzzy Existing]] #tag ((${UUID})) ![shot](../assets/shot.png) `);
  await vi.waitFor(() => expect(root.querySelectorAll("[data-cm6-construct] a.page-ref, [data-cm6-construct] a.tag")).toHaveLength(2));
  for (let i = 0; i < 5; i++) await new Promise(resolve => setTimeout(resolve, 0));

  expect.soft(root.querySelector(".page-ref-icon")).toBeNull();
  expect.soft(icons).not.toHaveBeenCalled();
  expect.soft(blocks).not.toHaveBeenCalled();
  expect.soft(asset).not.toHaveBeenCalled();
  expect.soft(errors()).toEqual([]);
});
