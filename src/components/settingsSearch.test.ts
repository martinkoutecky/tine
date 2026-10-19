import { afterEach, expect, it } from "vitest";
import { advancedMatch, matchedDescription, rankedSettingMatches, SETTING_SEARCH, settingMatches, settingHighlight } from "./settingsSearch";
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

it("a one-character query matches shown-language names and descriptions, not hidden English (GH #706)", async () => {
  await setActiveLocale("zh-CN");
  // 图谱 and 拼写检查语言 contain 发 in their descriptions: beta.5 dropped them (4 -> 2).
  const fa = labelsFor("发");
  expect(fa).toEqual(expect.arrayContaining(["开发者工具", "发布", "图谱", "拼写检查语言"]));
  expect(fa.indexOf("图谱")).toBeGreaterThan(fa.indexOf("发布"));
  // A single "S" in the Chinese interface matches only text shown in Chinese (CSS, collapsed::),
  // not the English label of almost every setting.
  const s = labelsFor("S");
  expect(s).toContain("编辑全局 CSS");
  expect(s).not.toContain("语言");
  expect(s).not.toContain("发布");
  expect(s).not.toContain("键盘快捷键");
  // Longer words still match the English originals.
  expect(labelsFor("Publish")).toContain("发布");
  await setActiveLocale("en");
  const description = { tab: "graph" as const, label: "Alpha", description: "zebra" };
  expect(settingMatches(description, "z")).toBe(true);
  expect(settingMatches(description, "zebra")).toBe(true);
  expect(settingMatches({ ...description, keywords: "zulu" }, "u")).toBe(true);
  expect(settingMatches({ ...description, aliases: ["quux"] }, "q")).toBe(false);
  expect(settingMatches({ ...description, aliases: ["quux"] }, "quux")).toBe(true);
  expect(settingMatches(description, "a z")).toBe(true);
  expect(settingMatches(description, "a y")).toBe(false);
});

it("shows the matching description under a result only when its name does not match (GH #706)", async () => {
  const entry = { tab: "graph" as const, label: "Alpha", description: "zebra crossing" };
  expect(matchedDescription(entry, "zebra")).toBe("zebra crossing");
  expect(matchedDescription(entry, "alpha")).toBeUndefined();
  expect(matchedDescription(entry, " ")).toBeUndefined();
  await setActiveLocale("zh-CN");
  const graph = SETTING_SEARCH.find(entry => text(entry.label) === "图谱")!;
  expect(matchedDescription(graph, "发")).toContain("发");
  await setActiveLocale("en");
  // Catalog markup is not shown or matched, and a long description is cut around its match.
  const spell = SETTING_SEARCH.find(entry => text(entry.label) === "Spellcheck languages")!;
  expect(matchedDescription(spell, "rescan")).toMatch(/^….*Rescan\.$/);
  expect(matchedDescription(spell, "rescan")).not.toMatch(/<\/?[a-z]+>/);
  expect(matchedDescription(spell, "rescan")!.length).toBeLessThanOrEqual(122);
  expect(settingMatches(spell, "strong")).toBe(false);
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
