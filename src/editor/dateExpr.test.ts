import { describe, it, expect } from "vitest";
import { resolveDateToken, previewDate } from "./dateExpr";
import { localCalendarDate } from "../journal";
import golden from "../../tests/fixtures/i12-date-token-golden.json";

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

// I-12: `resolve_date_token` in Rust (advanced_patterns.rs) is the one grammar;
// this preview reads the SAME golden file as its Rust test, so the two cannot
// drift (OG-C5 mess dateExpr twin: `-7D`, `2026_01_05`, the 10,000-year bound
// and the year-0 leap clamp all used to disagree).
describe("resolveDateToken agrees with the native date-token golden", () => {
  it("resolves every recorded case to the recorded ordinal", () => {
    const mismatches: string[] = [];
    for (const c of golden.cases) {
      const today = localCalendarDate(Math.trunc(c.today / 10000), Math.trunc((c.today % 10000) / 100) - 1, c.today % 100)!;
      const got = resolveDateToken(c.token, today);
      const ordinal = got ? got.getFullYear() * 10000 + (got.getMonth() + 1) * 100 + got.getDate() : null;
      if (ordinal !== c.ordinal) mismatches.push(`today ${c.today} token ${JSON.stringify(c.token)}: golden ${c.ordinal}, preview ${ordinal}`);
    }
    expect(mismatches).toEqual([]);
  });
});
