// Concord live-draft conflicts (og 8e): an editor draft whose guarded save was
// refused because its file changed on disk, as an object for the in-page
// resolver. Two origins, one object shape:
// - the open editor's `disk-changed` conflict (not a save group's: the group
//   kinds stay on the conflict bar), whose draft the resolver reads from the
//   editor at each review;
// - a `live-conflict` capsule an earlier session kept in the draft store (og
//   ADR 0061), whose record is the only copy of that draft after a restart.
// Derived on every read; nothing here is stored.
import { untrack } from "solid-js";
import { conflictReason, conflicts, liveConflictDraft, pageByName } from "./document";
import { earlierDrafts } from "./draftStore";
import { conflictQueue, syncConflicts } from "./conflictQueue";
import type { ConflictObject, DraftRecord, LiveConflictDraft, PageKind } from "./types";

function liveObject(name: string, path: string, kind: PageKind, live: LiveConflictDraft): ConflictObject {
  return {
    id: live.record_id ? `live-record:${live.record_id}` : `live:${path}`,
    source: "live-save",
    page_name: name,
    page_path: path,
    kind,
    sides: [
      { role: "mine", label: live.restored ? "Your kept draft" : "Your unsaved edits" },
      { role: "theirs", label: "The file on disk now", path },
    ],
    live,
  };
}

/** The restored capsule for page `name` (or file `path`), newest first. */
export function restoredLiveRecord(name: string, path?: string | null): DraftRecord | undefined {
  return earlierDrafts().find((record) => record.kind === "live-conflict"
    && !!record.path && (path ? record.path === path : record.page_name === name));
}

/** The live-draft conflict to resolve at page `name` loaded from `path`, if
 *  any. Tracks only the conflict state and the restored records, never the
 *  page's blocks, so typing does not re-derive it. O(1) plus one page
 *  projection when the page is in a disk-changed conflict. */
export function liveConflictForPage(name: string, path: string | undefined): ConflictObject | undefined {
  if (conflictReason(name)?.kind === "disk-changed" && path) {
    const draft = untrack(() => liveConflictDraft(name));
    if (draft) return liveObject(name, path, draft.page.kind, { base_rev: draft.baseRev, restored: false });
  }
  const record = restoredLiveRecord(name, path);
  if (!record?.path) return undefined;
  return liveObject(record.page_name, record.path, record.page.kind, {
    page: record.page, base_rev: record.base_rev ?? null, restored: true, record_id: record.id,
  });
}

/** Every live-draft conflict, for the overview's "Unsaved drafts" group
 *  (master ConflictOverview): each open editor's disk-changed draft, then each
 *  restored record whose page has none. O(conflicts + records). */
export function liveConflictObjects(): ConflictObject[] {
  const open = conflicts().flatMap((name) => {
    const path = pageByName(name)?.id;
    const live = path && conflictReason(name)?.kind === "disk-changed" ? liveConflictForPage(name, path) : undefined;
    return live && !live.live?.restored ? [live] : [];
  });
  const paths = new Set(open.map((conflict) => conflict.page_path));
  const restored = earlierDrafts().flatMap((record) => record.kind === "live-conflict" && record.path && !paths.has(record.path)
    ? [liveObject(record.page_name, record.path, record.page.kind, { page: record.page, base_rev: record.base_rev ?? null, restored: true, record_id: record.id })]
    : []);
  return [...open, ...restored];
}


/** THE count of items needing a decision, one answer for the sidebar badge,
 *  the Settings pointer and the Conflicts overview (master counts its one
 *  combined queue): the derived artifact queue, a copy whose page is gone
 *  (nothing else points at it), and every live-draft conflict.
 *  O(conflicts + records). */
export function pendingConflictCount(): number {
  return conflictQueue().length + syncConflicts().filter((c) => !c.base_path).length + liveConflictObjects().length;
}
