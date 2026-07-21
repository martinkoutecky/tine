import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { For, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { clearClipboardSlot, copyBlockOutline, peekClipboardSlot } from "../clipboard";
import { setCopyIncludeSubtree } from "../copySettings";
import { startEditing } from "../editorController";
import { initParser } from "../render/parse";
import { doc, loadSingle, pageByName, pageInstanceGeneration, resetStore } from "../store";
import type { BlockDto } from "../types";
import { setGraphMeta } from "../ui";
import { Block } from "./Block";

beforeAll(() => initParser());

afterEach(() => {
  clearClipboardSlot();
  resetStore();
  setGraphMeta(null);
  setCopyIncludeSubtree(true);
  vi.restoreAllMocks();
  document.body.innerHTML = "";
});

function mount(node: () => JSX.Element) {
  const root = document.createElement("div");
  document.body.appendChild(root);
  return { root, dispose: render(node, root) };
}

function paste(textarea: HTMLTextAreaElement, text: string): Event {
  const event = new Event("paste", { bubbles: true, cancelable: true });
  Object.defineProperty(event, "clipboardData", {
    value: {
      files: [],
      items: [],
      types: ["text/plain"],
      getData: (type: string) => type === "text/plain" ? text : "",
    },
  });
  textarea.dispatchEvent(event);
  return event;
}

function seedHost(id: string): void {
  const host: BlockDto = { id, raw: "", collapsed: false, children: [] };
  loadSingle({ name: "Paste", kind: "page", title: "Paste", pre_block: null, blocks: [host], format: "md" });
  startEditing(id, 0);
}

describe("private block payload paste necessity", () => {
  it("preserves a cut id so an existing block reference does not dangle", async () => {
    const host = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    const preserved = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
    seedHost(host);
    setGraphMeta({ root: "/graph" } as any);
    vi.spyOn(backend(), "writeRich").mockResolvedValue();
    vi.spyOn(backend(), "resolveBlocks").mockResolvedValue([null]);
    await copyBlockOutline("cut", "- identity", {
      blocks: [{ raw: `identity\nid:: ${preserved}`, sourceFormat: "md", children: [] }],
      sourcePages: [{
        name: "Paste",
        kind: "page",
        generation: pageInstanceGeneration("Paste")!,
      }],
    });

    const { root, dispose } = mount(() => (
      <For each={pageByName("Paste")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));
    try {
      paste(root.querySelector("textarea")!, "- identity");
      await vi.waitFor(() => {
        expect(doc.byId[preserved]?.raw).toBe(`identity\nid:: ${preserved}`);
        expect(doc.byId[preserved]).toBeDefined();
      });
    } finally {
      dispose();
    }
  });

  it("keeps collapsed metadata and applies it immediately on copy-paste", async () => {
    const host = "cccccccc-cccc-4ccc-8ccc-cccccccccccc";
    seedHost(host);
    setGraphMeta({ root: "/graph" } as any);
    vi.spyOn(backend(), "writeRich").mockResolvedValue();
    await copyBlockOutline("copy", "- folded\n\t- child", {
      blocks: [{
        raw: "folded\ncollapsed:: true",
        sourceFormat: "md",
        children: [{ raw: "child", sourceFormat: "md", children: [] }],
      }],
      sourcePages: [],
    });

    const { root, dispose } = mount(() => (
      <For each={pageByName("Paste")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));
    try {
      paste(root.querySelector("textarea")!, "- folded\n\t- child");
      await vi.waitFor(() => {
        const pasted = doc.byId[pageByName("Paste")!.roots[0]];
        expect(pasted.raw).toBe("folded\ncollapsed:: true");
        expect(pasted.collapsed).toBe(true);
      });
    } finally {
      dispose();
    }
  });

  it("replays the private full subtree for foreign equal text when subtree text-copy is disabled", async () => {
    const host = "dddddddd-dddd-4ddd-8ddd-dddddddddddd";
    seedHost(host);
    setGraphMeta({ root: "/graph" } as any);
    setCopyIncludeSubtree(false);
    vi.spyOn(backend(), "writeRich").mockResolvedValue();
    await copyBlockOutline("copy", "- parent", {
      blocks: [{
        raw: "parent\ncollapsed:: true",
        sourceFormat: "md",
        children: [{ raw: "private child", sourceFormat: "md", children: [] }],
      }],
      sourcePages: [],
    });
    const { root, dispose } = mount(() => (
      <For each={pageByName("Paste")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));
    try {
      paste(root.querySelector("textarea")!, "- parent");
      await vi.waitFor(() => {
        const replayed = doc.byId[pageByName("Paste")!.roots[0]];
        expect(doc.byId[replayed.children[0]].raw).toBe("private child");
        expect(replayed.collapsed).toBe(true);
      });
    } finally {
      dispose();
    }
  });

  it("does not consult or consume the slot for the raw-paste latch", async () => {
    const host = "eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee";
    seedHost(host);
    setGraphMeta({ root: "/graph" } as any);
    vi.spyOn(backend(), "writeRich").mockResolvedValue();
    await copyBlockOutline("cut", "- payload", {
      blocks: [{ raw: `payload\nid:: bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb`, sourceFormat: "md", children: [] }],
      sourcePages: [],
    });
    const { root, dispose } = mount(() => (
      <For each={pageByName("Paste")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));
    try {
      const textarea = root.querySelector("textarea")!;
      textarea.dispatchEvent(new KeyboardEvent("keydown", {
        key: "v", code: "KeyV", ctrlKey: true, shiftKey: true, bubbles: true, cancelable: true,
      }));
      paste(textarea, "- payload");
      expect(doc.byId[host].raw).toBe("- payload");
      expect(peekClipboardSlot()?.op).toBe("cut");
      expect(doc.byId["bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"]).toBeUndefined();
    } finally {
      dispose();
    }
  });

  it("clears the slot on a non-matching non-empty paste", async () => {
    const host = "ffffffff-ffff-4fff-8fff-ffffffffffff";
    seedHost(host);
    setGraphMeta({ root: "/graph" } as any);
    vi.spyOn(backend(), "writeRich").mockResolvedValue();
    await copyBlockOutline("copy", "- payload", {
      blocks: [{ raw: "payload", sourceFormat: "md", children: [] }],
      sourcePages: [],
    });
    const { root, dispose } = mount(() => (
      <For each={pageByName("Paste")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));
    try {
      paste(root.querySelector("textarea")!, "foreign text");
      expect(peekClipboardSlot()).toBeNull();
    } finally {
      dispose();
    }
  });

  it("bypasses the payload branch inside a syntax-sensitive fence", async () => {
    const host = "99999999-9999-4999-8999-999999999999";
    const fenced: BlockDto = { id: host, raw: "```\n\n```", collapsed: false, children: [] };
    loadSingle({ name: "Paste", kind: "page", title: "Paste", pre_block: null, blocks: [fenced], format: "md" });
    startEditing(host, 4);
    setGraphMeta({ root: "/graph" } as any);
    vi.spyOn(backend(), "writeRich").mockResolvedValue();
    await copyBlockOutline("cut", "- payload", {
      blocks: [{ raw: `payload\nid:: bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb`, sourceFormat: "md", children: [] }],
      sourcePages: [],
    });
    const { root, dispose } = mount(() => (
      <For each={pageByName("Paste")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));
    try {
      const textarea = root.querySelector("textarea")!;
      textarea.setSelectionRange(4, 4);
      paste(textarea, "- payload");
      expect(peekClipboardSlot()?.op).toBe("cut");
      expect(doc.byId["bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"]).toBeUndefined();
    } finally {
      dispose();
    }
  });
});
