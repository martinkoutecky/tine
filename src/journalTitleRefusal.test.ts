// D-20261119-YL: a configured journal title format that names no single day is
// not used. Titles render in the default; the refused format is no text route
// (model round 3), though an existing journal file it names keeps its day; the
// graph says so once per load (no persisted state). The decision is native
// (`GraphMeta.journal_title`); these tests read the native answers from the
// golden file Rust pins (`graph_meta_carries_the_native_title_decision`).
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { applyConfigDerivedState } from "./graph";
import { setGraphMeta } from "./graphSession";
import { graphTitleRefusal, journalTitle, journalTitleRefusalMessage, parseJournalTitle, setJournalFormats } from "./journal";
import { changeJournalTitleFormat } from "./ui";
import { renderEnglish } from "./i18n";
import { initParser } from "./render/parse";
import { dismissToast, toasts } from "./toasts";
import type { GraphMeta, JournalTitleSaved, JournalTitleStatus } from "./types";
import goldenRaw from "../tests/fixtures/journal-title-status-golden.json?raw";

const GOLDEN = JSON.parse(goldenRaw) as Record<string, JournalTitleStatus>;
const status = (configured: string): JournalTitleStatus => {
  const answer = GOLDEN[configured];
  if (!answer) throw new Error(`no native answer for ${configured} in the golden file`);
  return answer;
};

beforeAll(() => initParser());
afterEach(() => {
  setGraphMeta(null);
  setJournalFormats(null, null);
  for (const toast of toasts()) dismissToast(toast.id);
});

const meta = (title: string, root = "/graph") => ({
  root, preferred_workflow: "now", favorites: [], favorites_page: null,
  journal_page_title_format: title, journal_title: status(title), journal_file_name_format: "yyyy_MM_dd", default_home: null,
} as unknown as GraphMeta);

describe("the effective journal title format", () => {
  it("renders the default under MM-dd, so two years of 11-02 are two titles and [[11-02]] is no day", () => {
    applyConfigDerivedState(meta("MM-dd"), null);
    expect(journalTitle(new Date(2026, 10, 2))).toBe("Nov 2nd, 2026");
    expect(journalTitle(new Date(2027, 10, 2))).toBe("Nov 2nd, 2027");
    expect(parseJournalTitle("11-02")).toBeNull();
    expect(parseJournalTitle("Nov 2nd, 2027")?.getFullYear()).toBe(2027);
  });

  it("accepts a valid format and omits refused title and filename text routes", () => {
    applyConfigDerivedState({ ...meta("dd.MM.yyyy"), journal_file_name_format: "" } as GraphMeta, null);
    expect(journalTitle(new Date(2026, 10, 2))).toBe("02.11.2026");
    applyConfigDerivedState({ ...meta("d.M.yy"), journal_file_name_format: "" } as GraphMeta, null);
    expect(journalTitle(new Date(2026, 0, 5))).toBe("Jan 5th, 2026");
    // A refused title format is no text route: `[[5.1.26]]` is an ordinary page.
    expect(parseJournalTitle("5.1.26")).toBeNull();
    expect(parseJournalTitle("Jan 5th, 2026")?.getFullYear()).toBe(2026);
    const defaults = meta("MMM do, yyyy");
    applyConfigDerivedState({ ...defaults, journal_file_name_format: "d.M.yy",
      journal_title: { ...defaults.journal_title, file_text_route: false } }, null);
    expect(parseJournalTitle("5.1.26")).toBeNull();
    expect(renderEnglish(journalTitleRefusalMessage("d.M.yy", status("d.M.yy"))!)).toContain('titles 1900-01-01 "1.1.00", which opens 2000-01-01');
  });
});

describe("the frontend reads with the file-name format as written (review F3)", () => {
  it("passes a whitespace or NBSP file format through to the text routes", () => {
    for (const file of [" ", "\t", "\r\n", "\u00a0"]) {
      // Rust keeps a blank-looking filename pattern as written, so the
      // `yyyy_dd_MM` title has no competing default filename route.
      setJournalFormats(status("yyyy_dd_MM").effective, file);
      expect(journalTitle(new Date(2026, 10, 2)), JSON.stringify(file)).toBe("2026_02_11");
      expect(parseJournalTitle("2026_02_11")?.getMonth(), JSON.stringify(file)).toBe(10);
      expect(graphTitleRefusal({ journal_page_title_format: "yyyy_dd_MM", journal_title: status("yyyy_dd_MM") })).toBeNull();
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

  it("warns at most once per load: a changed diagnostic retires the shown notice and says nothing new", () => {
    applyConfigDerivedState(meta("MM-dd"), null);
    expect(shown()).toHaveLength(1);
    applyConfigDerivedState(meta("dd-MM"), meta("MM-dd"));
    expect(notices()).toHaveLength(0);
    applyConfigDerivedState(meta("yyyy-MM-dd"), meta("dd-MM"));
    expect(notices()).toHaveLength(0);
  });

  it("warns at most once per load when the setting is corrected and refused again", () => {
    applyConfigDerivedState(meta("MM-dd"), null);
    expect(shown()).toHaveLength(1);
    applyConfigDerivedState(meta("yyyy-MM-dd"), meta("MM-dd"));
    expect(notices()).toHaveLength(0);
    applyConfigDerivedState(meta("MM-dd"), meta("yyyy-MM-dd"));
    expect(notices()).toHaveLength(0);
    // Settings keeps saying why while it is refused.
    expect(graphTitleRefusal(meta("MM-dd"))).not.toBeNull();
    // The next load warns again.
    applyConfigDerivedState(meta("MM-dd"), null);
    expect(shown()).toHaveLength(1);
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

  it("is shown after a successful save of a refused format (no config event reaches the writer)", async () => {
    const loaded = await backend().loadGraph("");
    if (loaded.kind === "focused_existing") throw new Error("no graph metadata");
    const graph = { ...loaded.meta, root: "/graph", journal_page_title_format: "MMM do, yyyy", journal_title: status("MMM do, yyyy"), journal_file_name_format: "MM-dd-yyyy" };
    applyConfigDerivedState(graph, null);
    expect(notices()).toHaveLength(0);
    let finish!: (saved: JournalTitleSaved) => void;
    vi.spyOn(backend(), "setJournalTitleFormat").mockImplementationOnce(() => new Promise<JournalTitleSaved>((resolve) => { finish = resolve; }));
    changeJournalTitleFormat("dd-MM");
    expect(notices()).toHaveLength(0);
    finish({ journal_config_diagnostics: [], journal_title: status("dd-MM") });
    await Promise.resolve();
    await Promise.resolve();
    expect(shown()).toHaveLength(1);
    expect(shown()[0]).toContain('not "dd-MM"');
    // The native decision replaced the optimistic one: titles use the default.
    expect(journalTitle(new Date(2026, 10, 2))).toBe("Nov 2nd, 2026");
    expect(parseJournalTitle("02-11")).toBeNull();
    vi.restoreAllMocks();
  });
});
