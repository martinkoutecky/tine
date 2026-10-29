import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";

const css = readFileSync("src/styles/app.css", "utf8");

function rule(selector: string): string {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return new RegExp(`(?:^|\\n)${escaped}\\s*\\{([^}]*)\\}`).exec(css)?.[1] ?? "";
}

// Martin's recording (2026-10-29, development on Linux): moving the pointer off a journal
// title row made the hover-revealed carry buttons (with the star, share and actions
// icons) fade out, come back at full opacity for a frame or two, and vanish again. The
// reveal is an opacity change on elements whose only job is to appear on hover; a
// composited opacity transition is what ends in that flash, so the reveal is a plain
// switch. Colour/background transitions on hover stay.
describe("title-row hover reveals do not animate opacity (carry-button flicker)", () => {
  it.each([".page-carry-actions", ".carry-btn", ".fav-star", ".page-share-btn", ".page-actions-trigger", ".tag-table-toggle"])(
    "%s has no opacity transition",
    (selector) => {
      const body = rule(selector);
      expect(body).not.toBe("");
      const transition = /(?:^|;|\s)transition:\s*([^;]*);/.exec(body)?.[1] ?? "";
      expect(transition).not.toMatch(/\bopacity\b/);
    },
  );

  it.each([".page-carry-actions", ".fav-star", ".page-share-btn", ".page-actions-trigger", ".tag-table-toggle"])(
    "%s is still hidden at rest",
    (selector) => {
      expect(rule(selector)).toMatch(/opacity:\s*0\s*;/);
    },
  );
});
