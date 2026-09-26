import { describe, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { resetStore } from "./store";
import {
  journalConflicts, refreshJournalConflicts, refreshSyncConflicts,
  setJournalConflicts, setSyncConflicts, syncConflicts, toasts, setToasts,
} from "./ui";
import type { JournalConflict, SyncConflict } from "./types";

describe("late conflict lists (I-20)", () => {
  it("discards a duplicate-journal list and notification from the old graph", async () => {
    setJournalConflicts([]);
    setToasts([]);
    let finish!: (items: JournalConflict[]) => void;
    const list = vi.spyOn(backend(), "listJournalConflicts").mockImplementationOnce(() =>
      new Promise((resolve) => { finish = resolve; })
    );
    const refreshing = refreshJournalConflicts(true);
    resetStore();
    finish([{ title: "Old day", files: [] }]);
    await refreshing;
    expect(journalConflicts()).toEqual([]);
    expect(toasts()).toEqual([]);
    list.mockRestore();
  });

  it("discards a sync-conflict list and notification from the old graph", async () => {
    setSyncConflicts([]);
    setToasts([]);
    let finish!: (items: SyncConflict[]) => void;
    const list = vi.spyOn(backend(), "listSyncConflicts").mockImplementationOnce(() =>
      new Promise((resolve) => { finish = resolve; })
    );
    const refreshing = refreshSyncConflicts(true);
    resetStore();
    finish([{ path: "pages/old.sync-conflict.md", base_name: "Old", base_path: null, kind: "page", tag: "old", preview: "old" }]);
    await refreshing;
    expect(syncConflicts()).toEqual([]);
    expect(toasts()).toEqual([]);
    list.mockRestore();
  });
});
