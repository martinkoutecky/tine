import { afterEach, expect, it, vi } from "vitest";
import { pdfTextSelection } from "./pdfSelection";

afterEach(() => { vi.restoreAllMocks(); document.body.replaceChildren(); });

// Logseq 0.10.9 pdf.utils/fix-selection-text-breakline, including its
// non-Latin, punctuation and literal-sentinel branches.
it.each([
  ["First line\nSecond line", "First line Second line"],
  ["hyphen-\r\nated", "hyphenated"],
  ["中文\n文本", "中文文本"],
  ["word\n123\n!\n_next", "word123! _next"],
  ["a\r\n\nb", "a b"],
  ["a\n b", "a b"],
  ["a|#|b", "a b"],
])("normalizes selected PDF text %j to %j", (input, expected) => {
  const page = document.createElement("div");
  page.className = "pdf-page";
  page.dataset.page = "1";
  document.body.appendChild(page);
  const selectedText = vi.fn(() => input);
  vi.spyOn(window, "getSelection").mockReturnValue({
    isCollapsed: false,
    toString: selectedText,
    getRangeAt: () => ({
      commonAncestorContainer: page,
      getClientRects: () => [{ left: 10, top: 20, right: 110, bottom: 32, width: 100, height: 12 }],
    }),
  } as unknown as Selection);
  expect(pdfTextSelection(page, undefined, [{ w: 612, h: 792 }, { w: 612, h: 792 }], 1)?.text).toBe(expected);
  expect(selectedText).toHaveBeenCalledOnce();
});
