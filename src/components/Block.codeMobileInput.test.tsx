import { afterEach, describe, expect, it, vi } from "vitest";
import { node } from "../document";
import { loadSingle } from "../document/workingSet";
import { endEdit, startEditing } from "../editorController";
import { editorIn } from "../tests/editorDriver";
import { blk, installBlockEditorLifecycle, mount, page } from "../tests/blockEditorTestkit";
import { Block } from "./Block";

installBlockEditorLifecycle();
afterEach(() => endEdit("page-navigation"));

async function newCode(entry: "typed" | "slash") {
  loadSingle(page("Mobile code", [blk("code", "")]));
  startEditing("code", 0);
  const mounted = mount(() => <Block id="code" />);
  const editor = editorIn(mounted.root);
  if (entry === "typed") {
    for (const ch of "```") editor.type(ch);
  } else {
    editor.input("/code", 5);
    await vi.waitFor(() => expect(document.querySelector(".ac-label")?.textContent).toBe("Code block"));
    editor.keydown({ key: "Enter", bubbles: true, cancelable: true });
  }
  await vi.waitFor(() => expect(document.querySelector(".ac-label")?.textContent).toBe("JavaScript"));
  expect(editor.value()).toBe("```\n\n```");
  // A phone can skip the picker by tapping the empty body. The subsequent
  // selectionchange can arrive after input; it does not dismiss this picker.
  editor.select(4, 4);
  return { ...mounted, editor };
}

describe.each(["typed", "slash"] as const)("new code via %s (GH #879)", entry => {
  it.each(["123", "abc"])("keeps direct input %s in order and the caret after every character", async text => {
    const { editor, dispose } = await newCode(entry);
    try {
      for (const [i, ch] of [...text].entries()) {
        editor.type(ch);
        await Promise.resolve();
        editor.element.ownerDocument.dispatchEvent(new Event("selectionchange"));
        expect(editor.value()).toBe(text.slice(0, i + 1));
        expect(editor.selection()).toEqual([i + 1, i + 1]);
      }
      endEdit("page-navigation");
      expect(node("code")!.raw).toBe("```\n" + text + "\n```");
    } finally { dispose(); }
  });

  it("keeps the caret after the IME's committed text", async () => {
    const { editor, dispose } = await newCode(entry);
    try {
      editor.compositionStart();
      editor.input("```\n123\n```", 7, { inputType: "insertCompositionText", data: "123", isComposing: true });
      expect(node("code")!.raw).toBe("```\n\n```");
      editor.compositionEnd("123");
      await Promise.resolve();
      editor.element.ownerDocument.dispatchEvent(new Event("selectionchange"));
      expect(editor.value()).toBe("123");
      expect(editor.selection()).toEqual([3, 3]);
      editor.type("4");
      endEdit("page-navigation");
      expect(node("code")!.raw).toBe("```\n1234\n```");
    } finally { dispose(); }
  });
});
