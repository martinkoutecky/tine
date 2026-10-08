# 0061. Unsaved drafts survive a crash in one app-data draft store

- **Status:** Accepted — Martin approved 2026-09-29 (og QUESTIONS Q6). A new
  persisted format (OG-RULES Rule 8) and a persisted per-edit artifact (AGENTS §12).
- **Date:** 2026-09-29

## Context

Tine keeps an edit that cannot be saved in the window: a page in conflict, or a
page whose save failed (disk full, a read-only file, a network drive that went
away). The recovery panel (GH #540) lets the user retry, open the page or copy
the draft while the window lives. A crash, a kill by the OS, or a power cut in
that state loses the draft, because it exists nowhere on disk: the page's file
holds the older text by design. Master keeps the equivalent draft for Concord
live-save conflicts in `<app data>/conflict-capsules/<graph-key>.v1.json`
(Harvest B3). og has no Concord live-draft slice yet (8e), and needs the same
guarantee for its own conflict and failed-save states.

## Decision

One store per graph, `<app data>/drafts/<graph-id>.v1.json`, never inside the
graph (`src-tauri/src/drafts.rs`). `<graph-id>` is the session file's graph key.
The envelope is `{"version":1,"drafts":[record…]}`. A record is
`{id, kind, session, page_name, path, reason, saved_at, page}`: `id` is
`<session>:<page name>`, `page` is the page as the editor holds it (a `PageDto`),
and `reason` is `conflict` or `save-failed`.

**One store, not two.** `kind` is `unsaved` for these drafts. The value
`live-conflict` is reserved for og's Concord live-draft capsule (slice 8e). That
capsule is a record in this store with its own fields, not a second file.

**Amendment 2026-09-29 (og 21a, slice 8e): the live-conflict capsule.** A page
whose save was refused because its file changed on disk (a plain `disk-changed`
conflict, outside any save group) is written with `kind: "live-conflict"` and
two more fields: `base_rev`, the revision the draft was edited from, and
`observed_rev`, the disk revision the refused save observed. Every other field,
the id (`<session>:<page name>`, one record per page whichever kind), the
bounds, the write cadence and the retirement rules are unchanged, so the
envelope stays `version: 1` and this is no new format: a reader that knows only
`unsaved` records still loads the file, and the Rust store already accepted
both kinds. After a restart the in-page resolver offers the capsule for its
page (also when the page itself cannot be opened, GH #541): it reviews the
kept draft against the file as it is now, 3-way when the Concord ledger (ADR
0056) retains `base_rev`, and Apply writes through the guarded
`resolve_live_conflict` command. The record is retired only after that commit
succeeds, and only in the graph it was read for; the recovery panel still
lists it for Copy and Dismiss. The ledger base is looked up by revision, never
copied into the record, so the capsule costs nothing beyond the `unsaved`
record it replaces (two revision strings, ≤ 130 bytes).

**When it is written.** Only while a page is at risk: from the moment the save
engine marks it conflicted or reports a failed save, until the page saves, the
user resolves the conflict (Keep mine or Use disk), or the page leaves this
window. While at risk, the draft is rewritten at most every 500 ms, and only
when it changed. It is also written at close before a window that keeps unsaved
pages closes. An ordinary save never writes here. At a graph switch, a page
edited while the next graph was loading (after the last flush) is written to
the old graph's store, named by its root because the window's binding has
already moved, under a session tag of its own, so reopening that graph in the
same window offers it (og T4).

**When it is read.** Once per graph open. Records from an earlier session are
offered by a sticky toast ("Tine kept unsaved drafts of … from an earlier
session", with Review). Review opens the recovery panel, which shows each draft
as source text with Copy draft, Copy complete recovery data and Dismiss. Only
Dismiss retires an earlier session's record: it is the user's choice.

**Bounds.** At most 64 records and 8 MiB per file.

**Writes.** Every write is `device_io::atomic_write` (temp + fsync + rename +
directory sync). Retiring the last record removes the file and syncs the
directory. Writes are serialized by one process lock.

## Refusals and their scenarios

| Refusal / degradation | In-scope scenario |
|---|---|
| A store write past 64 records or 8 MiB is refused. The draft stays in the window and a warning says it will not survive a crash. | Disk error or exhaustion. A page stuck for hours must not grow the file without limit. |
| An unreadable, malformed, foreign-version or oversized file is set aside as `.unreadable-<n>`. The store loads empty and the graph opens normally. | A crash or power loss leaving a torn file, a disk error, or a sync client delivering another build's file into app data. The bytes are kept, not deleted. |
| A record without an `id` or a known `kind` is refused. | Malformed input from a newer or older frontend. |
| A keeper call from a retired graph binding does nothing. | An honest graph switch racing a pending write, so a record can never land in the next graph's store. |

## Unit cost

- **Ordinary edit:** 0 bytes, 0 files. The store is untouched while saves
  succeed.
- **Edit to an at-risk page:** one rewrite of the whole store, at most every
  500 ms while the draft changes. The store holds every at-risk page of the
  graph.
  - 1-block page: ≈ 0.5 KB per flush (median 520 B, max 3.1 KB).
  - 60-block page: ≈ 10 KB (median 10.4 KB, max 17 KB, 55–65-block pages).
  - Largest page in the corpus (645 blocks): 115 KB.
- **Files per flush:** 1 file, plus 1 temp during publication.
- **Transport:** 0 bytes. App data is local and is not synced.
- **How it was measured:** the JSON record for every Markdown page of
  `~/research/logseq-anonymized` (1,075 files), with a 36-byte id per block and
  64-byte revision strings as an upper bound. 135 one-block pages; 14 pages with
  55–65 blocks.

## Alternatives rejected

- **Per-page draft files.** These save one rewrite per flush, but they need a
  directory listing and orphan cleanup, and 8e's capsule would need the same
  again. With the 64-record bound the whole file stays small.
- **The graph session JSON.** It is rewritten on every navigation, so drafts
  would ride on unrelated writes and inherit its recovery rules.
- **`localStorage` / IndexedDB.** WebKitGTK localStorage is not reliably
  persistent, and the storage is keyed per WebView origin rather than per graph.
- **Writing drafts on every edit.** This would add bytes to every keystroke
  flush. A page that is saving normally already has its text on disk.

## Consequences

- A killed Tine offers every draft it could not save on the next open of that
  graph. The drafts are kept until the user dismisses them.
- "Restore into the editor" is not offered. The user copies from the draft,
  which keeps the page's file the only authority. Whether to add an in-place
  restore (installing the draft as a Keep-mine conflict) is a separate
  question.
- Native kill-and-reopen proof is the Rust store tests plus the frontend
  session-restart test. A native E2E that SIGKILLs the app is still to be
  written (master's equivalent journey is quarantined).
