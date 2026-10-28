# 0079 — Device-local git seen baselines

- Status: Accepted design, backend implementation; UI port follows.
- Date: 2026-10-28
- Authority: D-20261028-SG, seen-git DESIGN rev 1 with rev 2 overrides;
  D-20261018-SN1, D-20261017-SB2 and ADR 0078 identity conventions.

Desktop git graphs compare a page's baseline commit with HEAD. Only blocks
last touched by other authors count. "Me" is exactly the identity Tine's git
writers commit the user's changes as (ADR 0078; one function,
`git_run::user_identity`): the repository's `user.name`/`user.email` where
configured, "Your name" for a missing name, `user@tine.invalid` for a missing
email, `Tine user` when nothing is set. A configured email matches
case-insensitively whatever the name; the placeholder email matches only with
the same name, so another unconfigured Tine with another "Your name" counts as
someone else. `identity_known: false` means only the fixed fallback is known;
then the writer's own fallback commits still count as mine.
`tine_core::sync_diff` aligns page snapshots. More than 200 touching commits
falls back to an unattributed baseline-to-HEAD diff.

History is first-parent (accepted for 0.9): the page snapshots are those the
branch integrated, so edits that arrive through a merge count as the merge
commit's author's. A baseline reachable only through another parent is
reported unavailable.

Renames need no detection. A page with no record inherits the graph baseline
and attribution decides: when the user (Tine or an external tool committing
as the user) renamed or created the file, its blocks at the new path are the
user's and show nothing; when someone else did, they are theirs and show.
Tine's own rename, merge and delete drop the old page's record (by page key
and moved/trashed path, `forget_page`, also the `forget_seen_page` command);
the record is device-local, so a failure there is logged and never fails the
page operation. A record whose path differs from the page's current path
(an external rename under the same key) shows nothing until the page is left.

One versioned JSON file per canonical graph root lives in device-local app
data, using the session/drafts graph identity convention:
`seen-git/<session-id>.json`. It holds `first_open_commit`, `first_open_at`
and `pages: { page_key: { path, seen_commit, seen_at } }`, plus `version: 1`.
A null commit captures the empty tree for unborn HEAD. Graph binding records
the first available HEAD once. A page with no override inherits it. Leaving
or explicitly marking a page advances its override; marking the graph resets
the first-open baseline and clears overrides. Identical baselines do not
rewrite the file. Forget removes the record. Unknown or damaged records are
absent. Writes use the existing atomic device writer, never graph storage.

Git reads open an independent libgit2 handle off the UI thread; they never
write `.git`, acquire the run lock, acquire `index.lock`, or watch `.git`.
Android/iOS compile an unavailable stub. Read results map committed changes
onto the supplied current page snapshot using structural block paths. Local
changes do not acquire marks. Removed bodies carry surviving sibling anchors.
The UI re-reads on focus/visibility or page-file changes and marks on leave;
it must discard replies whose page snapshot or graph binding has changed.

Per open, the net baseline-to-HEAD alignment is computed once; each changed
HEAD block is then followed back through the touching commits, newest first,
only until the commit that changed it. A commit whose parent snapshot cannot
contain the block's body (some body line is absent from the parent's text)
changed it, so it needs no parse; otherwise both snapshots are parsed and
aligned. Removals follow lineage forward over every touching snapshot (the
restore rule needs it). Alignments trim content-equal leading and trailing
root blocks before `diff_blocks`, which keeps the full lsdoc projections to
the changed window, but only where `sync_diff::trimmed_alignment_is_exact`
proves the whole-list diff agrees: every root anchor fingerprint unique on
each side and the window's common anchors in order. A duplicate body or id,
or a reorder or move among anchors, falls back to the whole-list diff
(correctness over speed). The proof reads each distinct block body's id once
per open (`PersistedIds`). Parses, ids and alignments live only for one open
(no cache, D-10). Measured release medians, 600 blocks and 20 touching
commits by another author: about 4 ms with one changed block; about 17 ms
when the net diff also has a removal (bounded by one page parse per touching
commit). A page with duplicate blocks or a reorder pays the whole-list diff.

Unit cost (I-25): one graph record, O(tracked pages), rewritten on a changed
baseline; zero bytes, files or transport per edit.
