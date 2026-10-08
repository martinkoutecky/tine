import { describe, expect, it } from "vitest";
import { shouldOpenBlockContextMenu, shouldOpenTextContextMenu } from "../contextMenuPolicy";

describe("block context-menu targeting (GH #162)", () => {
  it("leaves native selection gestures alone and keeps explicit block-menu targets", () => {
    const row = document.createElement("div");
    const content = document.createElement("div");
    const editor = document.createElement("textarea");
    const bullet = document.createElement("span");
    row.append(content, editor, bullet);
    bullet.className = "bullet-container";

    expect(shouldOpenBlockContextMenu(editor, true)).toBe(false);
    expect(shouldOpenBlockContextMenu(content, true)).toBe(false);
    expect(shouldOpenBlockContextMenu(bullet, true)).toBe(true);
    expect(shouldOpenBlockContextMenu(editor, false)).toBe(false);
    expect(shouldOpenBlockContextMenu(content, false)).toBe(true);
    expect(shouldOpenTextContextMenu(content, true)).toBe(false);
    expect(shouldOpenTextContextMenu(content, false)).toBe(true);
    expect(shouldOpenTextContextMenu(editor, false)).toBe(false);
  });
  it("code caret opens desktop block actions while selections and mobile gestures stay native", () => {
    const editor = document.createElement("textarea");
    editor.className = "block-editor code-edit";
    editor.value = "payload";
    editor.setSelectionRange(0, 0);
    expect(shouldOpenBlockContextMenu(editor, false)).toBe(true);
    expect(shouldOpenBlockContextMenu(editor, true)).toBe(false);
    editor.setSelectionRange(0, 4);
    expect(shouldOpenBlockContextMenu(editor, false)).toBe(false);
    expect(shouldOpenTextContextMenu(editor, false)).toBe(false);
  });

});
