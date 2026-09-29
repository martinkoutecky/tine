import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { setToasts, toasts } from "../toasts";
import { OUTLINE_MAX_DEPTH } from "../editor/outline";
import type { ConflictInventory, ConflictObject, DiffRow, SyncConflictDiff } from "../types";

// og 8c: the in-page resolver for the two artifact sources (a sync tool's
// conflict copy, a VCS merge's markers). All content is synthetic.

const doc = vi.hoisted(() => ({
  dirty: false,
  conflicted: false,
  applyGraphChange: vi.fn(async () => {}),
  flushPage: vi.fn(async () => true),
}));
vi.mock("../document", async (importOriginal) => ({
  ...await importOriginal<typeof import("../document")>(),
  isDirty: () => doc.dirty,
  isSaving: () => false,
  isConflicted: () => doc.conflicted,
  applyGraphChange: doc.applyGraphChange,
  flushPage: doc.flushPage,
}));
import { PageConflictResolution } from "./ConflictResolution";
import { conflictQueue, setConflictInventory } from "../conflictQueue";

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));
const settle = async () => { for (let i = 0; i < 6; i++) await tick(); };
const view = (text: string) => ({ uuid: "", text, child_count: 0 });

const markerConflict: ConflictObject = {
  id: "markers:pages/Merged.md",
  source: "vcs-markers",
  page_name: "Merged",
  page_path: "pages/Merged.md",
  kind: "page",
  sides: [
    { role: "mine", label: "HEAD" },
    { role: "theirs", label: "feature" },
    { role: "base", label: "merged common ancestors" },
  ],
  block_conflicts: 2,
  markers: ["<<<<<<<", "=======", ">>>>>>>"],
};

const copyConflict: ConflictObject = {
  id: "copy:pages/Plan.sync-conflict-20260705-141233-ABCDEFG.md",
  source: "sync-copy",
  page_name: "Plan",
  page_path: "pages/Plan.md",
  kind: "page",
  sides: [
    { role: "mine", label: "This device", path: "pages/Plan.md" },
    { role: "theirs", label: "sync-conflict-20260705-141233-ABCDEFG", path: "pages/Plan.sync-conflict-20260705-141233-ABCDEFG.md" },
  ],
  block_conflicts: 1,
};

function diff(rows: DiffRow[], rev = "rev-1"): SyncConflictDiff {
  return { base_rev: rev, conflict_rev: "copy-rev", rows, mine_pre: null, theirs_pre: null, pre_differs: false, blocks_identical: false };
}

const threeWayRows: DiffRow[] = [
  { id: "0", kind: "modified", mine: view("TODO ship Friday"), theirs: view("TODO ship Thursday"), children: [], verdict: "theirs-only", suggestion: "theirs" },
  { id: "1", kind: "modified", mine: view("A mine"), theirs: view("A theirs"), children: [], verdict: "both-changed" },
];

function inventoryWith(conflict: ConflictObject): ConflictInventory {
  return {
    sync_conflicts: conflict.source === "sync-copy"
      ? [{ path: conflict.sides[1].path!, base_name: conflict.page_name, base_path: conflict.page_path, kind: "page", tag: conflict.sides[1].label, preview: "" }]
      : [],
    vcs_markers: conflict.source === "vcs-markers" ? [{ path: conflict.page_path, name: conflict.page_name, kind: "page", markers: conflict.markers ?? [] }] : [],
    queue: [conflict],
  };
}

function mount(conflict: ConflictObject) {
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(() => <PageConflictResolution conflict={conflict} />, host);
  return { host, dispose };
}

const button = (host: HTMLElement, text: string) =>
  [...host.querySelectorAll("button")].find((b) => b.textContent?.includes(text))!;

beforeEach(() => {
  doc.dirty = false;
  doc.conflicted = false;
  doc.applyGraphChange.mockClear();
  doc.flushPage.mockClear();
  setToasts([]);
});
afterEach(() => {
  document.body.innerHTML = "";
  setConflictInventory({ sync_conflicts: [], vcs_markers: [], queue: [] });
  vi.restoreAllMocks();
});

