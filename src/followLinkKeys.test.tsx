import { editorIn, type EditorDriver } from "./tests/editorDriver";
import { afterEach, describe, expect, it, vi } from "vitest";
import { installKeybindings } from "./keybindings";
import * as router from "./router";
import * as ui from "./ui";

// GH #274 at the real entry: a Ctrl+O / Ctrl+Shift+O keydown reaches the
// installed global keybinding layer while a block textarea has focus.
afterEach(() => {
  vi.restoreAllMocks();
  document.body.innerHTML = "";
});

function editorWith(text: string, caret: number): EditorDriver {
  const ta = document.createElement("textarea");
  ta.className = "block-editor";
  document.body.appendChild(ta);
  const editor = editorIn();
  editor.replaceText(text);
  editor.focus();
  editor.select(caret, caret);
  return editor;
}

function press(target: EditorDriver, init: KeyboardEventInit & { key: string }): KeyboardEvent {
  return target.keydown({ bubbles: true, cancelable: true, ...init });
}

describe("Ctrl+O follows the link at the caret (GH #274)", () => {
  it("mod+o navigates to the page ref nearest the caret", () => {
    const openPage = vi.spyOn(router, "openPage").mockImplementation(() => {});
    const dispose = installKeybindings();
    const ta = editorWith("see [[Some Page]] here", 8);
    press(ta, { key: "o", code: "KeyO", ctrlKey: true });
    expect(openPage).toHaveBeenCalledWith("Some Page");
    dispose();
  });

  it("mod+shift+o opens it in the right sidebar instead", () => {
    const sidebar = vi.spyOn(ui, "openPageInSidebar").mockImplementation(() => {});
    const openPage = vi.spyOn(router, "openPage").mockImplementation(() => {});
    const dispose = installKeybindings();
    const ta = editorWith("see [[Some Page]]", 8);
    press(ta, { key: "O", code: "KeyO", ctrlKey: true, shiftKey: true });
    expect(sidebar).toHaveBeenCalledWith("Some Page");
    expect(openPage).not.toHaveBeenCalled();
    dispose();
  });
});
