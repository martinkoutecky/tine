import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { commandDefaults, installKeybindings, paletteCommands, runGlobalCommand } from "./keybindings";
import { resetShortcutOverride, setShortcutOverride, shortcutOverrides } from "./ui";
import { commentMarginHidden } from "./commentMarginPref";
import { authorMarksHidden, setAuthorMarksHidden } from "./authorMarksPref";

const ID = "ui/toggle-author-marks";
const KEY = "logseq-claude.author-marks-hidden";
beforeEach(() => { setAuthorMarksHidden(false); });
afterEach(() => { vi.restoreAllMocks(); resetShortcutOverride(ID); setAuthorMarksHidden(false); });

it("author marks have an independent palette command without a default key", () => {
  expect(commandDefaults().find(c => c.id === ID)?.binding).toBe("");
  expect(paletteCommands().find(c => c.id === ID)?.label).toBe("Show/Hide author marks");
});
it("shown by default; storage refusal leaves the visible preference unchanged", () => {
  expect(authorMarksHidden()).toBe(false);
  vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => { throw new Error("quota"); });
  expect(setAuthorMarksHidden(true)).toBe(false);
  expect(authorMarksHidden()).toBe(false);
});
it("the command persists a separate opt-out and follows a rebound shortcut", () => {
  const commentsWereHidden = commentMarginHidden();
  expect(localStorage.getItem(KEY)).toBeNull();
  runGlobalCommand(ID);
  expect(localStorage.getItem(KEY)).toBe("1");
  expect(commentMarginHidden()).toBe(commentsWereHidden);
  setShortcutOverride(ID, "mod+shift+a");
  const uninstall = installKeybindings(shortcutOverrides());
  try {
    document.body.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "a", ctrlKey: true, shiftKey: true }));
    expect(localStorage.getItem(KEY)).toBeNull();
    expect(commentMarginHidden()).toBe(commentsWereHidden);
  } finally { uninstall(); }
});
