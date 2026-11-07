// @vitest-environment jsdom
import { afterEach, expect, it } from "vitest";
import { editorCommandFor, installKeybindings, matchesCommand, paletteCommands, shortcutBindingsConflict } from "./keybindings";
import { closeSwitcher, switcherOpen } from "./ui";
import { bindingAlternatives, shortcutValue } from "./shortcutBindings";
import { shortcutConflicts, type ShortcutSettingRow } from "./components/HelpShortcuts";

let dispose = () => {};
afterEach(() => { dispose(); closeSwitcher(); });
const key = (value: string, ctrlKey = false) => new KeyboardEvent("keydown", { key: value, ctrlKey, bubbles: true, cancelable: true });

it("dispatches every alternative, including a sequence, without triggering twice", () => {
  dispose = installKeybindings({ "go/search": ["mod+k", "mod+j", "g q"] });
  for (const chord of [key("k", true), key("j", true)]) {
    window.dispatchEvent(chord);
    expect(switcherOpen()).toBe(true);
    expect(chord.defaultPrevented).toBe(true);
    closeSwitcher();
  }
  window.dispatchEvent(key("g"));
  expect(switcherOpen()).toBe(false);
  window.dispatchEvent(key("q"));
  expect(switcherOpen()).toBe(true);
});

it("matches each editor and selection binding, while empty vectors disable", () => {
  dispose = installKeybindings({ "editor/insert-link": ["mod+k", "mod+j"], "editor/indent": [], "editor/quick-capture-file": ["mod+shift+enter", "f8"] });
  for (const letter of ["k", "j"]) {
    expect(matchesCommand(key(letter, true), "editor/insert-link")).toBe(true);
    expect(editorCommandFor(key(letter, true))).toBe("editor/insert-link");
  }
  expect(matchesCommand(key("Tab"), "editor/indent")).toBe(false);
  expect(matchesCommand(key("F8"), "editor/quick-capture-file")).toBe(true);
});

it("retains legacy strings and false, and displays all palette alternatives", () => {
  dispose = installKeybindings({ "go/search": "mod+j", "ui/open-settings": ["mod+8", "mod+9"], "editor/redo": "false" });
  expect(matchesCommand(key("j", true), "go/search")).toBe(true);
  expect(matchesCommand(key("k", true), "go/search")).toBe(false);
  expect(paletteCommands().find((c) => c.id === "ui/open-settings")?.binding).toBe("mod+8, mod+9");
  expect(paletteCommands().find((c) => c.id === "editor/redo")?.binding).toBe("");
  expect(bindingAlternatives("false")).toEqual([]);
  expect(shortcutValue(["mod+k"])).toBe("mod+k");
  expect(shortcutValue([])).toBe("false");
});

it("detects normalized collisions and sequence prefixes across all alternatives", () => {
  expect(shortcutBindingsConflict(["mod+k", "ctrl+shift+j"], ["mod+9", "shift+mod+j"])).toBe(true);
  expect(shortcutBindingsConflict(["x y", "g q"], ["g q a"])).toBe(true);
  expect(shortcutBindingsConflict([], "mod+k")).toBe(false);
  expect(shortcutBindingsConflict("false", ["mod+k"])).toBe(false);
  expect(shortcutBindingsConflict(["mod+k", "mod+j"], ["alt+k", "mod+shift+j"])).toBe(false);
  const row: ShortcutSettingRow = { id: "a", label: "A", binding: "", effective: ["mod+k", "mod+j"], overridden: true, scope: "editor" };
  const other = { ...row, id: "b", effective: ["mod+1", "mod+j"] };
  expect(shortcutConflicts(row, [row, other]).map((item) => item.id)).toEqual(["b"]);
  expect(shortcutConflicts(row, [row, { ...other, scope: "select" }])).toHaveLength(1);
  expect(shortcutConflicts(row, [row, { ...other, scope: "global" }])).toHaveLength(1);
});
