import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { For, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { pageByName, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import { startEditing } from "../editorController";
import { dispatchFocusedEditorCommand } from "../editorCommandBridge";
import { setToasts, toasts } from "../toasts";
import type { BlockDto, Format, PageDto } from "../types";
import { Block } from "./Block";
import * as mediaEditorSettings from "../mediaEditorSettings";

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  resetStore();
  setToasts([]);
  document.body.innerHTML = "";
});

function mount(node: () => JSX.Element): { root: HTMLDivElement; dispose: () => void } {
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(node, root);
  return { root, dispose };
}

function blk(id: string, raw: string): BlockDto {
  return { id, raw, collapsed: false, children: [] };
}

function page(name: string, blocks: BlockDto[], opts: { id?: string; format?: Format } = {}): PageDto & { id?: string } {
  return { name, kind: "page", title: name, pre_block: null, blocks, ...opts };
}

function tick(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

function imagePasteEvent(file: File): Event {
  const event = new Event("paste", { bubbles: true, cancelable: true });
  Object.defineProperty(event, "clipboardData", {
    value: {
      getData: () => "",
      items: [{ kind: "file", type: "image/png", getAsFile: () => file }],
      // Chromium/WebView2 commonly exposes MIME on DataTransferItem.type while
      // the top-level types list contains only the generic Files sentinel.
      types: ["Files"],
    },
  });
  return event;
}

function filePasteEvent(files: File[], text = ""): Event {
  const event = new Event("paste", { bubbles: true, cancelable: true });
  Object.defineProperty(event, "clipboardData", {
    value: {
      getData: (type: string) => type === "text/plain" ? text : "",
      items: files.map((file) => ({ kind: "file", type: file.type, getAsFile: () => file })),
      types: ["Files", "text/plain"],
    },
  });
  return event;
}

async function settle() {
  for (let i = 0; i < 6; i++) await tick();
}

describe("asset paste durability", () => {
  it("binds a picked upload to the graph selected before the picker", async () => {
    loadSingle(page("Assets", [blk("picked-switch", "")]));
    startEditing("picked-switch", 0);
    let generation = 1;
    let finishPick!: (path: string) => void;
    vi.spyOn(backend(), "graphBindingGeneration").mockImplementation(() => generation);
    vi.spyOn(backend(), "pickFile").mockImplementation(() => new Promise((resolve) => { finishPick = resolve; }));
    const writes: number[] = [];
    vi.spyOn(backend(), "importAsset").mockImplementation(async (_path, _name, requested) => {
      const target = requested ?? generation; // old TauriBackend leased the current graph
      writes.push(target);
      if (target !== generation) throw new Error("stale-graph-binding");
      return "picked.png";
    });
    const { root, dispose } = mount(() => <Block id="picked-switch" />);
    try {
      root.querySelector("textarea")!.focus();
      expect(dispatchFocusedEditorCommand("editor/upload-asset")).toBe(true);
      generation = 2;
      finishPick("/tmp/picked.png");
      await settle();
      expect(writes).toEqual([1]);
      expect(doc.byId["picked-switch"].raw).toBe("");
    } finally { dispose(); }
  });

  it("binds native capture before the camera returns", async () => {
    loadSingle(page("Assets", [blk("capture-switch", "")]));
    startEditing("capture-switch", 0);
    let generation = 1;
    let finishCapture!: (value: { status: "ok"; path: string; ext: string }) => void;
    vi.spyOn(backend(), "graphBindingGeneration").mockImplementation(() => generation);
    vi.spyOn(backend(), "capturePhoto").mockImplementation(() => new Promise((resolve) => { finishCapture = resolve; }));
    const writes: number[] = [];
    vi.spyOn(backend(), "importNativeCapture").mockImplementation(async (_path, _name, requested) => {
      const target = requested ?? generation;
      writes.push(target);
      if (target !== generation) throw new Error("stale-graph-binding");
      return "capture.jpg";
    });
    const { root, dispose } = mount(() => <Block id="capture-switch" />);
    try {
      root.querySelector("textarea")!.focus();
      expect(dispatchFocusedEditorCommand("editor/capture-photo")).toBe(true);
      generation = 2;
      finishCapture({ status: "ok", path: "/cache/tine_photo_1.jpg", ext: "jpg" });
      await settle();
      expect(writes).toEqual([1]);
      expect(doc.byId["capture-switch"].raw).toBe("");
    } finally { dispose(); }
  });

  it("binds clipboard files before the native clipboard read", async () => {
    loadSingle(page("Assets", [blk("clipboard-switch", "")]));
    startEditing("clipboard-switch", 0);
    let generation = 1;
    let finishRead!: (value: { files: { path: string; name: string; size: number }[]; skipped: number; truncated: boolean }) => void;
    vi.spyOn(backend(), "graphBindingGeneration").mockImplementation(() => generation);
    vi.spyOn(backend(), "clipboardFiles").mockImplementation(() => new Promise((resolve) => { finishRead = resolve; }));
    const writes: number[] = [];
    vi.spyOn(backend(), "importAsset").mockImplementation(async (_path, _name, requested) => {
      const target = requested ?? generation;
      writes.push(target);
      if (target !== generation) throw new Error("stale-graph-binding");
      return "clip.pdf";
    });
    const { root, dispose } = mount(() => <Block id="clipboard-switch" />);
    try {
      root.querySelector("textarea")!.dispatchEvent(filePasteEvent([
        new File(["x"], "clip.pdf", { type: "application/pdf" }),
      ]));
      generation = 2;
      finishRead({ files: [{ path: "/tmp/clip.pdf", name: "clip.pdf", size: 1 }], skipped: 0, truncated: false });
      await settle();
      expect(writes).toEqual([1]);
      expect(doc.byId["clipboard-switch"].raw).toBe("");
    } finally { dispose(); }
  });

  it("binds clipboard bytes before reading the browser file", async () => {
    loadSingle(page("Assets", [blk("bytes-switch", "")]));
    startEditing("bytes-switch", 0);
    let generation = 1;
    let finishBytes!: (bytes: ArrayBuffer) => void;
    const arrayBuffer = vi.fn(() => new Promise<ArrayBuffer>((resolve) => { finishBytes = resolve; }));
    const file = { name: "bytes.pdf", type: "application/pdf", size: 1, arrayBuffer } as unknown as File;
    vi.spyOn(backend(), "graphBindingGeneration").mockImplementation(() => generation);
    vi.spyOn(backend(), "clipboardFiles").mockResolvedValue({ files: [], skipped: 0, truncated: false });
    const writes: number[] = [];
    vi.spyOn(backend(), "saveAsset").mockImplementation(async (_name, _bytes, requested) => {
      const target = requested ?? generation;
      writes.push(target);
      if (target !== generation) throw new Error("stale-graph-binding");
      return "bytes.pdf";
    });
    const { root, dispose } = mount(() => <Block id="bytes-switch" />);
    try {
      root.querySelector("textarea")!.dispatchEvent(filePasteEvent([file]));
      await vi.waitFor(() => expect(arrayBuffer).toHaveBeenCalledOnce());
      generation = 2;
      finishBytes(new Uint8Array([1]).buffer);
      await settle();
      expect(writes).toEqual([1]);
      expect(doc.byId["bytes-switch"].raw).toBe("");
    } finally { dispose(); }
  });

  it("keeps the draw.io handoff on the graph that received its diagram", async () => {
    loadSingle(page("Assets", [blk("drawio-switch", "/drawio")]));
    startEditing("drawio-switch", 7);
    let generation = 1;
    let finishCommand!: (command: string) => void;
    vi.spyOn(backend(), "graphBindingGeneration").mockImplementation(() => generation);
    vi.spyOn(backend(), "saveAsset").mockResolvedValue("diagram.drawio.svg");
    vi.spyOn(mediaEditorSettings, "resolveMediaEditorCommand").mockImplementation(() => new Promise((resolve) => { finishCommand = resolve; }));
    const opened: number[] = [];
    vi.spyOn(backend(), "editAssetExternal").mockImplementation(async (_name, _cmd, requested) => {
      const target = requested ?? generation;
      opened.push(target);
      if (target !== generation) throw new Error("stale-graph-binding");
    });
    const { root, dispose } = mount(() => <Block id="drawio-switch" />);
    try {
      const textarea = root.querySelector("textarea")! as HTMLTextAreaElement;
      textarea.focus();
      textarea.value = "/drawio";
      textarea.setSelectionRange(7, 7);
      textarea.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: "o" }));
      await vi.waitFor(() => expect([...document.querySelectorAll(".autocomplete .ac-label")].map((x) => x.textContent)).toContain("Draw.io diagram"));
      const item = [...document.querySelectorAll<HTMLElement>(".autocomplete .ac-item")]
        .find((x) => x.querySelector(".ac-label")?.textContent === "Draw.io diagram")!;
      item.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true }));
      await vi.waitFor(() => expect(finishCommand).toBeTypeOf("function"));
      generation = 2;
      finishCommand("drawio {}");
      await settle();
      expect(opened).toEqual([]);
    } finally { dispose(); }
  });
  it("does not insert an asset link if saveAsset rejects", async () => {
    loadSingle(page("Assets", [blk("asset-1", "")]));
    const id = pageByName("Assets")!.roots[0];
    startEditing(id, 0);
    vi.stubGlobal("URL", {
      ...URL,
      createObjectURL: vi.fn(() => "blob:asset"),
      revokeObjectURL: vi.fn(),
    });
    vi.spyOn(backend(), "saveAsset").mockRejectedValue(new Error("disk full"));

    const { root, dispose } = mount(() => (
      <For each={pageByName("Assets")?.roots ?? []}>{(bid) => <Block id={bid} />}</For>
    ));

    try {
      const textarea = root.querySelector("textarea") as HTMLTextAreaElement | null;
      expect(textarea).not.toBeNull();
      textarea!.focus();
      textarea!.setSelectionRange(0, 0);
      textarea!.dispatchEvent(imagePasteEvent(new File([new Uint8Array([1, 2, 3])], "paste.png", { type: "image/png" })));

      await settle();

      expect(backend().saveAsset).toHaveBeenCalledOnce();
      expect(doc.byId[id].raw).not.toContain("../assets/");
    } finally {
      dispose();
    }
  });

  it("inserts the Markdown reference only after asset bytes are durable", async () => {
    loadSingle(page("Assets", [blk("asset-delayed", "")]));
    const id = pageByName("Assets")!.roots[0];
    startEditing(id, 0);
    vi.stubGlobal("URL", {
      ...URL,
      createObjectURL: vi.fn(() => "blob:asset"),
      revokeObjectURL: vi.fn(),
    });
    let finish!: (name: string) => void;
    vi.spyOn(backend(), "saveAsset").mockImplementation(
      () => new Promise<string>((resolve) => { finish = resolve; })
    );

    const { root, dispose } = mount(() => (
      <For each={pageByName("Assets")?.roots ?? []}>{(bid) => <Block id={bid} />}</For>
    ));
    try {
      const textarea = root.querySelector("textarea")! as HTMLTextAreaElement;
      textarea.dispatchEvent(imagePasteEvent(new File([new Uint8Array([1])], "paste.png", { type: "image/png" })));
      await settle();
      expect(backend().saveAsset).toHaveBeenCalledOnce();
      expect(doc.byId[id].raw).toBe("");

      finish("durable.png");
      await settle();
      expect(doc.byId[id].raw).toBe("![](../assets/durable.png)");
    } finally {
      dispose();
    }
  });

  it("keeps a saved asset out of an editor that lost ownership while saving", async () => {
    loadSingle(page("Assets", [blk("asset-stale-a", ""), blk("asset-stale-b", "")]));
    const [first, second] = pageByName("Assets")!.roots;
    startEditing(first, 0);
    vi.stubGlobal("URL", {
      ...URL,
      createObjectURL: vi.fn(() => "blob:asset"),
      revokeObjectURL: vi.fn(),
    });
    let finish!: (name: string) => void;
    vi.spyOn(backend(), "saveAsset").mockImplementation(
      () => new Promise<string>((resolve) => { finish = resolve; })
    );
    const { root, dispose } = mount(() => (
      <For each={pageByName("Assets")?.roots ?? []}>{(bid) => <Block id={bid} />}</For>
    ));
    try {
      root.querySelector("textarea")!.dispatchEvent(imagePasteEvent(
        new File([new Uint8Array([1])], "paste.png", { type: "image/png" })
      ));
      await settle();
      expect(backend().saveAsset).toHaveBeenCalledOnce();
      startEditing(second, 0);
      await settle();
      finish("durable.png");
      await settle();
      expect(doc.byId[first].raw).toBe("");
      expect(doc.byId[second].raw).toBe("");
      expect(toasts().some((toast) => toast.message ===
        "The asset was saved, but it was not inserted because the graph or block changed."
      )).toBe(true);
    } finally {
      dispose();
    }
  });

  it("keeps a picked upload stored when its original editor loses ownership", async () => {
    loadSingle(page("Assets", [blk("upload-stale-a", ""), blk("upload-stale-b", "")]));
    const [first, second] = pageByName("Assets")!.roots;
    startEditing(first, 0);
    vi.spyOn(backend(), "pickFile").mockResolvedValue("/tmp/upload.png");
    let finish!: (name: string) => void;
    vi.spyOn(backend(), "importAsset").mockImplementation(
      () => new Promise<string>((resolve) => { finish = resolve; })
    );
    const { root, dispose } = mount(() => (
      <For each={pageByName("Assets")?.roots ?? []}>{(bid) => <Block id={bid} />}</For>
    ));
    try {
      const textarea = root.querySelector("textarea")! as HTMLTextAreaElement;
      textarea.focus();
      expect(dispatchFocusedEditorCommand("editor/upload-asset")).toBe(true);
      await settle();
      expect(backend().importAsset).toHaveBeenCalledOnce();
      startEditing(second, 0);
      await settle();
      finish("stored-upload.png");
      await settle();
      expect(doc.byId[first].raw).toBe("");
      expect(doc.byId[second].raw).toBe("");
      expect(toasts().some((toast) => toast.message ===
        "The asset was saved, but it was not inserted because the graph or block changed."
      )).toBe(true);
    } finally {
      dispose();
    }
  });

  it("routes a Windows-style screenshot directly through image bytes", async () => {
    loadSingle(page("Assets", [blk("asset-win-shot", "")]));
    const id = pageByName("Assets")!.roots[0];
    startEditing(id, 0);
    vi.stubGlobal("URL", {
      ...URL,
      createObjectURL: vi.fn(() => "blob:asset"),
      revokeObjectURL: vi.fn(),
    });
    const native = vi.spyOn(backend(), "clipboardFiles").mockResolvedValue({
      files: [],
      skipped: 1,
      truncated: false,
    });
    vi.spyOn(backend(), "saveAsset").mockResolvedValue("screenshot.png");

    const { root, dispose } = mount(() => (
      <For each={pageByName("Assets")?.roots ?? []}>{(bid) => <Block id={bid} />}</For>
    ));
    try {
      const textarea = root.querySelector("textarea")! as HTMLTextAreaElement;
      textarea.dispatchEvent(imagePasteEvent(new File([new Uint8Array([1, 2, 3])], "image.png", { type: "image/png" })));
      await settle();

      // The native reader sees a pseudo path, then the event image bytes win and
      // suppress that bogus skipped count instead of showing GH #78's error.
      expect(native).toHaveBeenCalledOnce();
      expect(backend().saveAsset).toHaveBeenCalledOnce();
      expect(doc.byId[id].raw).toBe("![](../assets/screenshot.png)");
      expect(toasts().some((toast) => toast.message.startsWith("Skipped "))).toBe(false);
    } finally {
      dispose();
    }
  });

  it("does not materialize an oversized screenshot image", async () => {
    loadSingle(page("Assets", [blk("asset-huge-shot", "")]));
    const id = pageByName("Assets")!.roots[0];
    startEditing(id, 0);
    vi.spyOn(backend(), "clipboardFiles").mockResolvedValue({ files: [], skipped: 0, truncated: false });
    const save = vi.spyOn(backend(), "saveAsset");
    const arrayBuffer = vi.fn();
    const huge = {
      name: "image.png",
      type: "image/png",
      size: 64 * 1024 * 1024 + 1,
      arrayBuffer,
    } as unknown as File;

    const { root, dispose } = mount(() => (
      <For each={pageByName("Assets")?.roots ?? []}>{(bid) => <Block id={bid} />}</For>
    ));
    try {
      const textarea = root.querySelector("textarea")! as HTMLTextAreaElement;
      textarea.dispatchEvent(imagePasteEvent(huge));
      await settle();
      expect(arrayBuffer).not.toHaveBeenCalled();
      expect(save).not.toHaveBeenCalled();
      expect(doc.byId[id].raw).toBe("");
    } finally {
      dispose();
    }
  });

  it("keeps mixed copied files on the generic native-file path", async () => {
    loadSingle(page("Assets", [blk("asset-mixed", "")]));
    const id = pageByName("Assets")!.roots[0];
    startEditing(id, 0);
    const native = vi.spyOn(backend(), "clipboardFiles").mockResolvedValue({
      files: [
        { path: "C:\\photo.png", name: "photo.png", size: 3 },
        { path: "C:\\report.pdf", name: "report.pdf", size: 3 },
      ],
      skipped: 0,
      truncated: false,
    });
    vi.spyOn(backend(), "importAsset")
      .mockResolvedValueOnce("photo.png")
      .mockResolvedValueOnce("report.pdf");

    const { root, dispose } = mount(() => (
      <For each={pageByName("Assets")?.roots ?? []}>{(bid) => <Block id={bid} />}</For>
    ));
    try {
      const textarea = root.querySelector("textarea")! as HTMLTextAreaElement;
      textarea.dispatchEvent(filePasteEvent([
        new File([new Uint8Array([1])], "photo.png", { type: "image/png" }),
        new File([new Uint8Array([2])], "report.pdf", { type: "application/pdf" }),
      ]));
      await settle();
      expect(native).toHaveBeenCalledOnce();
      expect(doc.byId[id].raw).toBe("![](../assets/photo.png)\n![report.pdf](../assets/report.pdf)");
    } finally {
      dispose();
    }
  });

  it("keeps the source PDF name as its label when the asset template renames it", async () => {
    loadSingle(page("Nested assets", [blk("asset-renamed-pdf", "")], {
        id: "pages/projects/Nested assets.md",
      format: "md",
    }));
    const id = pageByName("Nested assets")!.roots[0];
    startEditing(id, 0);
    vi.spyOn(backend(), "clipboardFiles").mockResolvedValue({
      files: [{ path: "/tmp/Research paper.pdf", name: "Research paper.pdf", size: 3 }],
      skipped: 0,
      truncated: false,
    });
    vi.spyOn(backend(), "importAsset").mockResolvedValue("20300102-paper.pdf");

    const { root, dispose } = mount(() => (
      <For each={pageByName("Nested assets")?.roots ?? []}>{(bid) => <Block id={bid} />}</For>
    ));
    try {
      const textarea = root.querySelector("textarea")! as HTMLTextAreaElement;
      textarea.dispatchEvent(filePasteEvent([
        new File([new Uint8Array([1])], "Research paper.pdf", { type: "application/pdf" }),
      ]));
      await settle();

      expect(doc.byId[id].raw).toBe(
        "![Research paper.pdf](../../assets/20300102-paper.pdf)"
      );
    } finally {
      dispose();
    }
  });

  it("inserts an Org PDF link on an Org page", async () => {
    loadSingle(page("Org assets", [blk("asset-org-pdf", "")], {
        id: "pages/Org assets.org",
      format: "org",
    }));
    const id = pageByName("Org assets")!.roots[0];
    startEditing(id, 0);
    vi.spyOn(backend(), "clipboardFiles").mockResolvedValue({
      files: [{ path: "/tmp/Paper.pdf", name: "Paper.pdf", size: 3 }],
      skipped: 0,
      truncated: false,
    });
    vi.spyOn(backend(), "importAsset").mockResolvedValue("stored-paper.pdf");

    const { root, dispose } = mount(() => (
      <For each={pageByName("Org assets")?.roots ?? []}>{(bid) => <Block id={bid} />}</For>
    ));
    try {
      const textarea = root.querySelector("textarea")! as HTMLTextAreaElement;
      textarea.dispatchEvent(filePasteEvent([
        new File([new Uint8Array([1])], "Paper.pdf", { type: "application/pdf" }),
      ]));
      await settle();

      expect(doc.byId[id].raw).toBe("[[../assets/stored-paper.pdf][Paper.pdf]]");
    } finally {
      dispose();
    }
  });

  it("prefers native file paths over accompanying clipboard path text", async () => {
    loadSingle(page("Assets", [blk("asset-native", "")]));
    const id = pageByName("Assets")!.roots[0];
    startEditing(id, 0);
    vi.spyOn(backend(), "clipboardFiles").mockResolvedValue({
      files: [{ path: "C:\\Users\\me\\report.pdf", name: "report.pdf", size: 123 }],
      skipped: 0,
      truncated: false,
    });
    vi.spyOn(backend(), "importAsset").mockResolvedValue("report.pdf");

    const { root, dispose } = mount(() => (
      <For each={pageByName("Assets")?.roots ?? []}>{(bid) => <Block id={bid} />}</For>
    ));
    try {
      const textarea = root.querySelector("textarea")! as HTMLTextAreaElement;
      const event = filePasteEvent([], "C:\\Users\\me\\report.pdf");
      textarea.dispatchEvent(event);
      await settle();

      expect(event.defaultPrevented).toBe(true);
      expect(backend().importAsset).toHaveBeenCalledWith("C:\\Users\\me\\report.pdf", "report.pdf", 1);
      expect(doc.byId[id].raw).toBe("![report.pdf](../assets/report.pdf)");
      expect(doc.byId[id].raw).not.toContain("C:\\Users");
    } finally {
      dispose();
    }
  });

  it("saves a byte-only clipboard file and inserts its asset link", async () => {
    loadSingle(page("Assets", [blk("asset-bytes", "")]));
    const id = pageByName("Assets")!.roots[0];
    startEditing(id, 0);
    vi.spyOn(backend(), "clipboardFiles").mockResolvedValue({ files: [], skipped: 0, truncated: false });
    vi.spyOn(backend(), "saveAsset").mockResolvedValue("notes.pdf");

    const { root, dispose } = mount(() => (
      <For each={pageByName("Assets")?.roots ?? []}>{(bid) => <Block id={bid} />}</For>
    ));
    try {
      const textarea = root.querySelector("textarea")! as HTMLTextAreaElement;
      textarea.dispatchEvent(filePasteEvent([new File([new Uint8Array([1, 2, 3])], "notes.pdf", { type: "application/pdf" })]));
      await settle();

      expect(backend().saveAsset).toHaveBeenCalledOnce();
      await vi.waitFor(() => expect(doc.byId[id].raw).toBe("![notes.pdf](../assets/notes.pdf)"));
    } finally {
      dispose();
    }
  });

  it("does not materialize an oversized byte-only clipboard file", async () => {
    loadSingle(page("Assets", [blk("asset-huge", "")]));
    const id = pageByName("Assets")!.roots[0];
    startEditing(id, 0);
    vi.spyOn(backend(), "clipboardFiles").mockResolvedValue({ files: [], skipped: 0, truncated: false });
    const save = vi.spyOn(backend(), "saveAsset");
    const arrayBuffer = vi.fn();
    const huge = { name: "huge.zip", type: "application/zip", size: 64 * 1024 * 1024 + 1, arrayBuffer } as unknown as File;

    const { root, dispose } = mount(() => (
      <For each={pageByName("Assets")?.roots ?? []}>{(bid) => <Block id={bid} />}</For>
    ));
    try {
      const textarea = root.querySelector("textarea")! as HTMLTextAreaElement;
      textarea.dispatchEvent(filePasteEvent([huge]));
      await settle();

      expect(arrayBuffer).not.toHaveBeenCalled();
      expect(save).not.toHaveBeenCalled();
      expect(doc.byId[id].raw).toBe("");
    } finally {
      dispose();
    }
  });
});
