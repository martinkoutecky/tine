import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { setToasts, toasts } from "../toasts";
import type { ConflictObject, JournalConflict, SyncConflictDiff } from "../types";

// Master 9dc54e4a7, ported in meaning: a duplicate journal day (a date-stem
// file plus a title-named one) is a conflict-queue object resolved on the day's
// own page. Merge is implicit (row choices, keep-both folds), the per-file
// actions stay, and a cross-format pair says why it offers no rows.

const doc = vi.hoisted(() => ({ applyGraphChange: vi.fn(async () => {}) }));
vi.mock("../document", async (importOriginal) => ({
  ...await importOriginal<typeof import("../document")>(),
  isDirty: () => false,
  isSaving: () => false,
  isConflicted: () => false,
  applyGraphChange: doc.applyGraphChange,
}));
import { PageConflictResolution } from "./ConflictResolution";
import { conflictQueue, setConflictInventory } from "../conflictQueue";
import { setJournalConflicts } from "../ui";
import { ConflictOverview } from "./ConflictOverview";
import type { PaneRouter } from "../router";

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));
const settle = async () => { for (let i = 0; i < 6; i++) await tick(); };
const view = (text: string) => ({ uuid: "", text, child_count: 0 });

const KEEPER = "journals/2026_06_26.md";
const STRAY = "journals/Friday, 26-06-2026.md";
const day: ConflictObject = {
  id: `journal:${KEEPER}`,
  source: "duplicate-journal",
  page_name: "Friday, 26-06-2026",
  page_path: KEEPER,
  kind: "journal",
  sides: [
    { role: "mine", label: "2026_06_26.md", path: KEEPER },
    { role: "theirs", label: "Friday, 26-06-2026.md", path: STRAY },
  ],
  block_conflicts: 2,
};
const inventory: JournalConflict[] = [{
  title: "Friday, 26-06-2026",
  files: [
    { name: "2026_06_26.md", path: KEEPER, preview: "morning notes", canonical: true },
    { name: "Friday, 26-06-2026.md", path: STRAY, preview: "evening notes", canonical: false },
  ],
}];
// Disjoint content: every row one-sided (`added` = keeper-only, `removed` =
// stray-only, as `sync_diff` emits them), where the fold loses nothing.
const disjoint: SyncConflictDiff = {
  base_rev: "keeper-rev",
  conflict_rev: "stray-rev",
  rows: [
    { id: "0", kind: "added", mine: view("morning notes"), theirs: null, children: [] },
    { id: "1", kind: "removed", mine: null, theirs: view("evening notes"), children: [] },
  ],
  mine_pre: null,
  theirs_pre: null,
  pre_differs: false,
  blocks_identical: false,
};

function mount(conflict: ConflictObject) {
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(() => <PageConflictResolution conflict={conflict} />, host);
  return { host, dispose };
}
const button = (root: Element, text: string) =>
  [...root.querySelectorAll("button")].find((b) => b.textContent?.trim().startsWith(text))!;

beforeEach(() => {
  doc.applyGraphChange.mockClear();
  setToasts([]);
  setConflictInventory({ sync_conflicts: [], vcs_markers: [], queue: [day] });
  vi.spyOn(backend(), "listJournalConflicts").mockResolvedValue(inventory);
  vi.spyOn(backend(), "conflictInventory").mockImplementation(() => new Promise(() => {}));
});
afterEach(() => {
  document.body.innerHTML = "";
  setConflictInventory({ sync_conflicts: [], vcs_markers: [], queue: [] });
  setJournalConflicts([]);
  vi.restoreAllMocks();
});

describe("a duplicate journal day resolves at the page (master 9dc54e4a7)", () => {
  it("names both files as the sides and offers row choices: Merge is implicit", async () => {
    const diffCall = vi.spyOn(backend(), "duplicateJournalDiff").mockResolvedValue(disjoint);
    const sync = vi.spyOn(backend(), "syncConflictDiff");
    const { host, dispose } = mount(day);
    await settle();
    expect(diffCall).toHaveBeenCalledWith(KEEPER, STRAY);
    expect(sync).not.toHaveBeenCalled();
    expect(host.querySelector(".page-conflict-title")!.textContent).toBe("This day has more than one file");
    expect(host.querySelector(".page-conflict-side.mine")!.textContent).toBe("2026_06_26.md");
    expect(host.querySelector(".page-conflict-side.theirs")!.textContent).toBe("Friday, 26-06-2026.md");
    expect(host.querySelectorAll("[data-row-id]").length).toBe(2);
    expect(host.querySelector(".page-conflict-rows")!.textContent).toContain("evening notes");
    dispose();
  });

  it("applies the no-loss fold through the guarded duplicate-day command, retires the object and reloads the day", async () => {
    vi.spyOn(backend(), "duplicateJournalDiff").mockResolvedValue(disjoint);
    const resolve = vi.spyOn(backend(), "resolveDuplicateJournalDay").mockResolvedValue();
    const sync = vi.spyOn(backend(), "resolveSyncConflict").mockResolvedValue();
    const { host, dispose } = mount(day);
    await settle();
    button(host, "Apply resolution").click();
    await settle();
    // Each one-sided row keeps its block: both files' content survives.
    expect(resolve).toHaveBeenCalledWith(KEEPER, STRAY, { "0": "mine", "1": "theirs" }, "keeper-rev", "stray-rev", ["replace-page", "delete-page"], "union");
    expect(sync).not.toHaveBeenCalled();
    expect(conflictQueue()).toEqual([]);
    expect(doc.applyGraphChange).toHaveBeenCalledWith({ path: KEEPER, name: "Friday, 26-06-2026", kind: "journal", created: false, removed: false }, true);
    expect(toasts().at(-1)?.message).toContain("Folded the other file");
    dispose();
  });

  it("keeps the per-file actions, and Trash confirms before the recoverable trash", async () => {
    vi.spyOn(backend(), "duplicateJournalDiff").mockResolvedValue(disjoint);
    const confirm = vi.spyOn(backend(), "confirm").mockResolvedValue(true);
    const trash = vi.spyOn(backend(), "trashJournalFile").mockResolvedValue();
    const { host, dispose } = mount(day);
    await settle();
    expect(host.querySelectorAll("[data-journal-conflict]").length).toBe(2);
    const stray = host.querySelector(`[data-journal-conflict="${STRAY}"]`)!;
    for (const label of ["Open", "Rename…", "Trash"]) expect(button(stray, label)).toBeTruthy();
    button(stray, "Trash").click();
    await settle();
    expect(confirm).toHaveBeenCalled();
    expect(trash).toHaveBeenCalledWith("Friday, 26-06-2026.md", "delete-page");
    dispose();
  });

  it("explains a cross-format pair instead of offering choices it cannot apply", async () => {
    vi.spyOn(backend(), "duplicateJournalDiff").mockResolvedValue(null);
    const { host, dispose } = mount(day);
    await settle();
    expect(host.querySelector(".page-conflict-empty")!.textContent).toContain("one is Markdown and the other Org");
    expect(host.querySelectorAll("[data-row-id]").length).toBe(0);
    expect(host.querySelectorAll("[data-journal-conflict]").length).toBe(2);
    dispose();
  });

  it("lists the day in the Conflicts overview under its own group", async () => {
    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(() => <ConflictOverview router={{ openPageTarget: () => {} } as unknown as PaneRouter} />, host);
    await settle();
    const group = host.querySelector('[aria-label="Duplicate journal days"]')!;
    expect(group.textContent).toContain("Friday, 26-06-2026");
    expect(group.textContent).toContain("2 files for one day");
    dispose();
  });
});
