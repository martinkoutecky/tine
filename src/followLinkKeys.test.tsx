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

function editorWith(text: string, caret: number): HTMLTextAreaElement {
  const ta = document.createElement("textarea");
  ta.className = "block-editor";
  ta.value = text;
  document.body.appendChild(ta);
  ta.focus();
  ta.setSelectionRange(caret, caret);
  return ta;
}

function press(target: HTMLElement, init: KeyboardEventInit): KeyboardEvent {
  const e = new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init });
  target.dispatchEvent(e);
  return e;
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
