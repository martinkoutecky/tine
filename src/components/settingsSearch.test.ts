import { expect, it } from "vitest";
import { advancedMatch, SETTING_SEARCH, settingMatches } from "./settingsSearch";

it("finds Pinyin and reveals Editor's advanced settings", () => {
  expect(SETTING_SEARCH.filter(entry => settingMatches(entry, "pinyin")).map(entry => entry.tab)).toContain("editor");
  expect(advancedMatch("editor", "pinyin")).toBe(true);
});
