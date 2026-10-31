import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { installMiddleClickPasteGuard } from "./middleClickPaste";

// UI-MIDDLE-CLICK-LINK-PASTES. jsdom has no PRIMARY selection and never pastes
// on a middle release, so this drives the page-visible sequence WebKitGTK
// produces (mousedown, mouseup, then the cancelable paste at the focused
// editor) and pins which pastes the guard cancels.

let uninstall: () => void;
beforeEach(() => { uninstall = installMiddleClickPasteGuard(); });
afterEach(() => {
  uninstall();
  document.body.innerHTML = "";
});

function middleClick(target: Element) {
  target.dispatchEvent(new MouseEvent("mousedown", { button: 1, bubbles: true, cancelable: true }));
  target.dispatchEvent(new MouseEvent("mouseup", { button: 1, bubbles: true, cancelable: true }));
}

/** The paste WebKit dispatches at the focused editor; true when it went through. */
function pasteAt(editor: Element): boolean {
  return editor.dispatchEvent(new Event("paste", { bubbles: true, cancelable: true }));
}

function link(): HTMLAnchorElement {
  const a = document.createElement("a");
  a.href = "https://example.com/";
  a.textContent = "link";
  // Link surfaces consume middle presses (linkGesture.ts internalLinkMouseDown).
  a.addEventListener("mousedown", (event) => { if (event.button === 1) event.preventDefault(); });
  document.body.append(a);
  return a;
}

describe("middle-click paste guard", () => {
  it("does not paste at the textarea caret when a link elsewhere is middle-clicked", () => {
    const editor = document.body.appendChild(document.createElement("textarea"));
    const target = link();
    editor.focus();
    middleClick(target);
    expect(pasteAt(editor)).toBe(false);
    // One-shot: a later paste (Ctrl+V) goes through.
    expect(pasteAt(editor)).toBe(true);
  });

  it("does not paste into the CM6 editor when a link in another block is middle-clicked", () => {
    const editor = document.body.appendChild(document.createElement("div"));
    editor.setAttribute("contenteditable", "true");
    editor.tabIndex = 0;
    const line = editor.appendChild(document.createElement("div"));
    const target = link();
    editor.focus();
    expect(document.activeElement).toBe(editor);
    middleClick(target);
    expect(pasteAt(line)).toBe(false);
  });

  it("does not paste when a link inside the editor consumed the press", () => {
    const editor = document.body.appendChild(document.createElement("div"));
    editor.setAttribute("contenteditable", "true");
    editor.tabIndex = 0;
    const target = link();
    editor.append(target);
    editor.focus();
    middleClick(target);
    expect(pasteAt(editor)).toBe(false);
  });

  it("still pastes when the editor's own text is middle-clicked", () => {
    const editor = document.body.appendChild(document.createElement("textarea"));
    editor.focus();
    middleClick(editor);
    expect(pasteAt(editor)).toBe(true);
  });

  it("leaves a paste that does not follow the release alone", async () => {
    const editor = document.body.appendChild(document.createElement("textarea"));
    const target = link();
    editor.focus();
    middleClick(target);
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(pasteAt(editor)).toBe(true);
  });

  it("ignores left clicks and presses with no editor focused", () => {
    const editor = document.body.appendChild(document.createElement("textarea"));
    const target = link();
    middleClick(target);
    editor.focus();
    expect(pasteAt(editor)).toBe(true);
    target.dispatchEvent(new MouseEvent("mousedown", { button: 0, bubbles: true }));
    target.dispatchEvent(new MouseEvent("mouseup", { button: 0, bubbles: true }));
    expect(pasteAt(editor)).toBe(true);
  });
});
