// GH #845: the comment margin toggles from the palette, a default shortcut and
// any shortcut the user maps it to, like wide and document mode.
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { commandDefaults, installKeybindings, paletteCommands, runGlobalCommand } from "./keybindings";
import { resetShortcutOverride, setShortcutOverride, shortcutOverrides } from "./ui";
import { commentMarginHidden, forceCommentMarginShown } from "./commentMarginPref";

const KEY = "logseq-claude.comment-margin-hidden";
let uninstall: (() => void) | null = null;

function press(init: KeyboardEventInit & { key: string }): void {
  document.body.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init }));
}

beforeEach(() => {
  localStorage.clear();
  forceCommentMarginShown();
});
afterEach(() => {
  uninstall?.();
  uninstall = null;
  resetShortcutOverride("ui/toggle-comment-margin");
  localStorage.clear();
  forceCommentMarginShown();
});

describe("the comment margin command", () => {
  it("is a palette command with a default shortcut Tine and OG leave free", () => {
    const defaults = Object.fromEntries(commandDefaults().map((c) => [c.id, c.binding]));
    expect(defaults["ui/toggle-comment-margin"]).toBe("t m");
    // No other command, in Tine, holds the chord.
    expect(Object.entries(defaults).filter(([, binding]) => binding === "t m").map(([id]) => id)).toEqual(["ui/toggle-comment-margin"]);
    expect(paletteCommands().find((c) => c.id === "ui/toggle-comment-margin")?.label).toBe("Show/Hide comments");
  });

  it("toggles from the palette command and remembers the choice", () => {
    expect(commentMarginHidden()).toBe(false);
    runGlobalCommand("ui/toggle-comment-margin");
    expect(commentMarginHidden()).toBe(true);
    expect(localStorage.getItem(KEY)).toBe("1");
    runGlobalCommand("ui/toggle-comment-margin");
    expect(commentMarginHidden()).toBe(false);
    expect(localStorage.getItem(KEY)).toBeNull();
  });

  it("toggles from its default shortcut, t then m", () => {
    uninstall = installKeybindings();
    press({ key: "t" });
    press({ key: "m" });
    expect(commentMarginHidden()).toBe(true);
    press({ key: "t" });
    press({ key: "m" });
    expect(commentMarginHidden()).toBe(false);
  });

  it("follows a shortcut the user maps it to, and the default stops working", () => {
    setShortcutOverride("ui/toggle-comment-margin", "mod+shift+m");
    expect(shortcutOverrides()["ui/toggle-comment-margin"]).toBe("mod+shift+m");
    uninstall = installKeybindings(shortcutOverrides());
    press({ key: "t" });
    press({ key: "m" });
    expect(commentMarginHidden()).toBe(false);
    press({ key: "m", ctrlKey: true, shiftKey: true });
    expect(commentMarginHidden()).toBe(true);
    press({ key: "m", ctrlKey: true, shiftKey: true });
    expect(commentMarginHidden()).toBe(false);
  });
});
