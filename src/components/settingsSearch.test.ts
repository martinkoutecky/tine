import { afterEach, expect, it } from "vitest";
import { advancedMatch, rankedSettingMatches, SETTING_SEARCH, settingMatches, settingHighlight } from "./settingsSearch";
import { msg, render, setActiveLocale, type Msg } from "../i18n";

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

const text = (value: string | Msg) => typeof value === "string" ? value : render(value);
const labelsFor = (query: string) => rankedSettingMatches(SETTING_SEARCH, query).map(entry => text(entry.label));

it("a one-character query matches labels, aliases and keywords but never descriptions (GH #706)", async () => {
  await setActiveLocale("zh-CN");
  // 图谱 and 拼写检查语言 contain 发 only inside their descriptions.
  expect(labelsFor("发").sort()).toEqual([render(msg("settings-search-developer-tools")), render(msg("graph-publish-publish"))].sort());
  expect(labelsFor("发")).toEqual(expect.arrayContaining(["开发者工具", "发布"]));
  expect(labelsFor("发")).not.toContain("图谱");
  expect(labelsFor("发")).not.toContain("拼写检查语言");
  await setActiveLocale("en");
  const description = { tab: "graph" as const, label: "Alpha", description: "zebra" };
  expect(settingMatches(description, "z")).toBe(false);
  expect(settingMatches(description, "zebra")).toBe(true);
  expect(settingMatches({ ...description, aliases: ["zulu"] }, "z")).toBe(true);
  expect(settingMatches({ ...description, keywords: "zulu" }, "z")).toBe(true);
  expect(settingMatches(description, "a")).toBe(true);
  expect(settingMatches(description, "a z")).toBe(false);
});

it("lists label, keyword and alias matches before description-only matches, keeping the table order (GH #706)", () => {
  const all = SETTING_SEARCH.filter(entry => settingMatches(entry, "dev"));
  const ranked = rankedSettingMatches(SETTING_SEARCH, "dev");
  expect(new Set(ranked)).toEqual(new Set(all));
  expect(ranked).toHaveLength(all.length);
  const names = ranked.map(entry => text(entry.label));
  expect(names[0]).toBe("Developer tools");
  expect(names.indexOf("Developer tools")).toBeLessThan(names.indexOf("Query export size limit"));
  const nameHit = (entry: typeof all[number]) => [entry.label, entry.keywords ?? "", ...(entry.aliases ?? [])]
    .some(text => (typeof text === "string" ? text : render(text)).toLowerCase().includes("dev"));
  const flags = ranked.map(nameHit);
  expect(flags.indexOf(false)).toBeGreaterThan(0);
  expect(flags.slice(flags.indexOf(false)).every(flag => !flag)).toBe(true);
  // Stable within a group: both halves keep SETTING_SEARCH order.
  for (const group of [ranked.filter(nameHit), ranked.filter(entry => !nameHit(entry))]) {
    const positions = group.map(entry => SETTING_SEARCH.indexOf(entry));
    expect(positions).toEqual([...positions].sort((a, b) => a - b));
  }
});
