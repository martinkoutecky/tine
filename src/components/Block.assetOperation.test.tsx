// I-2/I-21: acquire -> durable asset -> reference is one owned editor operation.
// Native Android blur ordering is driven at the actual command/editor boundary;
// native picker/IO promises are parked, not a substitute text-insertion helper.
import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import { editingId, startEditing } from "../editorController";
import { dispatchFocusedEditorCommand } from "../editorCommandBridge";
import { setGraphMeta } from "../graphSession";
import { setToasts, toasts } from "../toasts";
import { Block, SurfaceContext } from "./Block";

beforeAll(() => initParser());
afterEach(() => {
  vi.unstubAllGlobals(); vi.restoreAllMocks(); resetStore(); setGraphMeta(null);
  setToasts([]); document.body.innerHTML = "";
});
const settle = async () => { for (let i = 0; i < 6; i++) await new Promise(r => setTimeout(r, 0)); };
const surfaces = ["main", "pane:split", "sidebar:item"];

function setup(surface: string, method: "photo" | "upload", launch = true) {
  setGraphMeta({ root: "/graphs/A" } as never);
  loadSingle({ name: "Assets", title: "Assets", kind: "page", pre_block: null,
    blocks: [{ id: "asset-host", raw: "before after", collapsed: false, children: [] }] });
  startEditing("asset-host", 7, null, surface);
  let pick!: (value: any) => void;
  let save!: (name: string) => void;
  const picked = new Promise<any>(r => { pick = r; });
  const stored = new Promise<string>(r => { save = r; });
  const picker = method === "photo" ? vi.spyOn(backend(), "capturePhoto") : vi.spyOn(backend(), "pickFile");
  picker.mockReturnValue(picked);
  const writer = method === "photo" ? vi.spyOn(backend(), "importNativeCapture") : vi.spyOn(backend(), "importAsset");
  writer.mockReturnValue(stored);
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <SurfaceContext.Provider value={surface}><Block id="asset-host" /></SurfaceContext.Provider>, root);
  const ta = root.querySelector("textarea")!;
  ta.focus(); ta.setSelectionRange(7, 7);
  if (launch) expect(dispatchFocusedEditorCommand(method === "photo" ? "editor/capture-photo" : "editor/upload-asset")).toBe(true);
  return { ta, dispose, writer, save, pick: () => pick(method === "photo"
    ? { status: "ok", path: "/cache/photo.jpg", ext: "jpg" } : "/tmp/photo.jpg"), cancel: () => pick(method === "photo" ? { status: "cancelled" } : null) };
}

it.each(surfaces.flatMap(surface => (["photo", "upload"] as const).map(method => [surface, method] as const)))
("%s %s retains the editor through import and inserts into the latest edited text", async (surface, method) => {
  const op = setup(surface, method);
  try {
    op.pick(); await settle(); expect(op.writer).toHaveBeenCalledOnce();
    // The user can keep editing while import is in flight; don't replay an old base.
    op.ta.value = "before new after"; op.ta.dispatchEvent(new Event("input", { bubbles: true }));
    op.ta.setSelectionRange(7, 10);
    vi.spyOn(document, "hasFocus").mockReturnValue(true);
    op.ta.blur(); // delayed native lifecycle event, after the picker already returned
    op.save("saved.jpg"); await settle();
    expect(doc.byId["asset-host"].raw).toBe("before ![](../assets/saved.jpg) after");
    expect(toasts().some(t => t.message.includes("not inserted"))).toBe(false);
    expect(document.activeElement).toBe(op.ta);
    // The acquisition is released: an ordinary subsequent blur exits the editor.
    op.ta.blur(); await settle(); expect(editingId()).toBe(null);
  } finally { op.dispose(); }
});

it.each(["photo", "upload"] as const)("%s cancellation releases blur ownership", async method => {
  const op = setup("main", method);
  try {
    op.cancel(); await settle(); vi.spyOn(document, "hasFocus").mockReturnValue(true);
    op.ta.blur(); await settle();
    expect(editingId()).toBe(null); expect(op.writer).not.toHaveBeenCalled();
    expect(doc.byId["asset-host"].raw).toBe("before after");
  } finally { op.dispose(); }
});

it.each(["graph", "surface"])("does not insert or reclaim focus after an explicit %s change", async change => {
  const op = setup("main", "photo");
  try {
    op.pick(); await settle();
    if (change === "graph") setGraphMeta({ root: "/graphs/B" } as never);
    else startEditing("asset-host", 0, null, "pane:other");
    op.save("saved.jpg"); await settle();
    expect(doc.byId["asset-host"].raw).toBe("before after");
    expect(toasts().some(t => t.message.includes("not inserted"))).toBe(true);
  } finally { op.dispose(); }
});

