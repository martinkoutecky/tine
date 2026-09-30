// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from "vitest";
import { caretAtFirstRow, caretAtLastRow, textareaCaretPoints } from "./caretRows";

afterEach(() => vi.restoreAllMocks());

describe("certain textarea row boundaries", () => {
  it("does not consult browser layout at the absolute start and end", () => {
    const textarea = document.createElement("textarea");
    textarea.value = "a long value that may wrap differently across browser hosts";
    const append = vi.spyOn(document.body, "appendChild");

    expect(caretAtFirstRow(textarea, 0)).toBe(true);
    expect(caretAtLastRow(textarea, textarea.value.length)).toBe(true);
    expect(append).not.toHaveBeenCalled();
  });
});

// I-12: all caret measurements use the same style mirror (caretRows.ts).
it.each(["soft", "off"])("mirrors shaping and wrapping for wrap=%s", (wrap) => {
  const ta = document.createElement("textarea");
  ta.wrap = wrap;
  ta.style.cssText = "white-space: pre-wrap; word-break: keep-all; overflow-wrap: anywhere; direction: rtl; text-align: right; font-kerning: none; font-feature-settings: 'liga' 0; font-variation-settings: 'wght' 550";
  document.body.append(ta);
  let mirror: HTMLDivElement | undefined;
  const append = document.body.appendChild.bind(document.body);
  vi.spyOn(document.body, "appendChild").mockImplementation((child) => {
    if (child instanceof HTMLDivElement) mirror = child;
    return append(child);
  });
  textareaCaretPoints(ta);
  expect(mirror!.style.whiteSpace).toBe(wrap === "off" ? "pre" : "pre-wrap");
  expect(mirror!.style.overflowWrap).toBe(wrap === "off" ? "normal" : "anywhere");
  expect(mirror!.style.wordBreak).toBe("keep-all");
  expect(mirror!.style.direction).toBe("rtl");
  expect(mirror!.style.textAlign).toBe("right");
  expect(mirror!.style.fontKerning).toBe("none");
  expect(mirror!.style.fontFeatureSettings).toBe(ta.style.fontFeatureSettings);
  expect(mirror!.style.fontVariationSettings).toBe(ta.style.fontVariationSettings);
  ta.remove();
});
