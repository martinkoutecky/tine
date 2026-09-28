import { describe, it, expect } from "vitest";
import dateGoldenRaw from "./fixtures/date-golden.json?raw";
import journalTitleGoldenRaw from "../tests/fixtures/i12-journal-title-golden.json?raw";
import { JOURNAL_TITLE_FORMATS } from "./journalTitleFormats";
import {
  formatJournal,
  isJournalTitle,
  parseJournalWith,
  setJournalTitleFormat,
  parseJournalTitle,
  localDayKey,
  localDateFromDayKey,
  localDayRolloverDelay,
  type JournalDateParts,
} from "./journal";

it("inverts a valid journal day key in years below 100", () => {
  const day = localDateFromDayKey(10102);
  expect([day.getFullYear(), day.getMonth() + 1, day.getDate()]).toEqual([1, 1, 2]);
});

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
const journalTitleGolden = JSON.parse(journalTitleGoldenRaw) as { cases: { name: string; expected: boolean }[] };

function localDate({ y, m, d }: JournalDateParts): Date {
  return new Date(y, m - 1, d);
}

describe("isJournalTitle (route [[date]] links to journals)", () => {
  it("matches the Rust journal title fixture", () => {
    setJournalTitleFormat(null);
    for (const { name, expected } of journalTitleGolden.cases) {
      expect(isJournalTitle(name), name).toBe(expected);
    }
  });
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
    expect(isJournalTitle("Feb 31st, 2026")).toBe(false);
    expect(isJournalTitle("2026_06_26")).toBe(true);
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

it("parses the containing journal's date across configured and fallback titles", () => {
  setJournalTitleFormat("dd.MM.yyyy");
  expect(localDayKey(parseJournalTitle("21.07.2026")!)).toBe(20260721);
  expect(localDayKey(parseJournalTitle("2026-07-21")!)).toBe(20260721);
  expect(parseJournalTitle("ordinary page")).toBeNull();
  setJournalTitleFormat(null);
});

it("computes the next local calendar rollover without 24-hour arithmetic", () => {
  expect(localDayRolloverDelay(new Date(2026, 6, 21, 23, 59, 59, 900))).toBe(125);
});

it("offers all three dotted Logseq journal title formats", () => {
  for (const pattern of ["E, dd.MM.yyyy", "EEE, dd.MM.yyyy", "EEEE, dd.MM.yyyy"]) {
    expect(JOURNAL_TITLE_FORMATS).toContain(pattern);
    const title = formatJournal(new Date(2026, 6, 21), pattern);
    expect(parseJournalWith(title, pattern)).toEqual({ y: 2026, m: 7, d: 21 });
  }
});
