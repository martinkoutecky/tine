import { describe, expect, it } from "vitest";
import { languageDisplayName } from "./spellcheckSettings";

// Master 424d3166f: dictionary names use one consistent "Language (Region)"
// style, not a mix of dialect names ("American English") and region forms.
describe("spellcheck dictionary display names", () => {
  it("names every regional dictionary as Language (Region)", () => {
    Object.defineProperty(globalThis, "navigator", { value: { language: "en" }, configurable: true });
    expect(languageDisplayName("en_US")).toBe("English (United States)");
    expect(languageDisplayName("en_GB")).toBe("English (United Kingdom)");
    expect(languageDisplayName("cs_CZ")).toBe("Czech (Czechia)");
    expect(languageDisplayName("not a locale")).toBe("not a locale");
  });
});
