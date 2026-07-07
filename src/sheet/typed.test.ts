import { describe, expect, it } from "vitest";
import { isPlainDecimalNumber } from "./typed";

describe("typed sheet helpers", () => {
  it("accepts only plain decimal numbers", () => {
    expect(isPlainDecimalNumber("12")).toBe(true);
    expect(isPlainDecimalNumber("-3.5")).toBe(true);
    expect(isPlainDecimalNumber("+0.25")).toBe(true);
    expect(isPlainDecimalNumber("12abc")).toBe(false);
    expect(isPlainDecimalNumber("1e3")).toBe(false);
    expect(isPlainDecimalNumber(".5")).toBe(false);
    expect(isPlainDecimalNumber("")).toBe(false);
  });
});
