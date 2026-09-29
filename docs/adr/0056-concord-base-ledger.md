# 0056. Concord base ledger: a disposable app-data record of each page's last agreed texts

- **Status:** Accepted (og port of master ADR 0056; persisted format approved by
  Martin, og QUESTIONS Q4, K=2, 2026-09-29)
- **Date:** 2026-09-29

## Context

A Syncthing or Dropbox conflict copy arrives with two texts: the winner and the
copy. Without their common ancestor the review is a 2-way diff, so every
changed block is a guess the user must make by hand, and two disjoint edits of
one block (one side dropped a word, the other appended one) can only be kept
side by side or one side lost. Master records the last text each page was
agreed at (ADR 0056) and turns the review into a 3-way diff whose rows carry
suggestions, including a composed `merged` body. og had the 3-way engine
(markers with a diff3 ancestor) but no ancestor for sync copies.

## Decision

`src-tauri/src/concord_ledger.rs` keeps, per graph, the last **K = 2 distinct**
texts of every page that Tine saved or admitted from disk.

**Where.** `<app data>/concord-ledger/<graph-id>/`, the same `<graph-id>` as
backups (`backup::root_backup_id`). Never inside the graph root, never synced.

| File | Contents |
|---|---|
| `pages/<sha256(path)>/index.json` | `{schema: 1, path, revs}`: sha256 of the retained texts, newest first, ≤ 2 |
| `pages/<sha256(path)>/<sha256(text)>` | one retained text, exact bytes |
| `pins/<sha256(copy path)>.json` | `{schema: 1, conflict_path, winner_path, hash}` |
| `pins/<sha256(copy path)>.blob` | the ancestor pinned for that copy, its own copy of the bytes |

Every write is `device_io::atomic_write` (temp + fsync + rename + directory
sync). A blob that falls out of retention is deleted in the same record.

**Fed by the change feed, never the save path.** The dispatch thread (the sole
`Store::subscribe` subscriber, `watcher.rs`) hands each published `Change`
to the ledger's worker thread: one channel send. The worker, per change:
1. for each sync copy that appeared, pins the newest retained winner text that
   differs from both files' current bytes (first pin wins, so a winner
   admission in the same change can never become the pin);
2. for each changed page whose current bytes still match the published
   revision, records them (a no-op when they equal the newest entry);
3. drops the pin of each copy that left the graph (resolve, discard, external
   delete).

At graph open the ledger prunes entries for pages and copies that are gone,
blobs no entry names, torn temps and corrupt entries. At `RunEvent::Exit`,
queued updates get one shared 200 ms drain.

**Never an authority.** Nothing waits on, refuses on, or fails because of the
ledger. Every read re-hashes the blob; a miss, a corrupt or foreign entry
(another page's index, another schema), or an unwritable directory answers
"no base" and the review is the 2-way diff. The only refusal involving it is
the resolve's `merge base changed since the review`: a `"merged"` row was
proposed against a base the diff stamped (`merge_base_rev`, the sha256 of the
base), and at apply time that base is gone or different (sync delivery or an
honest concurrent instance moved the ledger). Every other decision never reads
the base.

**Choosing the base.** `conflict_bases(copy, winner)` answers the pin, then the
winner's retained texts. The diff takes the newest candidate that differs from
the winner (master's rule: one equal to the winner is the admission artifact).
If that candidate equals the copy, the review stays 2-way (Tine addition): on
og it is normally the copy's own bytes — this device's last save, renamed to
the copy by Syncthing when the other device's edit won the winner name — and
3-way against it would pre-select discarding this device's edit. A copy that
genuinely equals the ancestor is indistinguishable, and falling back to an
older base could turn a winner-side revert into a "theirs" suggestion, so no
side is pre-selected (`concord_ledger_tests`
`a_base_equal_to_the_copys_own_bytes_is_not_used_as_the_ancestor`,
`a_copy_equal_to_the_newest_base_stays_two_way_without_reaching_older_bases`).

**Unit cost** (I-25; measured by
`concord_ledger::tests::unit_cost_per_recorded_save_is_one_blob_plus_one_index`,
sizes read back from disk after one recorded save):

| Page | Page bytes | Bytes written per save | Files written per save | Retained footprint (×K) |
|---|---:|---:|---:|---:|
| 1 block | 44 | 220 (text 44 + index 176) | 2 (new blob, rewritten index) + 1 evicted blob removed | 263 B in 3 files |
| 60 blocks | 2,631 | 2,808 (text 2,631 + index 177) | 2 + 1 removed | 5,438 B in 3 files |

A pin costs one more text copy plus ~250 B, once per conflict copy, and is
removed with it. Transport bytes per edit: **0** (app data is never synced).
CPU per save, off the UI and save threads: one bounded read of the saved page
and one sha256 of it. Graph-scale work happens only in the open-time prune
(O(ledger files)).

## Consequences

- A Syncthing copy of a page Tine has saved or seen is reviewed 3-way with
  pre-selected suggestions; disjoint edits of one block are offered as one
  merged body (`merged`), applied only after the user confirms it.
- Losing the ledger (new device, cleared app data, unwritable disk) only loses
  suggestions; nothing in the graph depends on it.
- App data holds at most K texts per page Tine has seen; a page Tine never saw
  since the ledger existed has no entry and gets the 2-way review.
