// D-20261119-YL: a configured journal title format that names no single day is
// not used. Titles render in the default; the configured format still feeds
// the one date parser; the graph says so once per load (no persisted state).
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { applyConfigDerivedState } from "./graph";
import { setGraphMeta } from "./graphSession";
import { journalTitle, journalTitleRefusalMessage, journalTitleStatus, parseJournalTitle, setJournalFormats } from "./journal";
import { renderEnglish } from "./i18n";
import { initParser } from "./render/parse";
import { dismissToast, toasts } from "./toasts";
import type { GraphMeta } from "./types";

beforeAll(() => initParser());
afterEach(() => {
  setGraphMeta(null);
  setJournalFormats(null, null);
  for (const toast of toasts()) dismissToast(toast.id);
});

const meta = (title: string, root = "/graph") => ({
  root, preferred_workflow: "now", favorites: [], favorites_page: null,
  journal_page_title_format: title, journal_file_name_format: "yyyy_MM_dd", default_home: null,
} as unknown as GraphMeta);

describe("the effective journal title format", () => {
  it("renders the default under MM-dd, so two years of 11-02 are two titles and [[11-02]] is no day", () => {
    setJournalFormats("MM-dd", "yyyy_MM_dd");
    expect(journalTitle(new Date(2026, 10, 2))).toBe("Nov 2nd, 2026");
    expect(journalTitle(new Date(2027, 10, 2))).toBe("Nov 2nd, 2027");
    expect(parseJournalTitle("11-02")).toBeNull();
    expect(parseJournalTitle("Nov 2nd, 2027")?.getFullYear()).toBe(2027);
    expect(journalTitleStatus("yyyy_MM_dd", "MM-dd")).toEqual({ effective: "MMM do, yyyy", refusal: { missing: "year" } });
  });

  it("uses a valid configured format, and keeps a refused one in the parser", () => {
    setJournalFormats("dd.MM.yyyy", "");
    expect(journalTitle(new Date(2026, 10, 2))).toBe("02.11.2026");
    expect(journalTitleStatus("", "dd.MM.yyyy").refusal).toBeNull();
    setJournalFormats("d.M.yy", "");
    expect(journalTitle(new Date(2026, 0, 5))).toBe("Jan 5th, 2026");
    // The parser is unchanged: a title a file or link already has still reads.
    expect(parseJournalTitle("5.1.26")?.getFullYear()).toBe(2026);
    const status = journalTitleStatus("", "d.M.yy");
    expect(status.refusal).toEqual({ day: { y: 1900, m: 1, d: 1 }, written: "1.1.00", read_as: { y: 2000, m: 1, d: 1 } });
    expect(renderEnglish(journalTitleRefusalMessage("d.M.yy", status)!)).toContain('titles 1900-01-01 "1.1.00", which opens 2000-01-01');
  });
});

describe("the notice", () => {
  const notices = () => toasts().filter((toast) => toast.message.startsWith("Journal titles use"));

  it("is shown once per graph load while the configured format is refused", () => {
    applyConfigDerivedState(meta("MM-dd"), null);
    expect(notices().map((toast) => [toast.kind, toast.message])).toEqual([[
      "warn",
      'Journal titles use "MMM do, yyyy", not "MM-dd" from config.edn: that format writes no year, so one title would name the same date in every year. Your files are unchanged.',
    ]]);
    // A later config publication in the same load says nothing more.
    applyConfigDerivedState(meta("dd-MM"), meta("MM-dd"));
    applyConfigDerivedState({ ...meta("dd-MM"), favorites: ["A"] } as GraphMeta, meta("dd-MM"));
    expect(notices()).toHaveLength(1);
    // The next graph load says it again.
    applyConfigDerivedState(meta("MM-dd", "/other"), null);
    expect(notices()).toHaveLength(2);
  });

  it("is not shown for a format that names one day", () => {
    applyConfigDerivedState(meta("yyyy-MM-dd"), null);
    applyConfigDerivedState(meta("MMM do, yyyy"), meta("yyyy-MM-dd"));
    expect(notices()).toHaveLength(0);
  });
});
