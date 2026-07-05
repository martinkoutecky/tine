# Plan 2 — Syncthing sync-conflict detection + block-level merge UI

**Status:** grounded, ready to execute · **Est:** ~3–5 days · **Backlog:** P2 ·
**Data-safety critical** (writes the user's real graph — see ADR 0007/0012)

## Goal

Syncthing (and Dropbox/git) leave `*.sync-conflict-*.md` copies rotting in the graph
when the same file was edited on two devices. Today Tine **silently indexes them as
pages** (garbage entries in All-Pages / quick-switch). Build the pragmatic slice that
attacks the symptom Martin actually hits, without adopting a CRDT/op-log:

1. **Detect** conflict copies (don't index them as pages).
2. **Surface** them (a badge/list of "N unresolved conflicts").
3. **Merge**: a per-block 2-way structural diff of the conflict copy against the
   winning file, with per-hunk keep-mine / keep-theirs triage.
4. **Resolve** safely: write the merged winning file through the normal save path,
   move the conflict copy to trash (never unlink, never auto-merge silently).

Origin: `subagent-tasks/notes/outl-study.md` §7 (the "conflict files on a Markdown
graph" problem; outl's 3-level structural matcher is the algorithmic model, not its code).

## Current state (grounded)

- **Conflict copies are ingested as pages.** The only filename gate is extension:
  `is_page_file` (`crates/tine-core/src/model.rs:58`) = `md|org`, no stem check. A
  `Foo.sync-conflict-….md` passes → `list_md` (`model.rs:2869`) derives a garbage page
  name; the whole-graph scan (`model.rs:2312`, `:2879`) and the watcher reconcile
  (`watcher.rs:117`, same `md|org` filter → `graph.sync_file`) all pick it up.
  **A new stem predicate must be threaded through all four sites.**
- **id:: collision is already partly handled:** `assign_uuids_rec` (`model.rs:2920`)
  gives duplicate `id::` values (which a conflict copy shares with the winner) fresh
  uuids after the first — so loading both files won't corrupt the id space, but it's a
  reason to keep the conflict copy *out* of the normal page cache.
- **Block identity for the diff:** `BlockDto.id` (`model.rs:100`) / `DocBlock.uuid`
  (`doc.rs:44`) = persisted `id::` if present, else a fresh random uuid per load. So
  **id-based matching only works for blocks that carry `id::`.** For the common case
  (no `id::`), use the existing **content-equality** primitive: `DocBlock` subtree
  equality that ignores uuid (`doc.rs:~116`, the external-change conflict guard) and
  `doc_has_content` (`model.rs:2897`).
- **Trash infra to reuse directly:** `move_to_trash` (`model.rs:3165`), `trash_stamp`
  (`:3157`), dir `logseq/.tine-trash` (outside pages//journals/ → never re-scanned).
- **The whole feature has a working template: the #21 duplicate-day journal reconcile.**
  Detect `journal_conflicts()` (`model.rs:698`, command `list_journal_conflicts`
  `commands.rs:422`) → inspect `read_journal_file` / `load_by_path` (`model.rs:1240`,
  loads a specific file by rel path even when (kind,name) collide) → `merge_pages`
  (`model.rs:796`, command `commands.rs:453`; appends src blocks to dst via the normal
  save path, then trashes src, with **stage-src-in-trash-before-committing-dst**
  ordering "L5" `model.rs:858`) → rescue `rename_file_to_page` (`model.rs:460`). Mirror
  this shape, but per-block instead of whole-file append.
- **Safe write-back (must use, not bypass):** `save_page` (`commands.rs:133` →
  `model.rs:2566`) takes a `base_rev`; under `page_lock` it rechecks `content_rev(disk)
  == base_rev` and returns `"conflict"` **without writing** on mismatch; `write_page` →
  `commit_write` (`model.rs:2371`) → `atomic_write` (`model.rs:3185`) does temp+fsync+
  rename, **skips byte-identical rewrites** (anti-Syncthing-churn), and sets a
  **self-write marker** (`note_self_write` `model.rs:2341`, consumed by
  `sync_file_content` `model.rs:2417`) so the watcher ignores Tine's own write. ADR 0012
  (one writer path) + ADR 0007 (never silently overwrite; recoverable trash; `.org`
  rewritten only if byte-reproducible, else read-only).
- **No block-level 2-way diff/merge algorithm exists in the repo.** The
  `src/devtools/lsdoc-diff/` panel (ADR 0018) is a *parser* oracle (lsdoc vs mldoc
  projections), not a merge tool. The block matcher is net-new.

## Approach

### Detection (backend)
Add a stem predicate `is_sync_conflict(stem)` (matches `.sync-conflict-` per Syncthing's
`name.sync-conflict-YYYYMMDD-HHMMSS-XXXXXXX.ext` scheme; also tolerate Dropbox's
"(conflicted copy)"/"conflicted copy" and a generic pattern). Thread it so conflict
copies are **excluded** from `is_page_file`/`list_md`/scan/watcher page ingestion, and
routed to a new enumerator `list_sync_conflicts() -> Vec<SyncConflict>` where each entry
carries: conflict file rel path, the **winning** page it shadows (strip the
`.sync-conflict-…` infix from the stem → the base page name/path), device/timestamp
parsed from the suffix, and whether the winner still exists. Command `list_sync_conflicts`;
watcher emits a `conflicts-changed` event on create/delete of a conflict file.

### Structural block diff (backend, net-new — the real work)
Given the winner doc and the conflict doc (both via `parse_doc` / `load_by_path`),
produce an ordered list of block-level hunks. Matching algorithm, adapting outl §7's
3-level matcher over lsdoc's block tree (NOT a text-blob diff):
- **L1 exact:** same `id::` (if present) OR content-hash + same parent path → unchanged
  (no hunk).
- **L2 near:** same-parent (± small position window) + high content similarity
  (normalized Levenshtein > ~0.8) → a **modified** hunk (mine vs theirs).
- **L3 none:** present only in winner → **added**; present only in conflict →
  **removed**. Always a hunk, never silently dropped.
Output `SyncConflictDiff { hunks: Vec<Hunk{ kind, path, mine, theirs }> }`. Use the
existing content-equality primitive (`doc.rs:~116`, `doc_has_content`) for L1/L2 so
id-less blocks still match. Keep it O(n) per sibling list (LCS/Myers over a sibling
sequence, recursed into children) — no quadratic whole-tree compare.

### Merge UI (frontend)
Model on the journal-conflict UI but per-hunk. A conflicts panel/badge (list from
`list_sync_conflicts`) → open a conflict → a two-column block diff (winner vs conflict)
with per-hunk **keep-mine / keep-theirs / keep-both** and a top-level "keep entire
winner" / "keep entire conflict" escape hatch. Nothing is applied until the user
confirms. Reuse `read_journal_file`/`load_by_path` to load both files by exact path.

### Resolve (backend, via the safe path only)
Compose the merged block tree per the user's hunk choices, serialize through the
**normal round-tripping save** (`save_page`/`commit_write`/`atomic_write`) with the
winner's current `base_rev` (or `force_save_page` only if the user explicitly chose
keep-mine-wholesale and accepts the overwrite), then `move_to_trash` the conflict copy
— mirroring `merge_pages`' **stage-src-in-trash-before-committing-dst** ordering so a
retry after a failed trash can't duplicate. Command `resolve_sync_conflict(winner_path,
conflict_path, decisions)`.

## Steps

1. **Detection + exclusion** (backend): `is_sync_conflict`, thread through the 4 sites,
   `list_sync_conflicts` + command + watcher event. Test: a conflict copy no longer
   appears in `list_pages`, does appear in `list_sync_conflicts`. **Ship this first** —
   it alone fixes the "garbage pages" symptom, even before the merge UI.
2. **Block diff** (backend): the 3-level matcher + `SyncConflictDiff`, unit-tested on
   hand-built winner/conflict pairs (added/removed/modified/reordered blocks; with and
   without `id::`). Fuzz against the existing derived-cache fuzz style if feasible.
3. **Conflicts surface** (frontend): badge + list panel from `list_sync_conflicts`.
4. **Merge UI**: two-column per-hunk triage.
5. **Resolve** (backend) via `save_page`/`commit_write` + `move_to_trash`, stage-before-
   commit ordering; a round-trip test (merge → winner has chosen blocks, conflict is in
   `.tine-trash`, no data lost, `content_rev` bump).
6. Docs: FEATURES, ADR (a short one — this is a load-bearing new writer + data-safety
   surface; record the "resolve goes only through save_page/commit_write, stage-before-
   commit" invariant), CHANGELOG; remove the backlog row.

## Risks / decisions (data-safety first)

- **Never auto-merge or auto-delete.** Every resolution is user-confirmed; the conflict
  copy goes to `.tine-trash`, never unlink. (ADR 0007.)
- **Only write through `save_page`/`commit_write`/`atomic_write`.** No bespoke write
  path (ADR 0012). Honor `base_rev` — if the winner changed on disk mid-merge, surface
  a conflict and re-diff, don't clobber.
- **id:: collisions:** while a conflict is loaded for diffing, load it via `load_by_path`
  (isolated), do **not** merge it into the live page cache (avoid the global id-dup
  churn `assign_uuids_rec` would trigger).
- **`.org` conflict copies** may be read-only if not byte-reproducible (ADR 0007) —
  detect and offer keep-mine/keep-theirs-wholesale rather than per-block for those.
- **Matcher false-pairs:** L2 similarity threshold can mis-pair; when in doubt, prefer
  showing two hunks (add + remove) over a wrong "modified" pairing — the user can
  keep-both. Tune the threshold on real conflict files from Martin's graph.
- **Scope guard:** this is detection + 2-way manual merge, NOT a sync engine / op-log
  (that's the Deferred CRDT item). Don't creep.

## Acceptance

- A `*.sync-conflict-*.md` in the graph: (1) is **not** in All-Pages / quick-switch;
  (2) shows in the conflicts list with its base page + device/timestamp.
- Opening it shows a correct per-block diff vs the winner (added/removed/modified),
  working both with `id::` and without (content-matched).
- Resolving writes the merged winner via the normal save path (atomic, self-write
  marked so the watcher doesn't loop), moves the conflict copy to `.tine-trash`, and
  loses nothing — verified by a round-trip test on Martin's real conflict files.
- If the winner changed on disk during the merge, the resolve surfaces a conflict
  instead of overwriting.
