// D-20261119-YL: a configured journal title format that names no single day is
// not used. Titles render in the default; the refused format is no text route
// (model round 3), though an existing journal file it names keeps its day; the
// graph says so once per load (no persisted state).
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { applyConfigDerivedState } from "./graph";
import { setGraphMeta } from "./graphSession";
import { graphTitleRefusal, journalTitle, journalTitleRefusalMessage, journalTitleStatus, parseJournalTitle, setJournalFormats } from "./journal";
import { JOURNAL_TITLE_FORMATS } from "./journalTitleFormats";
import { changeJournalTitleFormat } from "./ui";
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

  it("uses a valid configured format; a refused one reads no text, a file-name format still does", () => {
    setJournalFormats("dd.MM.yyyy", "");
    expect(journalTitle(new Date(2026, 10, 2))).toBe("02.11.2026");
    expect(journalTitleStatus("", "dd.MM.yyyy").refusal).toBeNull();
    setJournalFormats("d.M.yy", "");
    expect(journalTitle(new Date(2026, 0, 5))).toBe("Jan 5th, 2026");
    // A refused title format is no text route: `[[5.1.26]]` is an ordinary page.
    expect(parseJournalTitle("5.1.26")).toBeNull();
    expect(parseJournalTitle("Jan 5th, 2026")?.getFullYear()).toBe(2026);
    // The same pattern as the file-name format is a text route.
    setJournalFormats("MMM do, yyyy", "d.M.yy");
    expect(parseJournalTitle("5.1.26")?.getFullYear()).toBe(2026);
    const status = journalTitleStatus("", "d.M.yy");
    expect(status.refusal).toEqual({ day: { y: 1900, m: 1, d: 1 }, written: "1.1.00", read_as: { y: 2000, m: 1, d: 1 } });
    expect(renderEnglish(journalTitleRefusalMessage("d.M.yy", status)!)).toContain('titles 1900-01-01 "1.1.00", which opens 2000-01-01');
  });
});

describe("the frontend asks with the configured strings as written (review F3)", () => {
  it("passes a whitespace or NBSP file format through, so the frontend and Rust pick one effective title", () => {
    for (const file of [" ", "\t", "\r\n", "\u00a0"]) {
      // Rust keeps a blank-looking filename pattern as written, so the
      // configured `yyyy_dd_MM` has no competing default filename route.
      setJournalFormats("yyyy_dd_MM", file);
      const status = journalTitleStatus(file, "yyyy_dd_MM");
      expect(status, JSON.stringify(file)).toEqual({ effective: "yyyy_dd_MM", refusal: null });
      expect(journalTitle(new Date(2026, 10, 2)), JSON.stringify(file)).toBe("2026_02_11");
      expect(graphTitleRefusal({ journal_page_title_format: "yyyy_dd_MM", journal_file_name_format: file })).toBeNull();
    }
  });
});

describe("the notice", () => {
  const notices = () => toasts().filter((toast) => toast.message.startsWith("Journal titles use"));
  const shown = () => notices().map((toast) => toast.message);

  it("is shown once per graph load while the configured format is refused, and auto-dismisses", () => {
    vi.useFakeTimers();
    applyConfigDerivedState(meta("MM-dd"), null);
    expect(notices().map((toast) => [toast.kind, !!toast.sticky, toast.message])).toEqual([[
      "warn",
      false,
      'Journal titles use "MMM do, yyyy", not "MM-dd" from config.edn: that format writes no year, so one title would name the same date in every year. Your files are unchanged.',
    ]]);
    // A later publication with the same diagnostic says nothing more.
    applyConfigDerivedState({ ...meta("MM-dd"), favorites: ["A"] } as GraphMeta, meta("MM-dd"));
    applyConfigDerivedState({ ...meta("MM-dd"), journal_file_name_format: "yyyy_MM_dd" } as GraphMeta, meta("MM-dd"));
    expect(notices()).toHaveLength(1);
    vi.advanceTimersByTime(3300);
    vi.useRealTimers();
    expect(notices()).toHaveLength(0);
    applyConfigDerivedState({ ...meta("MM-dd"), favorites: ["B"] } as GraphMeta, meta("MM-dd"));
    expect(notices()).toHaveLength(0);
  });

  it("retires when the diagnostic changes or the setting is corrected", () => {
    applyConfigDerivedState(meta("MM-dd"), null);
    applyConfigDerivedState(meta("dd-MM"), meta("MM-dd"));
    expect(shown()).toHaveLength(1);
    expect(shown()[0]).toContain('not "dd-MM"');
    applyConfigDerivedState(meta("yyyy-MM-dd"), meta("dd-MM"));
    expect(notices()).toHaveLength(0);
  });

  it("retires on a graph switch; the next load says its own once", () => {
    applyConfigDerivedState(meta("MM-dd"), null);
    applyConfigDerivedState(meta("dd-MM", "/other"), null);
    expect(shown()).toHaveLength(1);
    expect(shown()[0]).toContain('not "dd-MM"');
    applyConfigDerivedState(meta("yyyy-MM-dd", "/third"), null);
    expect(notices()).toHaveLength(0);
  });

  it("is not shown for a format that names one day", () => {
    applyConfigDerivedState(meta("yyyy-MM-dd"), null);
    applyConfigDerivedState(meta("MMM do, yyyy"), meta("yyyy-MM-dd"));
    expect(notices()).toHaveLength(0);
  });

  it("is never needed for a Settings preset: each names one day, whatever the file-name format", () => {
    // The check is the title's own round trip (model round 3), so the file-name
    // format cannot make a preset refused (`MM-dd-yyyy` files, `dd-MM-yyyy` titles).
    for (const title of JOURNAL_TITLE_FORMATS) {
      for (const file of ["yyyy_MM_dd", "MM-dd-yyyy", "dd-MM-yyyy", title]) {
        expect(journalTitleStatus(file, title), `${title} / ${file}`).toEqual({ effective: title, refusal: null });
      }
    }
  });

  it("is shown after a successful save of a refused format (no config event reaches the writer)", async () => {
    const loaded = await backend().loadGraph("");
    if (loaded.kind === "focused_existing") throw new Error("no graph metadata");
    const graph = { ...loaded.meta, root: "/graph", journal_page_title_format: "MMM do, yyyy", journal_file_name_format: "MM-dd-yyyy" };
    applyConfigDerivedState(graph, null);
    expect(notices()).toHaveLength(0);
    let finish!: (notes: string[]) => void;
    vi.spyOn(backend(), "setJournalTitleFormat").mockImplementationOnce(() => new Promise<string[]>((resolve) => { finish = resolve; }));
    changeJournalTitleFormat("dd-MM");
    expect(notices()).toHaveLength(0);
    finish([]);
    await Promise.resolve();
    await Promise.resolve();
    expect(shown()).toHaveLength(1);
    expect(shown()[0]).toContain('not "dd-MM"');
    vi.restoreAllMocks();
  });
});
