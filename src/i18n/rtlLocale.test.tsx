// tine#823: right-to-left interface languages: the shipped Persian catalog, and a test-only
// pseudo locale for what no real catalog has to prove (English fallback isolation).
import { afterEach, describe, expect, it } from "vitest";
import { activeLocaleDirection, installPseudoRtlLocale, LOCALES, setActiveLocale, showLocale, t } from "./index";
import { noteLayoutDirAttr, noteLayoutRtl, setNoteLayoutRtl } from "../noteLayoutSetting";
// @ts-expect-error the catalog scripts carry no declarations
import { checkCatalog, localeMeta } from "../../scripts/lib/i18n-catalog.mjs";

const PSEUDO = "xx-Rtl";
installPseudoRtlLocale(PSEUDO, ["settings-ui-logical-outdenting = کلمه { $x }\n"]);
afterEach(() => showLocale("en"));

describe("the interface direction follows the interface language", () => {
  it("only Persian ships right to left; a left-to-right locale leaves the root without `dir`", async () => {
    expect(LOCALES.filter(entry => entry.direction === "rtl").map(entry => entry.tag)).toEqual(["fa"]);
    await setActiveLocale("zh-CN");
    expect(document.documentElement.hasAttribute("dir")).toBe(false);
    expect(activeLocaleDirection()).toBe("ltr");
    expect(noteLayoutDirAttr()).toBeUndefined();
  });

  it("choosing Persian mirrors the interface, defaults the note layout and shows Persian text", async () => {
    await setActiveLocale("fa");
    expect(document.documentElement.getAttribute("dir")).toBe("rtl");
    expect(document.documentElement.lang).toBe("fa");
    expect(noteLayoutDirAttr()).toBe("rtl");
    expect(t("settings-ui-settings")).toBe("تنظیمات");
    await setActiveLocale("en");
    expect(document.documentElement.hasAttribute("dir")).toBe(false);
  });

  it("Persian counts use Persian digits, with arguments isolated", async () => {
    await setActiveLocale("fa");
    expect(t("conflict-resolution-count", { count: 12 })).toContain("۱۲");
    expect(t("settings-ui-couldn-t-save", { value1: "disk full" })).toContain("\u2068disk full\u2069");
  });

  it("a right-to-left locale sets the root and the note layout's default", () => {
    showLocale(PSEUDO);
    expect(document.documentElement.getAttribute("dir")).toBe("rtl");
    expect(noteLayoutRtl()).toBe(true);
    expect(noteLayoutDirAttr()).toBe("rtl");
    showLocale("en");
    expect(document.documentElement.hasAttribute("dir")).toBe(false);
    expect(noteLayoutRtl()).toBe(false);
  });

  it("an explicit note layout outranks the language's default, both ways", () => {
    showLocale(PSEUDO);
    setNoteLayoutRtl(false);
    expect(noteLayoutDirAttr()).toBe("ltr");
    setNoteLayoutRtl(true);
    showLocale("en");
    expect(noteLayoutDirAttr()).toBe("rtl");
    setNoteLayoutRtl(false);
    expect(noteLayoutDirAttr()).toBeUndefined();
  });

  it("isolates arguments only in a right-to-left locale, including English fallback", () => {
    showLocale(PSEUDO);
    expect(t("settings-ui-logical-outdenting" as never, { x: "Tine" } as never)).toContain("⁨Tine⁩");
    showLocale("en");
    expect(t("settings-ui-logical-outdenting")).not.toMatch(/[⁨⁩]/);
  });

  it("the catalog reads the declared direction", () => {
    const meta = (text: string) => localeMeta({ tag: "xx", entries: new Map([["-locale-direction", { entry: { value: { elements: [{ type: "TextElement", value: text }] } } }]]) });
    expect(meta("rtl").direction).toBe("rtl");
    expect(localeMeta({ tag: "xx", entries: new Map() }).direction).toBe("ltr");
    expect(checkCatalog().errors).toEqual([]);
  });
});
