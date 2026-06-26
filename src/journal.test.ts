import { describe, it, expect } from "vitest";
import { isJournalTitle, setJournalTitleFormat } from "./journal";

describe("isJournalTitle (route [[date]] links to journals)", () => {
  it("recognizes the default MMM do, yyyy format", () => {
    setJournalTitleFormat("MMM do, yyyy");
    expect(isJournalTitle("Jun 26th, 2026")).toBe(true);
    expect(isJournalTitle("January 1st, 2020")).toBe(true);
    expect(isJournalTitle("Some Page")).toBe(false);
    expect(isJournalTitle("kitchen-sink")).toBe(false);
    expect(isJournalTitle("")).toBe(false);
  });

  it("recognizes a custom weekday format", () => {
    setJournalTitleFormat("EEEE, dd-MM-yyyy");
    expect(isJournalTitle("Friday, 26-06-2026")).toBe(true);
    expect(isJournalTitle("Thursday, 25-06-2026")).toBe(true);
    expect(isJournalTitle("not a date")).toBe(false);
    // The weekday word is consumed but not validated (mirrors the backend).
    expect(isJournalTitle("Monday, 26-06-2026")).toBe(true);
  });

  it("always accepts ISO + the default as fallbacks, whatever the active format", () => {
    setJournalTitleFormat("EEEE, dd-MM-yyyy");
    expect(isJournalTitle("2026-06-26")).toBe(true);
    expect(isJournalTitle("Jun 26th, 2026")).toBe(true);
  });

  it("rejects out-of-range values and trailing junk", () => {
    setJournalTitleFormat("yyyy-MM-dd");
    expect(isJournalTitle("2026-13-26")).toBe(false); // month 13
    expect(isJournalTitle("2026-06-40")).toBe(false); // day 40
    expect(isJournalTitle("2026-06-26 extra")).toBe(false);
  });
});
