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
commit's author's. One open walks at most 500 first-parent commits in total
(`WALK_LIMIT`, touching the page or not; about 0.45 ms each on a 3000-page
`pages/` tree); a baseline further back shows the unattributed net diff with
a reason. A baseline that is not on the walked line (a squash, rebase or
amend took it off the branch, a mark mid-run included, or it came in from a
side branch) falls back to the newest walked commit among its ancestors (the
merge base, searched over at most 500 of its ancestors), which over-reports
and never hides; the result carries a `reason`. A baseline sharing no
history with the branch, or whose object is gone, falls back to the graph's
first-open baseline, else the page is unavailable with a reason. A page
version in history that is not UTF-8 makes that open unavailable with a
reason, never a command error.

Renames need no detection. A page with no record inherits the graph baseline
and attribution decides: when the user (Tine or an external tool committing
as the user) renamed or created the file, its blocks at the new path are the
user's and show nothing; when someone else did, they are theirs and show.
Tine's own rename, merge and delete drop the old page's record (by page key
and moved/trashed path, `forget_page`, also the `forget_seen_page` command);
the record is device-local, so a failure there is logged and never fails the
page operation. A leave mark that arrives after that drop names a path
no longer on disk and writes nothing, so it cannot re-create the dropped
record (a page created there later starts from the graph baseline). This is
a path check, not a tombstone: the record keeps no history of dropped keys. A record whose path differs from the page's current path
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
restore rule needs it). Alignments diff only the window between
content-equal leading and trailing root blocks
(`sync_diff::diff_blocks_trimmed`) when that provably gives the whole-list
`diff_blocks` pairing; otherwise they diff the whole list (correctness over
speed). A *cut* is a root whose anchor fingerprint occurs once on each side;
repeated bodies (empty bullets, `---`) are allowed. Within the exact-LCS
budget the region between the head's last cut and the tail's first cut must
have a unique longest pairing (counted by DP); above it, patience's anchors
(the cuts) must keep their order and the equal end gaps must fit the gap
budget. Ids are read once per distinct body per open (`PersistedIds`).
Parses, ids and alignments live only for one open (no cache, D-10). Measured
release medians, 600 blocks and 20 touching commits by another author: about
4 ms with one changed block; about 17 ms when the net diff also has a
removal; about 53 ms on a real-shaped page (repeated empty and `---` roots,
nesting, properties; 72 ms before the cut predicate), dominated by the
removal-lineage pass and one parse per touching commit.

Unit cost (I-25): one graph record, O(tracked pages), rewritten on a changed
baseline; zero bytes, files or transport per edit.
