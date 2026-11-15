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

function editorWithPreedit(preedit: string, raw = "", caret = 0) {
  loadSingle(page(raw));
  startEditing("ime-blur", caret);
  const { root, dispose } = mount(() => (
    <For each={pageByName("IME blur")?.roots ?? []}>{(id) => <Block id={id} />}</For>
  ));
  const editor = maybeEditorIn(root) as EditorDriver;
  editor.compositionStart(new CompositionEvent("compositionstart", { bubbles: true }));
  const text = raw.slice(0, caret) + preedit + raw.slice(caret);
  editor.input(text, caret + preedit.length, { inputType: "insertCompositionText", data: preedit, isComposing: true });
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
      expect(doc.byId["ime-blur"].raw).toBe("あ"); // the blur commits the live buffer first; ending the preedit must not defer or lose it
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

  it("ends a language typed at a bare Org opener with its separator when the blur also leaves the editor (#879)", async () => {
    vi.spyOn(document, "hasFocus").mockReturnValue(true); // focus went elsewhere in the window
    const opener = "#+BEGIN_SRC";
    const { dispose } = editorWithPreedit("rust", `${opener}\nx\n#+END_SRC`, opener.length);
    const other = document.createElement("button");
    document.body.append(other);
    try {
      other.focus(); // focus moves elsewhere without compositionend; the editor exits
      await Promise.resolve();
      expect(doc.byId["ime-blur"].raw).toBe("#+BEGIN_SRC rust\nx\n#+END_SRC");
    } finally {
      dispose();
    }
  });
});
