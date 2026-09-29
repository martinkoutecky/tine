import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { For, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { startEditing } from "../editorController";
import { initParser } from "../render/parse";
import { pageByName, resetStore } from "../document";
import { doc } from "../document/model";
import { loadSingle } from "../document/workingSet";
import type { BlockDto, PageDto } from "../types";
import { Block } from "./Block";

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  vi.restoreAllMocks();
  resetStore();
  document.body.innerHTML = "";
});

function mount(node: () => JSX.Element): { root: HTMLDivElement; dispose: () => void } {
  const root = document.createElement("div");
  document.body.appendChild(root);
  return { root, dispose: render(node, root) };
}

let counter = 0;
function block(raw: string, children: BlockDto[] = []): BlockDto {
  return { id: `t${counter++}`, raw, collapsed: false, children };
}

function page(blocks: BlockDto[]): PageDto {
  return { name: "Caret", kind: "page", title: "Caret", pre_block: null, blocks };
}

function mountPage(): { root: HTMLDivElement; dispose: () => void } {
  return mount(() => <For each={pageByName("Caret")?.roots ?? []}>{(id) => <Block id={id} />}</For>);
}

const editor = (root: HTMLElement) => root.querySelector("textarea.block-editor") as HTMLTextAreaElement;
const key = (el: HTMLTextAreaElement, k: string, init: KeyboardEventInit = {}) =>
  el.dispatchEvent(new KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true, ...init }));

