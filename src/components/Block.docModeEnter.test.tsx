import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { For } from "solid-js";
import { render } from "solid-js/web";
import { endEdit, startEditing } from "../editorController";
import { initParser } from "../render/parse";
import { doc, loadSingle, pageByName, resetStore } from "../store";
import type { BlockDto, PageDto } from "../types";
import { setDocumentMode, setGraphMeta } from "../ui";
import { Block } from "./Block";

beforeAll(() => initParser());

afterEach(() => {
  endEdit("page-navigation");
  resetStore();
  setDocumentMode(false);
  setGraphMeta(null);
  document.body.innerHTML = "";
});

function page(raw: string): PageDto {
  const block: BlockDto = { id: "doc-mode-enter", raw, collapsed: false, children: [] };
  return { name: "Document mode Enter", kind: "page", title: "Document mode Enter", pre_block: null, blocks: [block] };
}

function mountEditor(raw = "alpha") {
  loadSingle(page(raw));
  startEditing("doc-mode-enter", raw.length);
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => (
    <For each={pageByName("Document mode Enter")?.roots ?? []}>{(id) => <Block id={id} />}</For>
  ), root);
  const editor = root.querySelector<HTMLTextAreaElement>("textarea.block-editor");
  if (!editor) throw new Error("missing active editor");
  editor.focus();
  editor.setSelectionRange(raw.length, raw.length);
  return { root, editor, dispose };
}

function pressEnter(editor: HTMLTextAreaElement, shiftKey = false) {
  const event = new KeyboardEvent("keydown", {
    key: "Enter", code: "Enter", shiftKey, bubbles: true, cancelable: true,
  });
  editor.dispatchEvent(event);
  return event;
}

describe("document-mode Enter mapping", () => {
  it("keeps the ordinary mapping outside document mode", async () => {
    setDocumentMode(false);
    const newline = mountEditor();
    try {
      expect(pressEnter(newline.editor, true).defaultPrevented).toBe(true);
      await vi.waitFor(() => expect(doc.byId["doc-mode-enter"].raw).toBe("alpha\n"));
      expect(doc.pages[0].roots).toEqual(["doc-mode-enter"]);
    } finally {
      newline.dispose();
    }

    resetStore();
    const split = mountEditor();
    try {
      expect(pressEnter(split.editor).defaultPrevented).toBe(true);
      await vi.waitFor(() => expect(doc.pages[0].roots).toHaveLength(2));
      expect(doc.byId["doc-mode-enter"].raw).toBe("alpha");
      expect(doc.byId[doc.pages[0].roots[1]].raw).toBe("");
    } finally {
      split.dispose();
    }
  });

  it("flips plain Enter and Shift+Enter in document mode", async () => {
    setDocumentMode(true);
    const newline = mountEditor();
    try {
      expect(pressEnter(newline.editor).defaultPrevented).toBe(true);
      await vi.waitFor(() => expect(doc.byId["doc-mode-enter"].raw).toBe("alpha\n"));
      expect(doc.pages[0].roots).toEqual(["doc-mode-enter"]);
    } finally {
      newline.dispose();
    }

    resetStore();
    const split = mountEditor();
    try {
      expect(pressEnter(split.editor, true).defaultPrevented).toBe(true);
      await vi.waitFor(() => expect(doc.pages[0].roots).toHaveLength(2));
      expect(doc.byId["doc-mode-enter"].raw).toBe("alpha");
      expect(doc.byId[doc.pages[0].roots[1]].raw).toBe("");
    } finally {
      split.dispose();
    }
  });

  it("honors the doc-mode Enter escape hatch", async () => {
    setDocumentMode(true);
    setGraphMeta({ doc_mode_enter_for_new_block: true } as never);
    const { editor, dispose } = mountEditor();
    try {
      expect(pressEnter(editor).defaultPrevented).toBe(true);
      await vi.waitFor(() => expect(doc.pages[0].roots).toHaveLength(2));
      expect(doc.byId["doc-mode-enter"].raw).toBe("alpha");
    } finally {
      dispose();
    }
  });
});
