// Browser-mock Concord surface: sync-conflict copies and the derived conflict
// queue. Split out of mock.ts (over the size ratchet); nothing here writes.
import type { BlockDto, ConflictInventory, ConflictObject, MarkerConflictDiff, PageDto, SyncConflict, SyncConflictDiff } from "./types";

// Gated on the `?conflicts` flag, like the journal-day demo, so the reconcile
// surfaces stay out of the marketing screenshots by default.
function conflictsDemo(): boolean {
  return typeof location !== "undefined" && /[?&]conflicts\b/.test(location.search);
}

function listSyncConflicts(): SyncConflict[] {
  if (!conflictsDemo()) return [];
  return [
    {
      path: "pages/Project Plan.sync-conflict-20260705-141233-A2B2C3D.md",
      base_name: "Project Plan",
      base_path: "pages/Project Plan.md",
      kind: "page" as const,
      tag: "sync-conflict-20260705-141233-A2B2C3D",
      preview: "Milestones for the launch",
    },
  ];
}

function syncConflictDiff(): SyncConflictDiff {
  const v = (text: string) => ({ uuid: "", text, child_count: 0 });
  return {
    base_rev: "mock-sync-diff-rev",
    conflict_rev: "mock-sync-copy-rev",
    rows: [
      { id: "0", kind: "unchanged" as const, mine: v("Milestones for the launch"), theirs: v("Milestones for the launch"), children: [] },
      { id: "1", kind: "modified" as const, mine: v("TODO ship the beta by Friday"), theirs: v("TODO ship the beta by Thursday"), children: [] },
      { id: "2", kind: "added" as const, mine: v("write the release notes"), theirs: null, children: [] },
      { id: "3", kind: "removed" as const, mine: null, theirs: v("ask marketing for the banner"), children: [] },
    ],
    mine_pre: "title:: Project Plan",
    theirs_pre: "title:: Project Plan",
    pre_differs: false,
    blocks_identical: false,
  };
}

function conflictInventory(): ConflictInventory {
  const sync_conflicts = listSyncConflicts();
  const queue: ConflictObject[] = sync_conflicts.flatMap((copy) => copy.base_path ? [{
    id: `copy:${copy.path}`,
    source: "sync-copy" as const,
    page_name: copy.base_name,
    page_path: copy.base_path,
    kind: copy.kind,
    sides: [
      { role: "mine" as const, label: "This device", path: copy.base_path },
      { role: "theirs" as const, label: copy.tag, path: copy.path },
    ],
    block_conflicts: 3,
  }] : []);
  return { sync_conflicts, vcs_markers: [], queue };
}

// Body of the page the sync-copy conflict above belongs to (empty unless `?conflicts`):
// it matches the diff's "mine" side and is long enough to scroll, so the in-page
// resolver and its pinned dock are demonstrable in the browser mock
// (scripts/shot-conflict-dock.mjs). mock.ts wraps it in the page (I-12: PageDto
// literals live in mock.ts and document/convert.ts only; this file may not import
// document, which imports the mock).
export const CONFLICT_DEMO_PAGE = "Project Plan";
export function conflictDemoBodies(): BlockDto[][] {
  if (!conflictsDemo()) return [];
  const blk = (raw: string, id: string) => ({ id, raw, collapsed: false, children: [] });
  return [[
    blk("Milestones for the launch", "plan-0"),
    blk("TODO ship the beta by Friday", "plan-1"),
    blk("write the release notes", "plan-2"),
    ...Array.from({ length: 60 }, (_, i) => blk(`launch checklist item ${i + 1} - status notes and follow-ups`, `plan-f${i}`)),
  ]];
}

export const mockConflictApi = {
  async listSyncConflicts() {
    return listSyncConflicts();
  },
  async syncConflictDiff() {
    return syncConflictDiff();
  },
  async resolveSyncConflict(): Promise<void> {
    // no-op in the browser mock
  },
  async trashSyncConflict(): Promise<void> {
    // no-op in the browser mock
  },
  async duplicateJournalDiff(): Promise<SyncConflictDiff | null> {
    return null;
  },
  async resolveDuplicateJournalDay(): Promise<void> {
    // no-op in the browser mock
  },
  async conflictInventory(): Promise<ConflictInventory> {
    return conflictInventory();
  },
  async vcsMarkerConflictDiff(): Promise<MarkerConflictDiff | null> {
    return null;
  },
  async resolveVcsMarkerConflict(): Promise<void> {
    // no-op in the browser mock
  },
  // The browser mock never refuses a save, so a live conflict arises only in
  // tests (which stub these): the review is 2-way and Apply echoes the draft.
  async liveConflictDiff(_path: string, _page: PageDto, baseRev: string | null): Promise<SyncConflictDiff> {
    return { base_rev: baseRev ?? "", conflict_rev: "absent", rows: [], mine_pre: null, theirs_pre: null,
      pre_differs: false, blocks_identical: true };
  },
  async resolveLiveConflict(_path: string, page: PageDto): Promise<PageDto> {
    return { ...page, rev: `mock-live-${Date.now()}` };
  },
};
