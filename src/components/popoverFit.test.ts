// GH #619: popover placement arithmetic, and the CSS invariant that stopped the field chooser's rows from
// being squeezed to a few px (jsdom has no layout; the Chromium proof is scripts/shot-query-chooser.mjs).
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { placePopover, placeSheetTop } from "./popoverFit";

describe("placePopover", () => {
  it("stays below when it fits there", () => {
    expect(placePopover(200, 300, 100)).toEqual({ flip: false, maxHeight: null });
  });
  it("flips above when it only fits there", () => {
    expect(placePopover(200, 100, 300)).toEqual({ flip: true, maxHeight: null });
  });
  it("clamps to the larger side and scrolls inside when it fits nowhere", () => {
    expect(placePopover(900, 300, 200)).toEqual({ flip: false, maxHeight: 300 });
    expect(placePopover(900, 200, 330.7)).toEqual({ flip: true, maxHeight: 330 });
  });
  it("never clamps below a usable height", () => {
    expect(placePopover(900, 40, 30).maxHeight).toBe(120);
  });
});

describe("placeSheetTop (GH #619: the sheet stays inside the window)", () => {
  it("hangs under the sentence when it fits there, and before it has been measured", () => {
    expect(placeSheetTop(100, 140, 300, 820)).toBe(140);
    expect(placeSheetTop(700, 740, 0, 820)).toBe(740);
  });
  it("flips above the sentence when it only fits there", () => {
    expect(placeSheetTop(600, 640, 291, 820)).toBe(309);
  });
  it("is pushed up to the bottom edge when it fits on neither side", () => {
    expect(placeSheetTop(200, 240, 700, 820)).toBe(112);
    expect(placeSheetTop(200, 240, 2000, 820)).toBe(8);
  });
});

describe("query sheet list rows never shrink (GH #619)", () => {
  const css = readFileSync(new URL("../styles/query.css", import.meta.url), "utf8");
  const rule = (selector: string) => {
    const at = css.indexOf(`\n${selector} {`);
    expect(at, `${selector} rule exists`).toBeGreaterThanOrEqual(0);
    return css.slice(at, css.indexOf("}", at));
  };
  it("a vocabulary row is a fixed-height, non-shrinking slot", () => {
    const row = rule(".qs-vocab-row");
    expect(row).toMatch(/flex:\s*none/);
    expect(row).toMatch(/height:\s*48px/);
  });
  it("direct children of a popover column do not shrink", () => {
    expect(css).toMatch(/\.qs-menu > \*\s*{[^}]*flex:\s*none/);
    expect(css).toMatch(/\.qs-options > \*\s*{[^}]*flex:\s*none/);
  });
});
