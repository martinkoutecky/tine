import { afterEach, expect, it } from "vitest";
import { advancedMatch, SETTING_SEARCH, settingMatches } from "./settingsSearch";
import { msg, setActiveLocale } from "../i18n";

afterEach(() => setActiveLocale("en"));

it("finds Pinyin and reveals Editor's advanced settings", () => {
  expect(SETTING_SEARCH.filter(entry => settingMatches(entry, "pinyin")).map(entry => entry.tab)).toContain("editor");
  expect(advancedMatch("editor", "pinyin")).toBe(true);
});

it("matches translated and original English vocabulary after a language change", async () => {
  const entry = { tab: "appearance" as const, label: msg("settings-language-label"), description: msg("settings-language-description") };
  await setActiveLocale("zh-CN");
  expect(settingMatches(entry, "语言")).toBe(true);
  expect(settingMatches(entry, "Language")).toBe(true);
  expect(settingMatches(entry, "settings")).toBe(true);
});
