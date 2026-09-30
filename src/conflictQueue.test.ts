import { afterEach, describe, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { toasts, setToasts } from "./toasts";
import { conflictInventory, conflictQueue, setConflictInventory, settleArtifactConflict, syncConflicts } from "./conflictQueue";
import { refreshSyncConflicts } from "./ui";
import type { ConflictInventory, ConflictObject } from "./types";

// og 8c: the derived conflict queue. Recomputed from disk, never persisted.

const EMPTY: ConflictInventory = { sync_conflicts: [], vcs_markers: [], queue: [] };
function copyObject(name: string): ConflictObject {
  const path = `pages/${name}.sync-conflict-20260705-141233-ABCDEFG.md`;
  return {
    id: `copy:${path}`, source: "sync-copy", page_name: name, page_path: `pages/${name}.md`, kind: "page",
    sides: [{ role: "mine", label: "This device", path: `pages/${name}.md` }, { role: "theirs", label: "sync-conflict", path }],
  };
}
function inventoryOf(...names: string[]): ConflictInventory {
  const queue = names.map(copyObject);
  return {
    sync_conflicts: queue.map((c) => ({ path: c.sides[1].path!, base_name: c.page_name, base_path: c.page_path, kind: "page" as const, tag: "sync-conflict", preview: "" })),
    vcs_markers: [],
    queue,
  };
}

afterEach(() => {
  setConflictInventory(EMPTY);
  setToasts([]);
  vi.restoreAllMocks();
});

describe("the derived conflict queue", () => {
  it("announces only sync copies that arrived since the last refresh", async () => {
    vi.spyOn(backend(), "conflictInventory").mockResolvedValueOnce(inventoryOf("A")).mockResolvedValueOnce(inventoryOf("A", "B"));
    await refreshSyncConflicts();
    expect(toasts()).toEqual([]);
    await refreshSyncConflicts("new");
    expect(toasts().map((t) => [t.message, t.action?.label])).toEqual([["1 new sync conflict needs review", "Review"]]);
    expect(conflictQueue().map((c) => c.page_name)).toEqual(["A", "B"]);
  });

  it("keeps the last inventory and reports a failed read", async () => {
    setConflictInventory(inventoryOf("A"));
    vi.spyOn(backend(), "conflictInventory").mockRejectedValue(new Error("io:PermissionDenied"));
    await expect(refreshSyncConflicts()).resolves.toBeUndefined();
    expect(conflictInventory()).toEqual(inventoryOf("A"));
    expect(toasts()).toEqual([expect.objectContaining({ kind: "error", sticky: true })]);
  });

  it("settles a resolved object at once, and an older walk cannot resurrect it", async () => {
    setConflictInventory(inventoryOf("A", "B"));
    let finish!: (inventory: ConflictInventory) => void;
    vi.spyOn(backend(), "conflictInventory").mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
    const walking = refreshSyncConflicts();
    settleArtifactConflict(copyObject("A").id);
    finish(inventoryOf("A", "B"));
    await walking;
    expect(conflictQueue().map((c) => c.page_name)).toEqual(["B"]);
    expect(syncConflicts().map((c) => c.base_name)).toEqual(["B"]);
  });

  // Master 042054c1b: a sticky "needs review" notice names live objects, so it
  // is retired once they are gone, or it contradicts the green "Merged" toast.
  it("retires the arrival notice when a re-derivation no longer has its conflict", async () => {
    vi.spyOn(backend(), "conflictInventory").mockResolvedValueOnce(inventoryOf("A")).mockResolvedValueOnce(EMPTY);
    await refreshSyncConflicts("new");
    expect(toasts().map((t) => t.message)).toEqual(["1 new sync conflict needs review"]);
    await refreshSyncConflicts();
    expect(toasts()).toEqual([]);
  });

  it("retires the arrival notice when its conflict is settled locally, and keeps one with a live conflict", async () => {
    vi.spyOn(backend(), "conflictInventory").mockResolvedValueOnce(inventoryOf("A", "B"));
    await refreshSyncConflicts("new");
    expect(toasts()).toHaveLength(1);
    settleArtifactConflict(copyObject("A").id);
    expect(toasts()).toHaveLength(1);
    settleArtifactConflict(copyObject("B").id);
    expect(toasts()).toEqual([]);
  });
});
