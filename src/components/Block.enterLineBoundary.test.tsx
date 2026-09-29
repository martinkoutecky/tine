// GH #361: pressing Enter with the caret at an in-block line boundary splits
// the block there and consumes the boundary newline, so neither block shows
// an empty line. Shift+Enter keeps inserting an in-block newline.
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { For } from "solid-js";
import { render } from "solid-js/web";
import { endEdit, startEditing } from "../editorController";
import { initParser } from "../render/parse";
import { pageByName, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import type { BlockDto, PageDto } from "../types";
import { Block } from "./Block";

beforeAll(() => initParser());

afterEach(() => {
  endEdit("page-navigation");
  resetStore();
  document.body.innerHTML = "";
});

function page(raw: string): PageDto {
  const block: BlockDto = { id: "boundary-enter", raw, collapsed: false, children: [] };
  return { name: "Boundary Enter", kind: "page", title: "Boundary Enter", pre_block: null, blocks: [block] };
}

function mountEditor(raw: string, offset: number) {
  loadSingle(page(raw));
  startEditing("boundary-enter", offset);
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => (
    <For each={pageByName("Boundary Enter")?.roots ?? []}>{(id) => <Block id={id} />}</For>
  ), root);
  const editor = root.querySelector<HTMLTextAreaElement>("textarea.block-editor")!;
  editor.focus();
  editor.setSelectionRange(offset, offset);
  return { root, editor, dispose };
}

function pressEnter(editor: HTMLTextAreaElement, shiftKey = false) {
  editor.dispatchEvent(new KeyboardEvent("keydown", {
    key: "Enter", code: "Enter", shiftKey, bubbles: true, cancelable: true,
  }));
}

describe("Enter at an in-block line boundary (GH #361)", () => {
  it("splits into 'line1' and 'line2' with no retained empty line", () => {
    const { root, editor, dispose } = mountEditor("line1\nline2", 6);
    try {
      pressEnter(editor);
      const roots = pageByName("Boundary Enter")!.roots;
      expect(roots).toHaveLength(2);
      expect(doc.byId[roots[0]].raw).toBe("line1");
      expect(doc.byId[roots[1]].raw).toBe("line2");
      // The caret lands at the start of the new block's actual text.
      const nextEditor = root.querySelector<HTMLTextAreaElement>("textarea.block-editor")!;
      expect(nextEditor.value).toBe("line2");
      expect(nextEditor.selectionStart).toBe(0);
    } finally {
      dispose();
    }
  });

  it("keeps Shift+Enter inserting an in-block newline", () => {
    const { editor, dispose } = mountEditor("line1", 5);
    try {
      pressEnter(editor, true);
      expect(pageByName("Boundary Enter")!.roots).toHaveLength(1);
      expect(doc.byId["boundary-enter"].raw).toBe("line1\n");
    } finally {
      dispose();
    }
  });

  it("leaves an ordinary mid-line split working exactly as before", () => {
    const { editor, dispose } = mountEditor("line1\nline2", 3);
    try {
      pressEnter(editor);
      const roots = pageByName("Boundary Enter")!.roots;
      expect(roots).toHaveLength(2);
      expect(doc.byId[roots[0]].raw).toBe("lin");
      expect(doc.byId[roots[1]].raw).toBe("e1\nline2");
    } finally {
      dispose();
    }
  });
});
