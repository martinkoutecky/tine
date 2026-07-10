import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { For, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { doc, loadSingle, pageByName, resetStore } from "../store";
import { startEditing } from "../editorController";
import type { BlockDto, PageDto } from "../types";
import { Block } from "./Block";

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  resetStore();
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

function page(name: string, blocks: BlockDto[]): PageDto {
  return { name, kind: "page", title: name, pre_block: null, blocks };
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
      types: ["Files", "image/png"],
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
  it("rolls back the inserted asset link if saveAsset rejects", async () => {
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
      expect(doc.byId[id].raw).toBe("[report.pdf](../assets/report.pdf)");
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
      await vi.waitFor(() => expect(doc.byId[id].raw).toBe("[notes.pdf](../assets/notes.pdf)"));
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
