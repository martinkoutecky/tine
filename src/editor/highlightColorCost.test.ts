// @vitest-environment jsdom
import { describe, expect, it, vi } from "vitest";

// Merge review of 75e6ab532, finding 3: finding the highlight that owns a selection late in a
// block walks the block's text once, however many highlights come before it. The counter wraps
// the shared byte-to-UTF-16 cursor and adds up the code points each query walks (a query below
// the last answer restarts from 0, as the cursor does).
const walked = vi.hoisted(() => ({ units: 0 }));
vi.mock("../render/utf16Cursor", async (importOriginal) => {
  const real = await importOriginal<typeof import("../render/utf16Cursor")>();
  return {
    utf8ToUtf16Cursor: (text: string) => {
      const cursor = real.utf8ToUtf16Cursor(text);
      let lastByte = 0, lastUnit = 0;
      return (byte: number) => {
        const unit = cursor(byte);
        walked.units += byte < lastByte ? unit : unit - lastUnit;
        lastByte = byte;
        lastUnit = unit;
        return unit;
      };
    },
  };
});

const { setHighlightColor } = await import("./format");

describe("setHighlightColor cost", () => {
  it("walks the text of a block with 2,000 earlier highlights once", () => {
    const text = "x " + Array.from({ length: 2000 }, (_, i) => `==w${i}==`).join(" ") + " last";
    walked.units = 0;
    const edit = setHighlightColor(text, text.length - 4, text.length, "red");
    expect(edit?.text.endsWith('#fdd8d8)">last</mark>')).toBe(true);
    expect(walked.units).toBeLessThanOrEqual(text.length);
  });
});
