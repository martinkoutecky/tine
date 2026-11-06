// GH #835: pasting an image into the Quick Capture window failed with
// "Couldn't import pasted file: no graph loaded for window capture". The capture
// window has no graph slot, so the graph-window asset commands refuse it by design;
// its pastes go to the narrow capture asset commands, and the captured block
// references the stored file exactly as a graph window's block does.
import { maybeEditorIn } from "../tests/editorDriver";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import type { JSX } from "solid-js";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import { startEditing } from "../editorController";
import { setToasts, toasts } from "../toasts";
import type { BlockDto, PageDto } from "../types";
import { Block, CaptureCtx, type CaptureApi } from "./Block";

beforeAll(async () => { await initParser(); });
afterEach(() => { vi.unstubAllGlobals(); vi.restoreAllMocks(); resetStore(); setToasts([]); document.body.innerHTML = ""; });

const NO_SLOT = "no graph loaded for window capture";
const blk = (id: string, raw: string): BlockDto => ({ id, raw, collapsed: false, children: [] });
const page = (name: string, blocks: BlockDto[]): PageDto => ({ name, kind: "page", title: name, pre_block: null, blocks });
const settle = async () => { for (let i = 0; i < 8; i++) await new Promise((resolve) => setTimeout(resolve, 0)); };
const capture: CaptureApi = { submit: vi.fn(), cancel: vi.fn(), enterFiles: () => true, quickSwitch: async () => [] };

function mount(node: () => JSX.Element) {
  const root = document.createElement("div");
  document.body.appendChild(root);
  return { root, dispose: render(node, root) };
}

function pasteEvent(files: File[]): Event {
  const event = new Event("paste", { bubbles: true, cancelable: true });
  Object.defineProperty(event, "clipboardData", { value: {
    getData: () => "",
    items: files.map((file) => ({ kind: "file", type: file.type, getAsFile: () => file })),
    types: ["Files"],
  } });
  return event;
}

function refuseGraphWindowAssetCommands() {
  const refuse = async () => { throw new Error(NO_SLOT); };
  vi.spyOn(backend(), "importAsset").mockImplementation(refuse);
  vi.spyOn(backend(), "saveAsset").mockImplementation(refuse);
}

describe("GH #835 Quick Capture pastes a file", () => {
  it("imports a copied file through the capture import and references it", async () => {
    loadSingle(page("·capture·", [blk("cap-file", "")]));
    startEditing("cap-file", 0);
    refuseGraphWindowAssetCommands();
    vi.spyOn(backend(), "graphBindingGeneration").mockReturnValue(7);
    vi.spyOn(backend(), "clipboardFiles").mockResolvedValue({ files: [{ path: "/tmp/shot.png", name: "shot.png", size: 3 }], skipped: 0, truncated: false });
    const imported = vi.spyOn(backend(), "captureImportAsset").mockResolvedValue("shot_1.png");
    const { root, dispose } = mount(() => <CaptureCtx.Provider value={capture}><Block id="cap-file" /></CaptureCtx.Provider>);
    try {
      maybeEditorIn(root)!.paste(pasteEvent([new File([new Uint8Array([1, 2, 3])], "shot.png", { type: "image/png" })]));
      await settle();
      expect(imported).toHaveBeenCalledOnce();
      expect(imported.mock.calls[0][0]).toBe("/tmp/shot.png");
      expect(imported.mock.calls[0][2]).toBeGreaterThan(0);
      expect(backend().importAsset).not.toHaveBeenCalled();
      expect(doc.byId["cap-file"].raw).toBe("![](../assets/shot_1.png)");
      expect(toasts().filter((toast) => toast.kind === "error")).toEqual([]);
    } finally { dispose(); }
  });

  it("saves a pasted screenshot's bytes through the capture save and references it", async () => {
    loadSingle(page("·capture·", [blk("cap-bytes", "")]));
    startEditing("cap-bytes", 0);
    refuseGraphWindowAssetCommands();
    vi.stubGlobal("URL", { ...URL, createObjectURL: vi.fn(() => "blob:asset"), revokeObjectURL: vi.fn() });
    vi.spyOn(backend(), "clipboardFiles").mockResolvedValue({ files: [], skipped: 1, truncated: false });
    const saved = vi.spyOn(backend(), "captureSaveAsset").mockResolvedValue("screenshot.png");
    const { root, dispose } = mount(() => <CaptureCtx.Provider value={capture}><Block id="cap-bytes" /></CaptureCtx.Provider>);
    try {
      maybeEditorIn(root)!.paste(pasteEvent([new File([new Uint8Array([1, 2, 3])], "image.png", { type: "image/png" })]));
      await settle();
      expect(saved).toHaveBeenCalledOnce();
      expect(backend().saveAsset).not.toHaveBeenCalled();
      expect(doc.byId["cap-bytes"].raw).toBe("![](../assets/screenshot.png)");
      expect(toasts().filter((toast) => toast.kind === "error")).toEqual([]);
    } finally { dispose(); }
  });

  it("a graph window's paste still uses the graph-window commands", async () => {
    loadSingle(page("Plain", [blk("plain-file", "")]));
    startEditing("plain-file", 0);
    vi.spyOn(backend(), "graphBindingGeneration").mockReturnValue(7);
    vi.spyOn(backend(), "clipboardFiles").mockResolvedValue({ files: [{ path: "/tmp/doc.pdf", name: "doc.pdf", size: 3 }], skipped: 0, truncated: false });
    vi.spyOn(backend(), "importAsset").mockResolvedValue("doc.pdf");
    const narrow = vi.spyOn(backend(), "captureImportAsset").mockResolvedValue("never");
    const { root, dispose } = mount(() => <Block id="plain-file" />);
    try {
      maybeEditorIn(root)!.paste(pasteEvent([new File(["x"], "doc.pdf", { type: "application/pdf" })]));
      await settle();
      expect(backend().importAsset).toHaveBeenCalledOnce();
      expect(narrow).not.toHaveBeenCalled();
    } finally { dispose(); }
  });
});
