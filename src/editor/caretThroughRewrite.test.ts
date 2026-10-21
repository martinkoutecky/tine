import { describe, expect, it } from "vitest";
import { offsetThroughRewrite } from "./caretThroughRewrite";

describe("offsetThroughRewrite", () => {
  const before = "DOING";
  const after = "DOING\n:LOGBOOK:\nCLOCK: [x]\n:END:";
  it("keeps an offset before an appended span, including the end of the old text", () => {
    expect(offsetThroughRewrite(before, after, 0)).toBe(0);
    expect(offsetThroughRewrite(before, after, 5)).toBe(5);
  });
  it("shifts an offset after an inserted span by the length change", () => {
    expect(offsetThroughRewrite("DOING task", "DOING\n:LOGBOOK:\n:END: task", 8)).toBe(8 + 16);
  });
  it("sends an offset inside a replaced span to the end of the replacement", () => {
    expect(offsetThroughRewrite("abcXYZdef", "abc12def", 4)).toBe(5);
  });
  it("is the identity when nothing changed", () => {
    expect(offsetThroughRewrite("x", "x", 1)).toBe(1);
  });
});
