import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { For, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { doc, loadSingle, pageByName, resetStore } from "../store";
import { startEditing } from "../editorController";
import { setToasts, toasts } from "../ui";
import type { BlockDto, PageDto } from "../types";
import { Block } from "./Block";

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

function page(name: string, blocks: BlockDto[], opts: Pick<PageDto, "path" | "format"> = {}): PageDto {
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
      path: "pages/projects/Nested assets.md",
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
      path: "pages/Org assets.org",
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
      expect(backend().importAsset).toHaveBeenCalledWith("C:\\Users\\me\\report.pdf", "report.pdf");
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
