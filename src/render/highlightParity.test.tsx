import { describe, expect, it } from "vitest";
import parity from "../../tests/fixtures/highlight-colour-parity.json";
import { HIGHLIGHT_STYLES } from "../blockColors";
import { HIGHLIGHT_MARK_ATTR, rawHtmlPresentation } from "./htmlSanitize";

// The colour row's highlight (UI-HLC-INLINE-COLOURS) widens what raw HTML renders by exactly
// its seven style strings on a plain `<mark>`. Every other input renders exactly as on
// development: the parity table holds development's own output for the state model's
// acceptance tests 1-7 and 9-11 (inputs every past review of this question named).
describe("raw HTML outside the colour row's highlight renders as on development", () => {
  for (const c of parity.cases) {
    it(c.name, () => {
      expect(rawHtmlPresentation(c.html).html).toBe(c.develop);
    });
  }
});

// Acceptance test 8: each of the seven strings, with and without one trailing `;` and
// surrounding whitespace, keeps its one declaration and gets the marker.
describe("the colour row's highlight", () => {
  for (const [name, style] of Object.entries(HIGHLIGHT_STYLES)) {
    it.each([style, `${style};`, ` \t${style}; \n`])(`${name}: %j`, (written) => {
      expect(rawHtmlPresentation(`<mark style="${written}">x</mark>`).html)
        .toBe(`<mark style="${style}" ${HIGHLIGHT_MARK_ATTR}="">x</mark>`);
    });
  }
});
