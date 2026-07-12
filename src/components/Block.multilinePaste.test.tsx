import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { For, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { doc, loadSingle, pageByName, resetStore, undo } from "../store";
import { startEditing } from "../editorController";
import type { BlockDto, PageDto } from "../types";
import { Block } from "./Block";

beforeAll(() => initParser());
afterEach(() => { resetStore(); document.body.innerHTML = ""; });

function mount(node: () => JSX.Element) {
  const root = document.createElement("div");
  document.body.appendChild(root);
  return { root, dispose: render(node, root) };
}

function paste(textarea: HTMLTextAreaElement, text: string, html = ""): Event {
  const event = new Event("paste", { bubbles: true, cancelable: true });
  Object.defineProperty(event, "clipboardData", {
    value: {
      files: [],
      items: [],
      types: html ? ["text/plain", "text/html"] : ["text/plain"],
      getData: (type: string) => type === "text/html" ? html : text,
    },
  });
  textarea.dispatchEvent(event);
  return event;
}

function keydown(textarea: HTMLTextAreaElement, init: KeyboardEventInit) {
  textarea.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init }));
}

describe("multiline paste into editor-visible empty blocks", () => {
  it("replaces an id-only host instead of leaving a ghost blank bullet", () => {
    const block: BlockDto = {
      id: "11111111-1111-4111-8111-111111111111",
      raw: "id:: 11111111-1111-4111-8111-111111111111",
      collapsed: false,
      children: [],
    };
    const page: PageDto = { name: "Paste", kind: "page", title: "Paste", pre_block: null, blocks: [block] };
    loadSingle(page);
    startEditing(block.id, 0);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Paste")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));
    try {
      const textarea = root.querySelector("textarea") as HTMLTextAreaElement;
      expect(textarea.value).toBe("");
      paste(textarea, "- first\n- second");
      expect(pageByName("Paste")!.roots).toHaveLength(2);
      expect(pageByName("Paste")!.roots[0]).toBe(block.id);
      expect(doc.byId[block.id].raw).toBe("first\nid:: 11111111-1111-4111-8111-111111111111");
      expect(doc.byId[pageByName("Paste")!.roots[1]].raw).toBe("second");
    } finally {
      dispose();
    }
  });

  it("keeps Ctrl+Shift+V multiline plain text inside the current block", () => {
    const block: BlockDto = {
      id: "22222222-2222-4222-8222-222222222222",
      raw: "before after",
      collapsed: false,
      children: [],
    };
    loadSingle({ name: "Paste", kind: "page", title: "Paste", pre_block: null, blocks: [block] });
    startEditing(block.id, 7);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Paste")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));
    try {
      const textarea = root.querySelector("textarea") as HTMLTextAreaElement;
      textarea.setSelectionRange(7, 7);
      keydown(textarea, { key: "v", code: "KeyV", ctrlKey: true, shiftKey: true });
      const event = paste(textarea, "first\nsecond");

      expect(event.defaultPrevented).toBe(true);
      expect(pageByName("Paste")!.roots).toEqual([block.id]);
      expect(doc.byId[block.id].raw).toBe("before first\nsecondafter");
    } finally {
      dispose();
    }
  });

  it("does not leak an aborted Ctrl+Shift+V gesture into a later paste", () => {
    const block: BlockDto = { id: "33333333-3333-4333-8333-333333333333", raw: "", collapsed: false, children: [] };
    loadSingle({ name: "Paste", kind: "page", title: "Paste", pre_block: null, blocks: [block] });
    startEditing(block.id, 0);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Paste")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));
    try {
      const textarea = root.querySelector("textarea") as HTMLTextAreaElement;
      keydown(textarea, { key: "v", code: "KeyV", ctrlKey: true, shiftKey: true });
      textarea.dispatchEvent(new KeyboardEvent("keyup", { key: "v", code: "KeyV", bubbles: true }));
      paste(textarea, "first\nsecond");
      expect(pageByName("Paste")!.roots).toHaveLength(2);
      expect(pageByName("Paste")!.roots.map((id) => doc.byId[id].raw)).toEqual(["first", "second"]);
    } finally {
      dispose();
    }
  });

  it("atomically replaces an empty block with a structured HTML outline", () => {
    const block: BlockDto = { id: "44444444-4444-4444-8444-444444444444", raw: "", collapsed: false, children: [] };
    loadSingle({ name: "Paste", kind: "page", title: "Paste", pre_block: null, blocks: [block] });
    startEditing(block.id, 0);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Paste")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));
    try {
      const textarea = root.querySelector("textarea") as HTMLTextAreaElement;
      paste(textarea, "Parent\nChild\nSibling", "<ul><li>Parent<ul><li>Child</li></ul></li><li>Sibling</li></ul>");
      const roots = pageByName("Paste")!.roots;
      expect(roots).toHaveLength(2);
      expect(doc.byId[roots[0]].raw).toBe("Parent");
      expect(doc.byId[doc.byId[roots[0]].children[0]].raw).toBe("Child");
      expect(doc.byId[roots[1]].raw).toBe("Sibling");

      undo();
      expect(pageByName("Paste")!.roots).toEqual([block.id]);
      expect(doc.byId[block.id].raw).toBe("");
    } finally {
      dispose();
    }
  });

  it("keeps a Markdown table source in one block", () => {
    const block: BlockDto = { id: "55555555-5555-4555-8555-555555555555", raw: "", collapsed: false, children: [] };
    const table = "| Name | Value |\n| --- | ---: |\n| Alpha | 1 |";
    loadSingle({ name: "Paste", kind: "page", title: "Paste", pre_block: null, blocks: [block] });
    startEditing(block.id, 0);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Paste")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));
    try {
      paste(root.querySelector("textarea") as HTMLTextAreaElement, table);
      const roots = pageByName("Paste")!.roots;
      expect(roots).toHaveLength(1);
      expect(doc.byId[roots[0]].raw).toBe(table);
    } finally {
      dispose();
    }
  });

  it("uses plain text when Ctrl+Shift+V accompanies structured HTML", () => {
    const block: BlockDto = { id: "66666666-6666-4666-8666-666666666666", raw: "before", collapsed: false, children: [] };
    loadSingle({ name: "Paste", kind: "page", title: "Paste", pre_block: null, blocks: [block] });
    startEditing(block.id, 6);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Paste")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));
    try {
      const textarea = root.querySelector("textarea") as HTMLTextAreaElement;
      textarea.setSelectionRange(6, 6);
      keydown(textarea, { key: "v", code: "KeyV", ctrlKey: true, shiftKey: true });
      paste(textarea, "\nParent\nChild", "<ul><li>Parent<ul><li>Child</li></ul></li></ul>");
      expect(pageByName("Paste")!.roots).toEqual([block.id]);
      expect(doc.byId[block.id].raw).toBe("before\nParent\nChild");
    } finally {
      dispose();
    }
  });
});
