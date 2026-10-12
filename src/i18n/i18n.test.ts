import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterAll, describe, expect, it } from "vitest";
// @ts-expect-error: plain ESM build script, no types.
import { checkCatalog, sourceHash, readLocale } from "../../scripts/lib/i18n-catalog.mjs";
// @ts-expect-error: plain ESM build script, no types.
import { generate } from "../../scripts/i18n-gen.mjs";
import { activeLocale, msg, negotiateLocale, rawText, render, renderEnglish, setActiveLocale, t, textKey } from "./index";

describe("the catalog", () => {
  it("has no errors, and its generated derivatives are current", () => {
    const { errors } = checkCatalog();
    expect(errors).toEqual([]);
    for (const [file, text] of generate() as [string, string][]) {
      expect(fs.readFileSync(file, "utf8"), `${file} is stale: run node scripts/i18n-gen.mjs`).toBe(text);
    }
  });
});

describe("catalog rules", () => {
  const roots: string[] = [];
  afterAll(() => roots.forEach((root) => fs.rmSync(root, { recursive: true, force: true })));
  function catalog(files: Record<string, string>) {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "tine-i18n-"));
    roots.push(root);
    const base: Record<string, string> = {
      "en/meta.ftl": "-locale-name = English\n",
      "xx/meta.ftl": "-locale-name = Xx\n",
    };
    for (const [file, text] of Object.entries({ ...base, ...files })) {
      fs.mkdirSync(path.dirname(path.join(root, file)), { recursive: true });
      fs.writeFileSync(path.join(root, file), text);
    }
    return { root, ...checkCatalog(root) };
  }

  it("accepts a translation that drops a variable English passes", () => {
    const { errors } = catalog({
      "en/a.ftl": "items = { $count } items in { $page }\n",
      "xx/a.ftl": "items = { $count } things\n",
    });
    expect(errors).toEqual([]);
  });

  it("rejects a translation that uses a variable English does not pass", () => {
    const { errors } = catalog({
      "en/a.ftl": "items = { $count } items\n",
      "xx/a.ftl": "items = { $count } in { $page }\n",
    });
    expect(errors.join("\n")).toContain("$page");
  });

  it("rejects orphans, unknown references, cycles, unsupported functions and junk", () => {
    const { errors } = catalog({
      "en/a.ftl": "a = { b }\nb = { a }\nc = { PLATFORM() }\nd = { missing }\ne = {\n",
      "xx/a.ftl": "gone = old\n",
    });
    const text = errors.join("\n");
    expect(text).toContain("reference cycle");
    expect(text).toContain("PLATFORM");
    expect(text).toContain("unknown `missing`");
    expect(text).toContain("unparsable");
    expect(text).toContain("`gone` is not in English");
  });

  it("requires shell messages to be literal text", () => {
    const { errors } = catalog({
      "en/shell.ftl": "shell-ok = Open\nshell-var = Open { $name }\nshell-term = Open { -brand }\n",
      "en/terms.ftl": "-brand = Tine\n",
    });
    expect(errors.filter((e: string) => e.includes("shell.ftl"))).toHaveLength(2);
  });

  it("calls a translation stale until accepted, and again when English or a referenced term changes", () => {
    const en = "-brand = Tine\nwelcome = Welcome to { -brand }\n";
    const first = catalog({ "en/a.ftl": en, "xx/a.ftl": "-brand = Tine\nwelcome = Hi\n" });
    expect(first.report.xx.stale).toContain("welcome");
    const english = readLocale("en", first.root).entries;
    const hash = sourceHash("welcome", english);
    const accepted = catalog({
      "en/a.ftl": en,
      "xx/a.ftl": "-brand = Tine\nwelcome = Hi\n",
      "xx/.source-hashes.json": JSON.stringify({ welcome: hash, "-brand": sourceHash("-brand", english) }),
    });
    expect(accepted.report.xx.stale).toEqual([]);
    const termChanged = catalog({
      "en/a.ftl": "-brand = Tine Nightly\nwelcome = Welcome to { -brand }\n",
      "xx/a.ftl": "-brand = Tine\nwelcome = Hi\n",
      "xx/.source-hashes.json": JSON.stringify({ welcome: hash }),
    });
    expect(termChanged.report.xx.stale).toContain("welcome");
  });

  it("lists English messages a locale lacks as missing", () => {
    const { report } = catalog({ "en/a.ftl": "one = One\ntwo = Two\n", "xx/a.ftl": "one = Un\n" });
    expect(report.xx.missing).toEqual(["two"]);
  });
});

describe("locale negotiation", () => {
  it("honours an explicit shipped preference", () => {
    expect(negotiateLocale("zh-CN", ["en-US"])).toBe("zh-CN");
  });
  it("maps the system's Chinese tags to Simplified Chinese and everything unknown to English", () => {
    expect(negotiateLocale("", ["zh-Hans-CN"])).toBe("zh-CN");
    expect(negotiateLocale("", ["zh"])).toBe("zh-CN");
    expect(negotiateLocale("", ["cs-CZ", "en-GB"])).toBe("en");
    expect(negotiateLocale("", ["fr-FR"])).toBe("en");
  });
  it("ignores a stored preference for a locale this build does not ship", () => {
    expect(negotiateLocale("xx-YY", ["zh-CN"])).toBe("zh-CN");
  });
});

describe("formatting", () => {
  it("formats English, then the active locale, falling back to English per message", async () => {
    expect(t("settings-language-label")).toBe("Language");
    expect(t("settings-language-system", { language: "English" })).toBe("System default (English)");
    await setActiveLocale("zh-CN");
    expect(activeLocale()).toBe("zh-CN");
    expect(t("settings-language-label")).toBe("语言");
    // Terms resolve inside the message; no isolation marks leak into the text.
    expect(t("settings-language-description")).toMatch(/^Tine 的菜单/);
    await setActiveLocale("en");
  });

  it("renders a stored Msg in the active locale and keys it by id and English", async () => {
    const stored = msg("settings-language-system", { language: "简体中文" });
    await setActiveLocale("zh-CN");
    expect(render(stored)).toBe("跟随系统（简体中文）");
    expect(renderEnglish(stored)).toBe("System default (简体中文)");
    expect(textKey(stored)).toBe("settings-language-system:System default (简体中文)");
    await setActiveLocale("en");
    expect(render(stored)).toBe("System default (简体中文)");
    expect(render(rawText("from a plugin"))).toBe("from a plugin");
  });
});
