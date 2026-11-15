import { afterEach, describe, expect, it, vi } from "vitest";
import { node, selectBlock, undo } from "../document";
import { loadSingle } from "../document/workingSet";
import { endEdit } from "../editorController";
import { installKeybindings } from "../keybindings";
import { editorIn, type EditorKind } from "../tests/editorDriver";
import { useEditorKind } from "../tests/editorKind";
import { blk, installBlockEditorLifecycle, mount, page } from "../tests/blockEditorTestkit";
import { Block } from "./Block";
import { backend } from "../backend";

installBlockEditorLifecycle();
afterEach(() => {
  endEdit("page-navigation");
  vi.restoreAllMocks();
  delete (document as { caretRangeFromPoint?: unknown }).caretRangeFromPoint;
});

// D-20261114-CS879: source/body is an entry choice, never a caret-dependent view.
describe.each<EditorKind>(["textarea", "cm6"])("code mode in %s", kind => {
  useEditorKind(kind);
  it.each([false, true])("keeps the chosen mode across undo (body: %s)", body => {
    const raw = "```js\none\ntwo\n```";
    loadSingle(page("Undo mode", [blk("code", raw)]));
    const { root, dispose } = mount(() => <Block id="code" />);
    try {
      if (body) {
        root.querySelector("pre.code-block > code")!.dispatchEvent(new MouseEvent("mousedown", { button: 0, bubbles: true, cancelable: true }));
        document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
      } else root.querySelector<HTMLButtonElement>("button.code-language")!.click();
      const editor = editorIn(root);
      editor.select(body ? 1 : raw.indexOf("one") + 1); editor.type("x");
      expect(undo()).toBe(true);
      expect(editorIn(root).value()).toBe(body ? "one\ntwo" : raw);
      expect(editorIn(root).element.classList.contains("code-edit")).toBe(body);
      expect(node("code").raw).toBe(raw);
    } finally { dispose(); }
  });
  // Review HOLD 3dbab7a56: undo restores a selection past the now shorter body; the body stays chosen.
  it("keeps the code body when undo shortens it below the restored selection", () => {
    const raw = "```js\nx\n```";
    loadSingle(page("Undo shorter", [blk("code", raw)]));
    const { root, dispose } = mount(() => <Block id="code" />);
    try {
      root.querySelector("pre.code-block > code")!.dispatchEvent(new MouseEvent("mousedown", { button: 0, bubbles: true, cancelable: true }));
      document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
      const editor = editorIn(root);
      editor.select(1); editor.type("a long appended string");
      expect(undo()).toBe(true);
      expect(editorIn(root).value()).toBe("x");
      expect(editorIn(root).element.classList.contains("code-edit")).toBe(true);
      expect(node("code").raw).toBe(raw);
    } finally { dispose(); }
  });
  it.each((["md", "org"] as const).flatMap(format => [false, true].map(empty => ({ format, empty }))))(
    "opens the code card without a caret range: %j", ({ format, empty }) => {
      const open = format === "md" ? "```js\n" : "#+BEGIN_SRC js\n";
      const close = format === "md" ? "\n```" : "\n#+END_SRC";
      const body = empty ? "" : "one\ntwo", raw = open + body + close;
      loadSingle({ ...page("Code target", [blk("code", raw)]), format });
      const { root, dispose } = mount(() => <Block id="code" />);
      try {
        root.querySelector(empty ? "pre.code-block" : "pre.code-block > code")!.dispatchEvent(new MouseEvent("mousedown", { button: 0, bubbles: true, cancelable: true }));
        document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
        const editor = editorIn(root);
        expect(editor.value()).toBe(body);
        expect(editor.element.classList.contains("code-edit")).toBe(true);
        expect(editor.selection()).toEqual([body.length, body.length]);
        endEdit("page-navigation");
        expect(node("code").raw).toBe(raw);
      } finally { dispose(); }
    });
  it("keeps existing source visible after choosing its language", async () => {
    const raw = "```js\none\n```";
    loadSingle(page("Language mode", [blk("code", raw)]));
    const { root, dispose } = mount(() => <Block id="code" />);
    try {
      root.querySelector<HTMLButtonElement>("button.code-language")!.click();
      const editor = editorIn(root), [at] = editor.selection();
      editor.select(at, at + 2); editor.type("js");
      editor.reportInput({ bubbles: true, inputType: "insertText", data: "s" });
      await vi.waitFor(() => expect(document.querySelector(".autocomplete .ac-label")?.textContent).toBe("JavaScript"));
      document.querySelector(".autocomplete .ac-item")!.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true }));
      await vi.waitFor(() => expect(editor.value()).toBe(raw.replace("js", "javascript")));
      expect(editor.element.classList.contains("code-edit")).toBe(false);
      expect(node("code").raw).toBe(raw.replace("js", "javascript"));
    } finally { dispose(); }
  });
  it.each((["md", "org"] as const).flatMap(format => [false, true].flatMap(mixed =>
    ["code", "elsewhere", "language", "Enter"].map(target => ({ format, mixed, target })))))(
    "keeps the mode chosen by entry: %j", ({ format, mixed, target }) => {
      const open = format === "md" ? "```js\n" : "#+BEGIN_SRC js\n";
      const close = format === "md" ? "\n```" : "\n#+END_SRC";
      const raw = `${mixed ? "Before\n" : ""}${open}one\ntwo${close}${mixed ? "\nAfter" : ""}`;
      loadSingle({ ...page("Mode", [blk("code", raw)]), format });
      const { root, dispose } = mount(() => <Block id="code" />);
      const release = installKeybindings();
      const reads = vi.spyOn(backend(), "getPage"), submits = vi.spyOn(backend(), "pageSubmit");
      try {
        if (target === "language") root.querySelector<HTMLButtonElement>("button.code-language")!.click();
        else if (target === "Enter") {
          selectBlock("code");
          document.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
        } else {
          const content = root.querySelector<HTMLElement>(".block-content")!;
          const code = content.querySelector("pre.code-block > code")!;
          // At a card's outside edge the engine may report its nearest code
          // glyph. The actual press target still determines source versus body.
          const text = document.createTreeWalker(code, NodeFilter.SHOW_TEXT).nextNode()!;
          (document as unknown as { caretRangeFromPoint: () => Range }).caretRangeFromPoint = () => {
            const range = document.createRange(); range.setStart(text, 1); range.collapse(true); return range;
          };
          (target === "code" ? code : content).dispatchEvent(new MouseEvent("mousedown", { button: 0, bubbles: true, cancelable: true }));
          document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
        }
        const editor = editorIn(root);
        const body = target === "code";
        expect(editor.value()).toBe(body ? "one\ntwo" : raw);
        expect(editor.element.classList.contains("code-edit")).toBe(body);
        if (!body && kind === "cm6") expect(editor.element.textContent).toContain(open.trimEnd());
        expect(root.querySelector(".calc-lineno") !== null).toBe(body);
        // A collapsed native selection inside code must not hide raw fences.
        const caret = body ? 2 : raw.indexOf("one") + 2;
        editor.selectByUser(caret);
        editor.reportSelection(new Event("select", { bubbles: true }));
        expect(editor.value()).toBe(body ? "one\ntwo" : raw);
        expect(editor.element.classList.contains("code-edit")).toBe(body);
        if (!body && kind === "cm6") expect(editor.element.textContent).toContain(close.trimStart());
        if (body) {
          editor.keydown({ key: "Escape" });
          expect(editor.value()).toBe(raw);
          expect(editor.element.classList.contains("code-edit")).toBe(false);
        }
        // Native selection spanning a fence stays source; collapsing it also stays source.
        editor.selectByUser(0, raw.indexOf("two") + 1);
        expect(editor.value()).toBe(raw);
        editor.selectByUser(raw.indexOf("two"));
        expect(editor.value()).toBe(raw);
        endEdit("page-navigation");
        expect(node("code").raw).toBe(raw);
        expect(reads).not.toHaveBeenCalled();
        expect(submits).not.toHaveBeenCalled();
      } finally { release(); dispose(); }
    });
});
