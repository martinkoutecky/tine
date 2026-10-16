import { afterEach, expect, it } from "vitest";
import { advancedMatch, SETTING_SEARCH, settingMatches, settingHighlight } from "./settingsSearch";
import { msg, setActiveLocale } from "../i18n";

afterEach(() => setActiveLocale("en"));

it("highlights every case-insensitive term, merges overlaps and preserves literal labels", () => {
  expect(settingHighlight("Sticky 父 blocks 父", "sticky 父")).toEqual([
    { text: "Sticky", marked: true }, { text: " ", marked: false },
    { text: "父", marked: true }, { text: " blocks ", marked: false }, { text: "父", marked: true },
  ]);
  expect(settingHighlight("Banana", "ana nana")).toEqual([{ text: "B", marked: false }, { text: "anana", marked: true }]);
  expect(settingHighlight("<b>父</b>", "<b>").map(part => part.text).join("")).toBe("<b>父</b>");
  expect(settingHighlight("aliases only", "中文")).toEqual([{ text: "aliases only", marked: false }]);
  expect(settingHighlight("No query", " ")).toEqual([{ text: "No query", marked: false }]);
  expect(settingHighlight("İ Sticky", "sticky")).toEqual([{ text: "İ ", marked: false }, { text: "Sticky", marked: true }]);
});

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
