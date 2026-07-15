import { describe, expect, it } from "vitest";
import { shouldOpenBlockContextMenu } from "./Block";

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
  });
});