describe("in-page conflict resolution", () => {
  it("names the sides the marker file itself named and says why saves are refused", async () => {
    vi.spyOn(backend(), "vcsMarkerConflictDiff").mockResolvedValue({ mine_label: "HEAD", theirs_label: "feature", regions: 2, diff: diff(threeWayRows) });
    const { host, dispose } = mount(markerConflict);
    await settle();
    expect(host.querySelector(".page-conflict-side.mine")!.textContent).toBe("HEAD");
    expect(host.querySelector(".page-conflict-side.theirs")!.textContent).toBe("feature");
    expect(host.querySelector(".page-conflict-side.base")!.textContent).toContain("merged common ancestors");
    expect(host.querySelector(".page-conflict-refusal")!.textContent).toContain("refuses to save");
    dispose();
  });

  it("pre-selects the suggested side, and keeps BOTH where no side is suggested", async () => {
    vi.spyOn(backend(), "vcsMarkerConflictDiff").mockResolvedValue({ mine_label: "HEAD", theirs_label: "feature", regions: 2, diff: diff(threeWayRows) });
    const { host, dispose } = mount(markerConflict);
    await settle();
    const active = (id: string) => host.querySelector(`[data-row-id="${id}"] .sync-merge-seg.active`)!.getAttribute("data-decision");
    expect(active("0")).toBe("theirs");
    expect(active("1")).toBe("both");
    expect(host.querySelector(".page-conflict-count")!.textContent).toBe("2 conflicts");
    dispose();
  });

  it("applies through the guarded marker path with the file's own base_rev, then reloads the page", async () => {
    setConflictInventory(inventoryWith(markerConflict));
    vi.spyOn(backend(), "vcsMarkerConflictDiff").mockResolvedValue({ mine_label: "HEAD", theirs_label: "feature", regions: 2, diff: diff(threeWayRows, "marker-rev") });
    const resolve = vi.spyOn(backend(), "resolveVcsMarkerConflict").mockResolvedValue();
    const sync = vi.spyOn(backend(), "resolveSyncConflict").mockResolvedValue();
    // The follow-up re-derivation never lands: the object must leave the queue
    // because the guarded write retired it, not because a later walk did.
    vi.spyOn(backend(), "conflictInventory").mockImplementation(() => new Promise(() => {}));
    const { host, dispose } = mount(markerConflict);
    await settle();
    button(host, "Apply resolution").click();
    await settle();
    expect(resolve).toHaveBeenCalledWith("pages/Merged.md", { "0": "theirs", "1": "both" }, "marker-rev", ["replace-page"], "union");
    expect(sync).not.toHaveBeenCalled();
    expect(conflictQueue()).toEqual([]);
    expect(doc.applyGraphChange).toHaveBeenCalledWith({ path: "pages/Merged.md", name: "Merged", kind: "page", created: false, removed: false });
    dispose();
  });

  it("routes a conflict copy through the sync resolve path, not the marker one", async () => {
    setConflictInventory(inventoryWith(copyConflict));
    const diffCall = vi.spyOn(backend(), "syncConflictDiff").mockResolvedValue(diff([threeWayRows[1]], "winner-rev"));
    const sync = vi.spyOn(backend(), "resolveSyncConflict").mockResolvedValue();
    const marker = vi.spyOn(backend(), "resolveVcsMarkerConflict").mockResolvedValue();
    // The follow-up re-derivation never lands: the object must leave the queue
    // because the guarded write retired it, not because a later walk did.
    vi.spyOn(backend(), "conflictInventory").mockImplementation(() => new Promise(() => {}));
    const { host, dispose } = mount(copyConflict);
    await settle();
    expect(diffCall).toHaveBeenCalledWith("pages/Plan.md", "pages/Plan.sync-conflict-20260705-141233-ABCDEFG.md");
    expect(host.querySelector(".page-conflict-side.theirs")!.textContent).toMatch(/^Sync copy · Jul 5/);
    button(host, "Apply resolution").click();
    await settle();
    expect(sync).toHaveBeenCalledWith(
      "pages/Plan.md", "pages/Plan.sync-conflict-20260705-141233-ABCDEFG.md", { "1": "both" },
      "winner-rev", "copy-rev", ["replace-page", "delete-page"], "union",
    );
    expect(marker).not.toHaveBeenCalled();
    expect(conflictQueue()).toEqual([]);
    dispose();
  });

  it("saves pending edits first and asks for a fresh review instead of writing over them", async () => {
    doc.dirty = true;
    const read = vi.spyOn(backend(), "vcsMarkerConflictDiff").mockResolvedValue({ mine_label: "HEAD", theirs_label: "feature", regions: 2, diff: diff(threeWayRows) });
    const resolve = vi.spyOn(backend(), "resolveVcsMarkerConflict").mockResolvedValue();
    const { host, dispose } = mount(markerConflict);
    await settle();
    button(host, "Apply resolution").click();
    await settle();
    expect(doc.flushPage).toHaveBeenCalledWith("Merged");
    expect(resolve).not.toHaveBeenCalled();
    expect(read).toHaveBeenCalledTimes(2);
    dispose();
  });

  it("refuses while the page holds its own save conflict", async () => {
    doc.conflicted = true;
    vi.spyOn(backend(), "vcsMarkerConflictDiff").mockResolvedValue({ mine_label: "HEAD", theirs_label: "feature", regions: 2, diff: diff(threeWayRows) });
    const resolve = vi.spyOn(backend(), "resolveVcsMarkerConflict").mockResolvedValue();
    const { host, dispose } = mount(markerConflict);
    await settle();
    button(host, "Apply resolution").click();
    await settle();
    expect(resolve).not.toHaveBeenCalled();
    expect(toasts().map((t) => t.message)).toContain("Resolve this page’s save conflict first, then apply this resolution.");
    dispose();
  });

  it("re-reads the file when the guarded write reports it changed on disk", async () => {
    setConflictInventory(inventoryWith(markerConflict));
    const read = vi.spyOn(backend(), "vcsMarkerConflictDiff").mockResolvedValue({ mine_label: "HEAD", theirs_label: "feature", regions: 2, diff: diff(threeWayRows) });
    vi.spyOn(backend(), "resolveVcsMarkerConflict").mockRejectedValue(new Error("conflict"));
    const { host, dispose } = mount(markerConflict);
    await settle();
    button(host, "Apply resolution").click();
    await settle();
    expect(read).toHaveBeenCalledTimes(2);
    expect(conflictQueue().map((c) => c.id)).toEqual([markerConflict.id]);
    expect(doc.applyGraphChange).not.toHaveBeenCalled();
    expect(toasts().some((t) => t.message.startsWith("The file changed on disk"))).toBe(true);
    dispose();
  });

  it("routes ordinary prose containing 'conflict' through the generic failure path", async () => {
    const read = vi.spyOn(backend(), "vcsMarkerConflictDiff").mockResolvedValue({ mine_label: "HEAD", theirs_label: "feature", regions: 2, diff: diff(threeWayRows) });
    vi.spyOn(backend(), "resolveVcsMarkerConflict").mockRejectedValue(new Error("a conflict of interest"));
    const { host, dispose } = mount(markerConflict);
    await settle();
    button(host, "Apply resolution").click();
    await settle();
    expect(read).toHaveBeenCalledTimes(1);
    expect(toasts().map((t) => t.message)).toContain("Couldn’t resolve it: a conflict of interest");
    dispose();
  });

  it("shows an unreadable conflict as text instead of blanking the page", async () => {
    vi.spyOn(backend(), "vcsMarkerConflictDiff").mockRejectedValue(new Error("io:NotFound"));
    const { host, dispose } = mount(markerConflict);
    await settle();
    expect(host.querySelector(".page-conflict-empty")!.textContent).toBe("Couldn’t read this conflict. (io:NotFound)");
    dispose();
  });
});

