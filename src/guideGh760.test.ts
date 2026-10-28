import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

// GH #760: the Guide teaches theme typography with the names Settings shows.
it("the Guide explains Use typography, the Typography selector, the missing-font fallback and the custom.css precedence (GH #760)", () => {
  const extend = readFileSync("crates/tine-core/src/templates/extend-tine.md", "utf8");
  for (const detail of ["**Use typography**", "**Typography** selector", "**Default**", "theme API 0.3", "never ship or download", "first font is missing", "`logseq/custom.css` still overrides"]) {
    expect(extend, detail).toContain(detail);
  }
  expect(extend).not.toContain("downloaded fonts");
  const customize = readFileSync("crates/tine-core/src/templates/customize-look.md", "utf8");
  expect(customize).toContain("**Use typography**");
  const locale = readFileSync("src/i18n/locales/en/themes.ftl", "utf8");
  expect(locale).toContain("themes-use-typography = Use typography");
  expect(locale).toContain("themes-typography = Typography");
});