it("the literal Upload slash command keeps the editor during the picker and import", async () => {
  const op = setup("pane:split", "upload", false);
  try {
    op.ta.value = "/upload"; op.ta.setSelectionRange(7, 7);
    op.ta.dispatchEvent(new Event("input", { bubbles: true }));
    await vi.waitFor(() => expect(document.body.querySelector(".autocomplete .ac-label")?.textContent).toBe("Upload an asset"));
    op.ta.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
    await settle(); expect(backend().pickFile).toHaveBeenCalledOnce();
    vi.spyOn(document, "hasFocus").mockReturnValue(true);
    op.ta.blur(); // Upload must also own blur while the chooser is still up.
    op.pick(); await settle(); op.save("saved.jpg"); await settle();
    expect(doc.byId["asset-host"].raw).toContain("![](../assets/saved.jpg)");
    expect(doc.byId["asset-host"].raw).not.toContain("/upload");
    expect(toasts().some(t => t.message.includes("not inserted"))).toBe(false);
  } finally { op.dispose(); }
});

it.each(surfaces)("%s file paste uses the current selection after durable import", async surface => {
  const op = setup(surface, "upload", false);
  vi.spyOn(backend(), "clipboardFiles").mockResolvedValue({ files: [{ path: "/tmp/photo.jpg", name: "photo.jpg", size: 8 }], skipped: 0, truncated: false });
  try {
    const paste = new Event("paste", { bubbles: true, cancelable: true });
    Object.defineProperty(paste, "clipboardData", { value: { getData: () => "", types: ["Files"], items: [{ kind: "file", getAsFile: () => new File(["bytes"], "photo.jpg") }] } });
    op.ta.dispatchEvent(paste); await settle(); expect(op.writer).toHaveBeenCalledOnce();
    op.ta.value = "before new after"; op.ta.dispatchEvent(new Event("input", { bubbles: true }));
    op.ta.setSelectionRange(7, 10); op.save("saved.jpg"); await settle();
    expect(doc.byId["asset-host"].raw).toBe("before ![](../assets/saved.jpg) after");
    expect(op.ta.value).toBe(doc.byId["asset-host"].raw);
  } finally { op.dispose(); }
});

it("failed import releases native operation ownership and preserves text", async () => {
  const op = setup("main", "photo");
  op.writer.mockRejectedValue(new Error("disk-full"));
  try {
    op.pick(); await settle();
    expect(toasts().some(t => t.message.includes("disk-full"))).toBe(true);
    vi.spyOn(document, "hasFocus").mockReturnValue(true); op.ta.blur(); await settle();
    expect(editingId()).toBe(null); expect(doc.byId["asset-host"].raw).toBe("before after");
  } finally { op.dispose(); }
});

it("clipboard image bytes share the landing path and use text edited during save", async () => {
  const op = setup("sidebar:item", "upload", false);
  vi.spyOn(backend(), "clipboardFiles").mockResolvedValue({ files: [], skipped: 0, truncated: false });
  vi.stubGlobal("URL", { ...URL, createObjectURL: () => "blob:asset", revokeObjectURL: vi.fn() });
  vi.spyOn(backend(), "saveAsset").mockReturnValue(new Promise(r => { op.save = r; }));
  try {
    const file = new File(["bytes"], "image.png", { type: "image/png" });
    Object.defineProperty(file, "arrayBuffer", { value: async () => new Uint8Array([1, 2, 3]).buffer });
    const paste = new Event("paste", { bubbles: true, cancelable: true });
    Object.defineProperty(paste, "clipboardData", { value: { getData: () => "", types: ["Files"], items: [{ kind: "file", type: "image/png", getAsFile: () => file }] } });
    op.ta.dispatchEvent(paste); await settle(); expect(backend().saveAsset).toHaveBeenCalledOnce();
    expect(doc.byId["asset-host"].raw).toBe("before after"); // no reference before bytes
    op.ta.value = "before new after"; op.ta.dispatchEvent(new Event("input", { bubbles: true }));
    op.ta.setSelectionRange(7, 10); op.save("saved.jpg"); await settle();
    expect(doc.byId["asset-host"].raw).toBe("before ![](../assets/saved.jpg) after");
  } finally { op.dispose(); }
});

it("finishing one native operation does not release another pending operation", async () => {
  const op = setup("main", "photo");
  let cancelSecond!: (value: any) => void;
  try {
    op.pick(); await settle();
    vi.mocked(backend().capturePhoto).mockReturnValue(new Promise(r => { cancelSecond = r; }));
    expect(dispatchFocusedEditorCommand("editor/capture-photo")).toBe(true);
    op.save("saved.jpg"); await settle();
    vi.spyOn(document, "hasFocus").mockReturnValue(true); op.ta.blur(); await settle();
    expect(editingId()).toBe("asset-host");
    cancelSecond({ status: "cancelled" }); await settle();
    op.ta.focus(); op.ta.blur(); await settle(); expect(editingId()).toBe(null);
  } finally { op.dispose(); }
});
