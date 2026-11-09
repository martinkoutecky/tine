import { maybeEditorIn, type EditorDriver } from "../tests/editorDriver";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { For, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { startEditing } from "../editorController";
import { initParser } from "../render/parse";
import { pageByName, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import type { BlockDto, PageDto } from "../types";
import { Block } from "./Block";

// L9-18: an IME the platform cancels on blur may never send compositionend. The
// composition transaction must end with the blur, or the editor that stays open
// (the window lost focus, a picker took focus) swallows every later edit as
// preedit. Both editors run this file: the textarea and CodeMirror each track it.
beforeAll(() => initParser());

afterEach(() => {
  vi.restoreAllMocks();
  resetStore();
  document.body.innerHTML = "";
});

function mount(node: () => JSX.Element) {
  const root = document.createElement("div");
  document.body.append(root);
  return { root, dispose: render(node, root) };
}

function page(raw: string): PageDto {
  const block: BlockDto = { id: "ime-blur", raw, collapsed: false, children: [] };
  return { name: "IME blur", kind: "page", title: "IME blur", pre_block: null, blocks: [block] };
}

function editorWithPreedit(preedit: string) {
  loadSingle(page(""));
  startEditing("ime-blur", 0);
  const { root, dispose } = mount(() => (
    <For each={pageByName("IME blur")?.roots ?? []}>{(id) => <Block id={id} />}</For>
  ));
  const editor = maybeEditorIn(root) as EditorDriver;
  editor.compositionStart(new CompositionEvent("compositionstart", { bubbles: true }));
  editor.input(preedit, preedit.length, { inputType: "insertCompositionText", data: preedit, isComposing: true });
  return { editor, dispose };
}

describe("a composition abandoned by blur", () => {
  it("does not swallow the next ordinary edit when the window loses focus mid-composition", async () => {
    // The window-focus branch keeps the same editor open and in edit mode.
    vi.spyOn(document, "hasFocus").mockReturnValue(false);
    const { editor, dispose } = editorWithPreedit("あ");
    try {
      expect(doc.byId["ime-blur"].raw).toBe("");
      editor.reportBlur(); // blur without compositionend
      await Promise.resolve(); // CM finishes the ended preedit in a microtask
      editor.input("あい", 2, { inputType: "insertText", data: "い" });
      expect(doc.byId["ime-blur"].raw).toBe("あい");
    } finally {
      dispose();
    }
  });

  it("keeps a later composition a composition: the preedit stays local until its own compositionend", async () => {
    vi.spyOn(document, "hasFocus").mockReturnValue(false);
    const { editor, dispose } = editorWithPreedit("あ");
    try {
      editor.reportBlur();
      await Promise.resolve();
      editor.compositionStart(new CompositionEvent("compositionstart", { bubbles: true }));
      editor.input("あか", 2, { inputType: "insertCompositionText", data: "か", isComposing: true });
      expect(doc.byId["ime-blur"].raw).toBe("あ");
      editor.compositionEnd(new CompositionEvent("compositionend", { bubbles: true }));
      await Promise.resolve();
      expect(doc.byId["ime-blur"].raw).toBe("あか");
    } finally {
      dispose();
    }
  });
});