// Moved from the retired Settings merge modal (og 15b, I-22 / I-4): both files
// are admitted at the parse cap, so a diff is at most OUTLINE_MAX_DEPTH deep and
// must render in full, in document order.
describe("resolver review depth", () => {
  function deepRows(levels: number): DiffRow[] {
    let row: DiffRow | null = null;
    for (let level = levels; level >= 1; level--) {
      const v = (text: string) => ({ uuid: "", text, child_count: row ? 1 : 0 });
      row = { id: String(level), kind: "modified", mine: v(`mine ${level}`), theirs: v(`copy ${level}`), children: row ? [row] : [] };
    }
    return [row!];
  }
  const shown = () => [...document.querySelectorAll<HTMLElement>(".sync-merge-row")].map((row) => `${row.style.paddingLeft}:${row.querySelector(".mine")?.textContent}`);

  it("renders a conflict diff exactly at the outline cap", async () => {
    vi.spyOn(backend(), "syncConflictDiff").mockResolvedValue(diff(deepRows(OUTLINE_MAX_DEPTH)));
    const { dispose } = mount(copyConflict);
    await settle();
    const rows = document.querySelectorAll(".sync-merge-row");
    expect(rows.length).toBe(OUTLINE_MAX_DEPTH);
    expect(rows[rows.length - 1].textContent).toContain(`copy ${OUTLINE_MAX_DEPTH}`);
    dispose();
  });

  it("keeps document order and hides an unchanged row with its subtree", async () => {
    const row = (id: string, kind: DiffRow["kind"], children: DiffRow[] = []): DiffRow => ({ id, kind, mine: view(id), theirs: view(id), children });
    vi.spyOn(backend(), "syncConflictDiff").mockResolvedValue(diff([
      row("A", "modified", [row("B", "unchanged", [row("C", "modified")]), row("E", "added")]),
      row("D", "removed"),
    ]));
    const { dispose } = mount(copyConflict);
    await settle();
    expect(shown()).toEqual(["0px:A", "16px:E", "0px:D"]);
    const toggle = document.querySelector<HTMLInputElement>(".sync-merge-showunchanged input")!;
    toggle.checked = true;
    toggle.dispatchEvent(new Event("change", { bubbles: true }));
    await tick();
    expect(shown()).toEqual(["0px:A", "16px:B", "32px:C", "16px:E", "0px:D"]);
    dispose();
  });
});
