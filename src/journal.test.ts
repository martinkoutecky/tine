import { describe, it, expect } from "vitest";
import dateGoldenRaw from "./fixtures/date-golden.json?raw";
import {
  formatJournal,
  isJournalTitle,
  parseJournalWith,
  setJournalTitleFormat,
  type JournalDateParts,
} from "./journal";

type FormatVector = {
  fmt: string;
  date: JournalDateParts;
  title: string;
};

type ParseVector = {
  fmt: string;
  input: string;
  date: JournalDateParts | null;
};

type DateGoldenFixture = {
  _readme: string;
  format: FormatVector[];
  parse: ParseVector[];
};

const dateGolden = JSON.parse(dateGoldenRaw) as DateGoldenFixture;

function localDate({ y, m, d }: JournalDateParts): Date {
  return new Date(y, m - 1, d);
}

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

describe("journal date grammar golden fixture", () => {
  it("formatJournal matches Rust date.rs vectors", () => {
    for (const vector of dateGolden.format) {
      expect(formatJournal(localDate(vector.date), vector.fmt), vector.fmt).toBe(vector.title);
    }
  });

  it("parseJournalWith matches Rust date.rs vectors", () => {
    // Wrong ordinal suffixes are intentional: OG cljs-time's
    // internal/parse.cljs parse-ordinal-suffix accepts any st/nd/rd/th suffix.
    for (const vector of dateGolden.parse) {
      expect(parseJournalWith(vector.input, vector.fmt), `${vector.fmt} <- ${vector.input}`).toEqual(
        vector.date,
      );
    }
  });
});
