import { describe, expect, it } from "vitest";
import { daysInCalendarMonth, utcCalendarMillis } from "../sheet/typed";
import { readFileSync } from "node:fs";

describe("DatePicker grid uses the shared calendar (D18 neighbor)", () => {
  it("year 0099 has its own weekday and leap rule, not 1999's", () => {
    // 0099-01-01 is a Thursday (proleptic Gregorian); 1999-01-01 is a Friday.
    expect(new Date(utcCalendarMillis(99, 0, 1)).getUTCDay()).toBe(4);
    expect(daysInCalendarMonth(96, 1)).toBe(29); // 0096 leap; 1996 too, so also check 0000
    expect(daysInCalendarMonth(0, 1)).toBe(29); // 0000 leap; 1900 is not
  });
  it("DatePicker grid does not construct local Dates from the view year", () => {
    const src = readFileSync(new URL("./DatePicker.tsx", import.meta.url), "utf8");
    expect(src).not.toMatch(/new Date\(y, m/);
  });
});
