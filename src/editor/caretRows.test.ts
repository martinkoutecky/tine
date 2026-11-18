// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from "vitest";
import { caretAtFirstRow, caretAtLastRow, caretRowTarget, textareaCaretLineRect, textareaCaretPoints } from "./caretRows";

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

describe("caret line rect for an anchored popup (GH #737)", () => {
  it("reports no rect without browser layout so callers fall back to the editor box", () => {
    const textarea = document.createElement("textarea");
    textarea.value = "a:: 1\nb:: 2\nc:: 3";
    document.body.appendChild(textarea);
    expect(textareaCaretLineRect(textarea, 8)).toBeNull();
    textarea.remove();
  });
});

// GH #986: Ctrl+P / Ctrl+N move the caret a visual row where the browser has no binding for them.
describe("caretRowTarget", () => {
  // "abcde" wrapped over rows "abcd" / "e": 4 chars of 10px per row, the wrap offset on the next row.
  const points = Array.from({ length: 6 }, (_, i) => ({ x: (i % 4) * 10, y: i < 4 ? 0 : 20 }));
  it("keeps the column on the neighbouring row", () => {
    expect(caretRowTarget(points, 2, 1)).toBe(5);
    expect(caretRowTarget(points, 5, -1)).toBe(1);
  });
  it("goes to the start or the end when there is no such row, as Up and Down do", () => {
    expect(caretRowTarget(points, 2, -1)).toBe(0);
    expect(caretRowTarget(points, 5, 1)).toBe(5);
  });
});
