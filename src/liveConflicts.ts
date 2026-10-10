// Concord live-draft conflicts (og 8e): an editor draft the page host could
// not save because its file changed on disk, as an object for the in-page
// resolver. The host keeps that draft's crash-recovery copy until the conflict
// is resolved, so the only draft is the open page's text, which the resolver
// reads at each review. Derived on every read; nothing here is stored.
import { untrack } from "solid-js";
import { conflicts, liveConflictDraft, pageByName } from "./document";
import { conflictQueue, syncConflicts } from "./conflictQueue";
import type { ConflictObject, LiveConflictDraft, PageKind } from "./types";

function liveObject(name: string, path: string, kind: PageKind, live: LiveConflictDraft): ConflictObject {
  return {
    id: `live:${path}`,
    source: "live-save",
    page_name: name,
    page_path: path,
    kind,
    sides: [
      { role: "mine", label: "Your unsaved edits" },
      { role: "theirs", label: "The file on disk now", path },
    ],
    live,
  };
}

/** The live-draft conflict to resolve at page `name` loaded from `path`, if
 *  any. Tracks only the conflict state, never the page's blocks, so typing does
 *  not re-derive it. O(1) plus one page projection when the page is in a
 *  reported conflict. */
export function liveConflictForPage(name: string, path: string | undefined): ConflictObject | undefined {
  if (!path || !conflicts().includes(name)) return undefined;
  const draft = untrack(() => liveConflictDraft(name));
  return draft ? liveObject(name, path, draft.page.kind, { base_rev: draft.baseRev }) : undefined;
}

/** Every live-draft conflict, for the overview's "Unsaved drafts" group
 *  (master ConflictOverview). O(conflicts). */
export function liveConflictObjects(): ConflictObject[] {
  return conflicts().flatMap((name) => {
    const live = liveConflictForPage(name, pageByName(name)?.id);
    return live ? [live] : [];
  });
}

/** THE count of items needing a decision, one answer for the sidebar badge,
 *  the Settings pointer and the Conflicts overview (master counts its one
 *  combined queue): the derived artifact queue, a copy whose page is gone
 *  (nothing else points at it), and every live-draft conflict.
 *  O(conflicts + records). */
export function pendingConflictCount(): number {
  return conflictQueue().length + syncConflicts().filter((c) => !c.base_path).length + liveConflictObjects().length;
}