describe("horizontal boundary navigation and forward merge (GH #213)", () => {
  it("ArrowLeft at block start moves to the end of the previous block", () => {
    loadSingle(page([block("first"), block("second"), block("third")]));
    const secondId = pageByName("Caret")!.roots[1];
    startEditing(secondId, 0);
    const m = mountPage();
    try {
      const ta = editor(m.root);
      expect(ta.value).toBe("second");
      ta.setSelectionRange(0, 0);
      key(ta, "ArrowLeft");
      const ta2 = editor(m.root);
      expect(ta2.value).toBe("first");
      expect(ta2.selectionStart).toBe("first".length);
    } finally {
      m.dispose();
    }
  });

  it("ArrowRight at block end moves to the start of the next block", () => {
    loadSingle(page([block("first"), block("second"), block("third")]));
    const firstId = pageByName("Caret")!.roots[0];
    startEditing(firstId, 2);
    const m = mountPage();
    try {
      const ta = editor(m.root);
      expect(ta.value).toBe("first");
      ta.setSelectionRange(5, 5);
      key(ta, "ArrowRight");
      const ta2 = editor(m.root);
      expect(ta2.value).toBe("second");
      expect(ta2.selectionStart).toBe(0);
      expect(ta2.selectionEnd).toBe(0);
    } finally {
      m.dispose();
    }
  });

  it("ArrowLeft at start does not cross when a modifier key is held", () => {
    loadSingle(page([block("first"), block("second")]));
    const secondId = pageByName("Caret")!.roots[1];
    startEditing(secondId, 0);
    const m = mountPage();
    try {
      const ta = editor(m.root);
      ta.setSelectionRange(0, 0);
      key(ta, "ArrowLeft", { ctrlKey: true });
      expect(editor(m.root).value).toBe("second");
      key(ta, "ArrowLeft", { shiftKey: true });
      expect(editor(m.root).value).toBe("second");
    } finally {
      m.dispose();
    }
  });

  it("ArrowLeft in mid-text stays native and keeps editing the same block", () => {
    loadSingle(page([block("first"), block("second")]));
    const secondId = pageByName("Caret")!.roots[1];
    startEditing(secondId, 3);
    const m = mountPage();
    try {
      const ta = editor(m.root);
      ta.setSelectionRange(3, 3);
      key(ta, "ArrowLeft");
      // No cross-block jump: still editing "second" (caret movement left to the DOM).
      expect(editor(m.root).value).toBe("second");
    } finally {
      m.dispose();
    }
  });

  it("Delete at block end merges the next block's text and children, caret at the join point", async () => {
    loadSingle(page([block("foo"), block("bar", [block("child")])]));
    const fooId = pageByName("Caret")!.roots[0];
    startEditing(fooId, 3);
    const m = mountPage();
    try {
      const ta = editor(m.root);
      expect(ta.value).toBe("foo");
      ta.setSelectionRange(3, 3);
      key(ta, "Delete");
      expect(pageByName("Caret")!.roots).toEqual([fooId]);
      expect(doc.byId[fooId].raw).toBe("foobar");
      expect(doc.byId[fooId].children).toHaveLength(1);
      const ta2 = editor(m.root);
      await vi.waitFor(() => expect(editor(m.root).value).toBe("foobar"));
      expect(ta2).not.toBeNull();
      expect(editor(m.root).selectionStart).toBe(3);
    } finally {
      m.dispose();
    }
  });

  it("Delete at block end absorbs an empty next visible block (GH #239)", async () => {
    loadSingle(page([block("kept"), block("")]));
    const keptId = pageByName("Caret")!.roots[0];
    const emptyId = pageByName("Caret")!.roots[1];
    startEditing(keptId, 4);
    const m = mountPage();
    try {
      const ta = editor(m.root);
      ta.setSelectionRange(4, 4);
      key(ta, "Delete");
      expect(pageByName("Caret")!.roots).toEqual([keptId]);
      expect(doc.byId[emptyId]).toBeUndefined();
      expect(doc.byId[keptId].raw).toBe("kept");
      await vi.waitFor(() => expect(editor(m.root).value).toBe("kept"));
      expect(editor(m.root).selectionStart).toBe(4);
    } finally {
      m.dispose();
    }
  });

  it("Delete at end does not merge into a following calc block", () => {
    loadSingle(page([block("foo"), block("```calc\n1 + 1\n```")]));
    const fooId = pageByName("Caret")!.roots[0];
    startEditing(fooId, 3);
    const m = mountPage();
    try {
      const ta = editor(m.root);
      ta.setSelectionRange(3, 3);
      key(ta, "Delete");
      expect(pageByName("Caret")!.roots).toHaveLength(2);
      expect(editor(m.root).value).toBe("foo");
    } finally {
      m.dispose();
    }
  });

  it("Delete at end of the last block is a no-op", () => {
    loadSingle(page([block("only")]));
    const onlyId = pageByName("Caret")!.roots[0];
    startEditing(onlyId, 4);
    const m = mountPage();
    try {
      const ta = editor(m.root);
      ta.setSelectionRange(4, 4);
      key(ta, "Delete");
      expect(pageByName("Caret")!.roots).toEqual([onlyId]);
      expect(doc.byId[onlyId].raw).toBe("only");
    } finally {
      m.dispose();
    }
  });

  it("Delete merges the next VISIBLE sibling when the block's own child is collapsed away", () => {
    // foo is collapsed, so its child is invisible: the next visible block is
    // the sibling "bar", which is what Delete at end must absorb.
    const foo: BlockDto = { id: "t-foo", raw: "foo", collapsed: true, children: [block("hidden")] };
    loadSingle(page([foo, block("bar")]));
    const fooId = pageByName("Caret")!.roots[0];
    const barId = pageByName("Caret")!.roots[1];
    startEditing(fooId, 3);
    const m = mountPage();
    try {
      const ta = editor(m.root);
      ta.setSelectionRange(3, 3);
      key(ta, "Delete");
      expect(doc.byId[fooId].raw).toBe("foobar");
      expect(doc.byId[barId]).toBeUndefined();
      // foo's own hidden child survives untouched under foo.
      expect(doc.byId[fooId].children).toHaveLength(1);
      expect(doc.byId[doc.byId[fooId].children[0]].raw).toBe("hidden");
    } finally {
      m.dispose();
    }
  });
});
