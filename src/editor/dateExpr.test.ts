import { describe, it, expect } from "vitest";
import { resolveDateToken, previewDate } from "./dateExpr";

// Fixed reference so relative math is deterministic: 2026-06-16.
const TODAY = new Date(2026, 5, 16);
const iso = (d: Date | null) =>
  d ? `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}` : null;

describe("resolveDateToken", () => {
  it("keywords", () => {
    expect(iso(resolveDateToken("today", TODAY))).toBe("2026-06-16");
    expect(iso(resolveDateToken("now", TODAY))).toBe("2026-06-16");
    expect(iso(resolveDateToken("yesterday", TODAY))).toBe("2026-06-15");
    expect(iso(resolveDateToken("tomorrow", TODAY))).toBe("2026-06-17");
  });

  it("relative durations (mirrors parse_relative)", () => {
    expect(iso(resolveDateToken("-7d", TODAY))).toBe("2026-06-09");
    expect(iso(resolveDateToken("+7d", TODAY))).toBe("2026-06-23");
    expect(iso(resolveDateToken("-30d", TODAY))).toBe("2026-05-17");
    expect(iso(resolveDateToken("-1m", TODAY))).toBe("2026-05-16");
    expect(iso(resolveDateToken("+1y", TODAY))).toBe("2027-06-16");
    expect(iso(resolveDateToken("-1w", TODAY))).toBe("2026-06-09");
  });

  it("clamps month overflow like add_months", () => {
    // Mar 31 minus 1 month → clamp to last day of Feb.
    expect(iso(resolveDateToken("-1m", new Date(2026, 2, 31)))).toBe("2026-02-28");
    // Jan 31 minus 1 month → Dec 31 of the previous year (no clamp needed).
    expect(iso(resolveDateToken("-1m", new Date(2026, 0, 31)))).toBe("2025-12-31");
  });

  it("ISO and journal-title forms", () => {
    expect(iso(resolveDateToken("2026-01-01", TODAY))).toBe("2026-01-01");
    expect(iso(resolveDateToken("Jun 16th, 2026", TODAY))).toBe("2026-06-16");
    expect(iso(resolveDateToken("Jan 1st, 2021", TODAY))).toBe("2021-01-01");
  });

  it("returns null for unresolvable tokens", () => {
    expect(resolveDateToken("Some Page", TODAY)).toBeNull();
    expect(resolveDateToken("", TODAY)).toBeNull();
    expect(resolveDateToken("garbage", TODAY)).toBeNull();
  });

  it("rejects impossible calendar dates instead of rolling them over", () => {
    expect(resolveDateToken("2026-02-31", TODAY)).toBeNull();
    expect(resolveDateToken("2026-13-01", TODAY)).toBeNull();
    expect(resolveDateToken("2026-00-10", TODAY)).toBeNull();
    expect(iso(resolveDateToken("2026-02-28", TODAY))).toBe("2026-02-28"); // valid still works
    // Years 0–99 are literal, not 1900-based.
    expect(resolveDateToken("0099-01-01", TODAY)?.getFullYear()).toBe(99);
  });

  it("previewDate renders or blanks", () => {
    expect(previewDate("-30d", TODAY)).toBe("May 17th, 2026");
    expect(previewDate("Some Page", TODAY)).toBe("");
  });
});
