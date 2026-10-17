import { describe, expect, it } from "vitest";
import { commandDefaults } from "../keybindings";
import { renderEnglish } from "../i18n";
import { HELP_ITEMS, buildShortcutPaneData, shortcutPaneCommandIds, type ShortcutSettingRow } from "./HelpShortcuts";

function rows(): ShortcutSettingRow[] {
  return commandDefaults().map((c) => ({
    ...c,
    effective: c.binding,
    overridden: false,
  }));
}

describe("shortcuts pane data", () => {
  it("puts the Guide action at the top of the help menu", () => {
    expect(renderEnglish(HELP_ITEMS[0].label)).toBe("Guide");
    expect(renderEnglish(HELP_ITEMS[0].detail)).toBe("Open the in-app how-to guide");
    expect("run" in HELP_ITEMS[0]).toBe(true);
  });

  it("contains every command id from the keybinding registry", () => {
    const expected = commandDefaults().map((c) => c.id);
    const actual = shortcutPaneCommandIds(buildShortcutPaneData(rows()));

    expect(actual).toHaveLength(expected.length);
    expect(new Set(actual)).toEqual(new Set(expected));
  });
});

// Margin dialogue (vision §3.7): Ctrl/Cmd+R is a built-in editor chord. It must
// be listed in the shortcuts pane, and no other built-in command may share it
// (the webview's own reload is consumed by the editor command instead).
describe("comment on selection chord", () => {
  it("is listed once under Editor and collides with no other default binding", () => {
    const data = buildShortcutPaneData(rows());
    const editor = data.find((section) => section.scope === "editor")!;
    const row = editor.commands.find((c) => c.id === "editor/comment");
    expect(row?.binding).toBe("mod+r");
    expect(row?.english).toBe("Comment on selected text");
    const holders = commandDefaults().filter((c) => c.binding.split(/\s+/).includes("mod+r") || c.alias === "mod+r");
    expect(holders.map((c) => c.id)).toEqual(["editor/comment"]);
    const palette = data.flatMap((section) => section.commands).find((c) => c.id === "comment/on-selection");
    expect(palette?.english).toBe("Comment on selection");
  });
});
