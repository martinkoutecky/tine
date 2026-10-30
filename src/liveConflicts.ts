import { msg, render } from "./i18n";
// Concord live-draft conflicts (og 8e): an editor draft the page host could
// not save because its file changed on disk, as an object for the in-page
// resolver. The host keeps that draft's crash-recovery copy until the conflict
// is resolved, so the only draft is the open page's text, which the resolver
// reads at each review. Derived on every read; nothing here is stored.
import { untrack } from "solid-js";
import { conflicts, liveConflictDraft, pageByName } from "./document";
import { conflictForPage, conflictQueue, syncConflicts } from "./conflictQueue";
import type { ConflictObject, LiveConflictDraft, PageKind } from "./types";

function liveObject(name: string, path: string, kind: PageKind, live: LiveConflictDraft): ConflictObject {
  return {
    id: `live:${path}`,
    source: "live-save",
    page_name: name,
    page_path: path,
    kind,
    sides: [
      { role: "mine", get label() { return render(msg("live-conflicts-your-unsaved-edits")); } },
      { role: "theirs", get label() { return render(msg("live-conflicts-the-file-on-disk-now")); }, path },
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

/** The one conflict the page at `name`/`path` resolves next (GH #800). A page
 *  can hold its own refused save AND a queued artifact (a sync copy, VCS
 *  markers, a duplicate day) at once. Every artifact fold refuses while the
 *  page holds unsaved input it has not reviewed (the native writers meet it as
 *  a changed-on-disk refusal), so the live draft's review comes FIRST: once it
 *  is resolved the page is clean and the artifact's panel follows. Showing the
 *  artifact first left its Apply refusing forever, the save conflict's review
 *  unreachable. Nothing is written by choosing; every version stays where it
 *  is (the draft in the host's drafts, the copy beside the page). */
export function pageConflictToResolve(name: string, path: string | undefined): ConflictObject | undefined {
  return liveConflictForPage(name, path) ?? conflictForPage(path);
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
