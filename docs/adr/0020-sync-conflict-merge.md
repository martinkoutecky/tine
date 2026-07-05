# 0020. Sync-conflict copies: detect, structural-merge, resolve only through the safe save path

- **Status:** Accepted
- **Date:** 2026-07-05

## Context

Syncthing and Dropbox drop a `*.sync-conflict-*` (or `(conflicted copy)`) file
into the graph whenever the same page is edited on two devices before they sync.
These accumulate in real graphs. Tine used to index them as ordinary pages —
garbage entries in All-Pages/quick-switch, and (because the copy shares the
winner's `id::` values) churn in the id space.

We want to attack this symptom directly without building the quarter-scale op-log
sync engine (the Deferred CRDT item): detect the copies, keep them out of the page
list, and offer a **manual** block-level merge against the winning page. The hard
parts are (a) not corrupting the user's real graph, and (b) making the diff the UI
shows and the merge the resolve applies agree exactly.

Alternatives considered: a text-blob 3-way merge (rejected — Logseq files are
block trees; a line diff mangles nesting and can't express per-block keep-both);
auto-merging (rejected outright — data-safety); a dedicated writer for the merged
file (rejected — would bypass the one-writer invariant).

## Decision

We will:

1. **Detect by filename** (`sync_conflict_base` / `is_sync_conflict`) and exclude
   conflict copies from the *listing* sites only — `list_md` (page list + cache
   warm) and `sync_file_content` (watcher/load reconcile). We will **not** exclude
   them in `is_page_file`/`entry_for_path`/`resolve_rel`, so the merge UI can still
   load a copy by path.
2. **Diff as a block-tree alignment**, not text. One shared alignment intermediate
   (`sync_diff::Node`) is the single source of truth that BOTH the diff rows the UI
   renders and the merged output the resolve produces derive from — so a row id
   addresses the same block in both. Matching per sibling list: L1 same `id::` or
   content-equal (LCS), L2 first-line similarity (Levenshtein > 0.8), L3
   added/removed. Quadratic only in siblings-at-one-level; on-demand, never a hot
   path.
3. **Resolve only through the normal save path.** `resolve_sync_conflict` runs
   under the winner's `page_lock`, honours a `base_rev` guard (winner changed on
   disk → conflict, no write), applies the org round-trip firewall, unions
   markdown page properties, writes via `save_page`/`commit_write`/`atomic_write`,
   and **stages the conflict copy into `.tine-trash` BEFORE committing the merged
   winner** (rolling the move back on write failure). Discard-without-merge
   (`trash_sync_conflict`) refuses anything that isn't a conflict copy.

## Consequences

- Conflict copies stop polluting the page list and the id space, and a real
  reconcile path exists — a concrete win for multi-device Syncthing users.
- The diff/merge symmetry (one alignment intermediate) means the UI and the applied
  result can't silently disagree; adding a merge behaviour means changing one place.
- Reusing the `merge_pages`/#21 machinery (page_lock, base_rev, stage-before-commit,
  `.tine-trash`) keeps this inside the existing one-writer + never-silently-overwrite
  invariants (ADR 0007, ADR 0012) — no new data-danger surface.
- keep-both duplicates a subtree; the pulled-in copy has its `id::`s stripped so it
  can't collide with the winner's on disk — the cost is that copy loses its stable
  id (becomes a fresh block), which is correct for an intentionally-duplicated block.
- Scope stays bounded to detection + manual 2-way merge. It is explicitly **not** a
  sync engine; the op-log CRDT remains Deferred, with this item as its prerequisite.
