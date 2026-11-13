import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { setGraphMeta } from "../graphSession";
import { applyGraphChangesBulk, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { mainPaneRouter, resetTabsToJournals } from "../router";
import { setConflictPolicyAlwaysAsk } from "../conflictPolicy";
import { PageView } from "./Page";
import { Settings } from "./Settings";
import { openSettings, closeSettings, setJournalConflicts } from "../ui";
import { setConflictInventory } from "../conflictQueue";
import type { PageRead, ConflictObject, SyncConflictDiff } from "../types";

afterEach(() => {
  resetStore(); resetTabsToJournals(); setGraphMeta(null); setConflictPolicyAlwaysAsk(false);
  closeSettings(); setJournalConflicts([]); setConflictInventory({ sync_conflicts: [], vcs_markers: [], queue: [] });
  vi.restoreAllMocks(); document.body.replaceChildren();
});

it.each(["Settings Merge", "duplicate-journal", "sync-copy", "vcs-markers"] as const)("%s: an own publication refreshes the already visible unheld page", async source => {
  const opened = await backend().loadGraph("");
  if (opened.kind === "focused_existing") throw new Error("missing graph");
  setGraphMeta({ ...opened.meta, root: "/synthetic-own-publication" });
  const before: PageRead = { name: "Oct 26th, 2026", title: "Oct 26th, 2026", kind: "journal", id: "journals/2026-10-26.md", rev: "A",
    pre_block: null, blocks: [{ id: "own-before", raw: "only in keeper", collapsed: false, children: [] }] };
  const after = { ...before, rev: "B", blocks: [...before.blocks, { id: "own-after", raw: "only in merged copy", collapsed: false, children: [] }] };
  let disk = before;
  const publish = async () => {
    disk = after;
    await applyGraphChangesBulk({ changes: [], answers: { rev: "1", inventoryChanged: false, blockRefCounts: {},
      summary: { rev: "1", pages: ["oct 26th, 2026"], blocks: [], linksTo: [], props: {}, inventoryChanged: false, full: false } } });
  };
  const diff: SyncConflictDiff = { base_rev: "A", conflict_rev: "copy", rows: [
    { id: "0", kind: "added", mine: { uuid: "", text: "only in keeper", child_count: 0 }, theirs: null, children: [] },
    { id: "1", kind: "removed", mine: null, theirs: { uuid: "", text: "only in merged copy", child_count: 0 }, children: [] },
  ], mine_pre: null, theirs_pre: null, pre_differs: false, blocks_identical: false };
  vi.spyOn(backend(), "duplicateJournalDiff").mockResolvedValue(diff);
  vi.spyOn(backend(), "syncConflictDiff").mockResolvedValue(diff);
  vi.spyOn(backend(), "vcsMarkerConflictDiff").mockResolvedValue({ mine_label: "HEAD", theirs_label: "branch", regions: 1, diff });
  vi.spyOn(backend(), "resolveDuplicateJournalDay").mockImplementation(publish);
  vi.spyOn(backend(), "resolveSyncConflict").mockImplementation(publish);
  vi.spyOn(backend(), "resolveVcsMarkerConflict").mockImplementation(publish);
  vi.spyOn(backend(), "mergePages").mockImplementation(publish);
  vi.spyOn(backend(), "conflictInventory").mockImplementation(() => new Promise(() => {}));
  vi.spyOn(backend(), "listJournalConflicts").mockResolvedValue([{ title: before.name, files: [
    { name: "2026-10-26.md", path: before.id, canonical: true, preview: "keeper" },
    { name: "2026_10_26.md", path: "journals/2026_10_26.md", canonical: false, preview: "copy" },
  ] }]);
  vi.spyOn(backend(), "getPage").mockImplementation(async () => disk);
  vi.spyOn(backend(), "getPageByPath").mockImplementation(async () => disk);
  loadSingle(before);
  mainPaneRouter.openPage(before.name, "journal", { inPlace: true });
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <><PageView /><Settings /></>, root);
  try {
    await vi.waitFor(() => expect(root.querySelector(".page-blocks")?.textContent).toContain("only in keeper"));
    expect(root.querySelector(".page-blocks")?.textContent).not.toContain("only in merged copy");
    // Own writes bypass the outside-change prompt, just as their host mail does.
    setConflictPolicyAlwaysAsk(true);
    if (source === "Settings Merge") openSettings("backups");
    else {
      const conflict: ConflictObject = { id: "own-artifact", source, page_name: before.name, page_path: before.id, kind: "journal", block_conflicts: 1,
        sides: [{ role: "mine", label: "keeper", path: before.id }, { role: "theirs", label: "copy", path: "journals/2026_10_26.md" }] };
      setConflictInventory({ sync_conflicts: [], vcs_markers: [], queue: [conflict] });
    }
    const action = () => [...root.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent?.trim() === (source === "Settings Merge" ? "Merge" : "Apply resolution"));
    await vi.waitFor(() => expect(action()).toBeDefined());
    action()!.click();
    await vi.waitFor(() => expect(disk.rev).toBe("B"));
    closeSettings();
    await vi.waitFor(() => expect(root.querySelector(".page-blocks")?.textContent).toContain("only in merged copy"));
  } finally { dispose(); }
});
