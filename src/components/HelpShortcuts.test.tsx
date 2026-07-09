import { describe, expect, it } from "vitest";
import { commandDefaults } from "../keybindings";
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
    expect(HELP_ITEMS[0]).toMatchObject({
      label: "Guide",
      detail: "Open the in-app how-to guide",
    });
    expect("run" in HELP_ITEMS[0]).toBe(true);
  });

  it("contains every command id from the keybinding registry", () => {
    const expected = commandDefaults().map((c) => c.id);
    const actual = shortcutPaneCommandIds(buildShortcutPaneData(rows()));

    expect(actual).toHaveLength(expected.length);
    expect(new Set(actual)).toEqual(new Set(expected));
  });
});
