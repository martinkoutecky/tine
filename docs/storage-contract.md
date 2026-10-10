# og plain-file storage contract

The graph files are the durable record. A save compares the caller's revision with
the current file, then publishes whole bytes through a synced temporary file.
A create uses a no-clobber rename; replacing an existing page uses an ordinary
rename after the final guard. Another local writer can
replace that file between the final comparison and rename; mandatory cross-process
locks are outside this plain-file contract. The editor retains unsaved content
on every refusal and offers conflict resolution or retry.

A failed config read opens the graph read-only: existing files remain readable,
page DTOs carry read-only, and transactions reject every mutation before disk
steps. Repair `config.edn` externally and refresh/reopen to resume writes using
its configured directories. The graph-open reply reports the config failure;
setup never throws over it.

Asset trash rechecks references in the latest published graph under the Store
writer lock, including external/sync arrivals already published by the watcher.
An incomplete graph view cannot prove orphan-ness and refuses trash. External
arrivals not yet published, and changes after the final check by another process,
remain a window the Store writer cannot exclude. Duplicate-day stray reference
completeness remains a separate recorded finding.

`Store::read_is_current` compares a streaming descriptor with a fresh validated
open using the existing cross-platform file-identity primitive, O(1). Verification
uses it after hashing and requires successful metadata reads; a displaced
file makes the report incomplete. An external replacement after that check is
still possible; verification does not lock external editors for a whole scan.

The store discovers regular `.md`, `.markdown`, and `.org` pages (case insensitive
extensions) throughout the graph, except
hidden and reserved folders such as `assets/`, `publish/`, and `node_modules/`.
The graph-relative file path remains the write identity. An ordinary page's
nonempty preamble `title::` (or Org title directive) is its logical name;
without one, the configured filename decoder supplies the name. A file whose
title cannot be read (the title reader panics on malformed imported
Markdown/Org) keeps its decoded filename name and is listed as unreadable;
it never stops the initial load or another page's lookup (GH #644). Name lookup,
the published inventory, references, and direct page reads use that one
effective name. A file named for a logical page wins over a second file that
claims the same name through `title::`. Editing a title rekeys the published
name without moving the file. New page filenames use a reversible, injective
Windows-safe codec; existing noncanonical paths remain pinned.

Page and journal deletion is idempotent after external removal (GH #620).
When the supplied displayed path no longer claims the requested name, deletion
succeeds without writing only if the refreshed inventory has no file claimant
and a fresh store read confirms that path is absent. A live file whose title
changed, a replacement claimant at another path, and ambiguous twins still
refuse; ordinary deletion rereads the revision and uses recoverable trash.
An external recreation after the absence read is left untouched. Proof:
`crates/tine-graph-features/tests/page_delete_missing.rs` and the native
`scripts/e2e-page-file-identity.mjs` menu/watcher journey.

After applying a transaction, the store reads each final file before declaring
its publication complete. A failed read or revision check returns
`TxOutcome::PublicationIncomplete` with graph-relative file locations; disk
steps may already have landed. An apply failure keeps any publication errors in
`TxOutcome::NotCommitted`. `Store::save_pages` carries rollback failures in
`undo_failed` and omitted final-state files in `publication_errors`, both as
graph-relative locations. The frontend keeps unsaved edits, marks matching
pages conflicted, and tells the user which files need inspection before retry.

A held `WholeGraph` view does not wait for later writers. Acquiring the first
view with `whole_graph()` can wait for the initial parse. The public operation
surface is 43 operations: the methods of `Store` and `Transaction` plus every
public function taking either. `crates/tine-store/SURFACE.txt` budgets it per
concept and `crates/tine-store/tests/shallow_ratchet.rs` keeps this count true.
The graph-command boundary guard lives at `crates/tine-store/tests/graph_command_boundary.rs`; the client path guard is
`crates/tine-store/tests/client_root_boundary.rs`.

Initial graph loading is not exposed as a state (the readiness probe
`is_graph_ready()` is a test-faults oracle). While the initial parse runs,
graph-wide answers block. After a failed initial parse, `page()` can read and
parse an existing file, but saves and observed edits do not publish a graph
generation; `whole_graph()` returns the load error. A successful
`refresh()` (at any `Depth`) retries the load and publishes a fresh
generation. A closed store is terminal.

`Store::refresh(Depth)` is the one refresh door. `Depth::Stamps` trusts a file
whose modification time, length and identity are unchanged (the watcher's
depth and the focus-return rescan's); `Depth::Bytes` hashes every graph-text
file and publishes the ones whose bytes differ as ordinary external changes
(scenario: a sync client or restore tool rewrites a file and puts its stamp
back; an export must not publish the stale document); `Depth::Rebuild` does
that and then re-parses every file through the cold-launch build (Settings
"Rescan graph").

A page open never waits for the whole graph (GH #623 BR3). `Store::page_named`
is the one page-by-name door: while the graph is loading (initial parse
running, or a launch checkpoint served while its diff runs), `page()` and
`page_named()` parse the file from disk without taking the writer; a name only
the index can resolve (an alias, a `title::` page no file is named for) waits
for Ready. After a failed parse `page_named()` answers from the file-name
listing. A save made in that window survives the load: the initial parse or
launch diff sees its new stamp. After Ready, a page open reads only that page:
canonicality comes from the published name index (`Store::canonical_claim`),
never from rereading every page's preamble. Proof:
`crates/tine-store/src/store/page_open_tests.rs`,
`crates/tine-graph-features/tests/br3_page_open_before_ready.rs`.

While a page host holds a page (STEP3 §5, amendment A-V4), its publication
consumer, or a reservation's transaction, is the page's only index writer
(R13). Invariant (G) (A-H2): every installed row about a held page (its
cache document, revision, anchor, file time, error rows, listing entry and
claimants, and the published snapshot's roots) comes from the bytes that
writer last published, and its name and document come from those same bytes.
A held page's row is installed only from those bytes, and only their parse:
a Document the owner supplies contributes its runtime identities alone, and
only when its content is exactly that parse (R8). One held but not yet
published, published absent, or whose identity is unknown (B1) has no row.
Every ownership change (hold, release, respelling, owner absence, a first
publication that fails) is one transition under the cache lock: it settles
every row whose authority it changes, the key's spellings and the colliding
entries that become unknown, with their error rows, and the writer section
that ran it publishes a snapshot when it settled any, so a view acquired
after it never shows the old row. A release rereads the files it withheld.
A disk disappearance never removes a held page's row; an owner's absent
publication does. One identity
rule (`Graph::identify`: the canonical existing parent, `Outside` when it
leaves the root, then the exact leaf; a folded collision is distinct only
when both names are listed apart and are different files) decides which held
key a path names, for the held map, the host and the watcher; one pass
resolves each parent and lists each directory once. A new entry whose
spelling is another entry's key gets a key of its own. Discovery, error and
build installs decide authority inside the installing critical section
(cache, then held, then the collection), so nothing decided before a
transition lands after it. A snapshot removal is named by its path
and the previous snapshot's name, with no file read. A launch checkpoint's
rows carry no owner provenance: while any page is held, the whole
checkpoint is declined and the launch builds. `page()` answers the bytes the
owner last published before reading anything of the file (or, before its
first publication, parses the file without publishing it). A write computed
from a page parses the bytes whose revision guards it (`Store::read` then
`Store::page_of`; A-V4b), never `page()`. With no host running nothing is
held and every path reads the file. Proof:
`crates/tine-store/src/page_host/binding_tests.rs`
`v4_a_read_or_rebuild_never_indexes_a_held_page` and
`a_consumer_publication_names_the_bytes_it_publishes`,
`crates/tine-store/src/model/derived_tests.rs`,
`crates/tine-store/src/model/entry_identity_tests.rs`,
`crates/tine-store/src/model/held_index.rs` (the `review2_*` tests and the
seam guard),
`crates/tine-store/src/store/checkpoint_tests.rs`
`a_checkpoint_is_declined_whole_while_a_page_is_held`,
`crates/tine-graph-features/tests/page_rename_merge.rs`
`merge_keeps_edits_held_pages_have_not_indexed`.

A save that changes only blocks' `collapsed::` property (value `true`, `false`
or absent, decided by the parser, `model/collapse_only.rs`) publishes a
generation that inherits the alias list and every unaffected memo, and does
not mark the launch checkpoint dirty; the next launch diff rereads the folded
file by its stamp. Unit cost: one fold writes 1 page file (68 B for a
1-parent page, 1529 B for a 60-block page) with 2 fsyncs, no checkpoint
rewrite and no work proportional to the graph. Proof:
`crates/tine-store/src/store/fold_save_tests.rs`.

## Page host binding (step 3b)

No production page host starts before step 3b P2b: `PageHost::start` is
crate-private and `start_for_tests` is test-gated, so everything below is
reachable only with a test host until then. Unit cost: none; no persisted
record changes (sessions, the orphan flag and the retirement map are memory
only).

**Sessions.** Every host launch and every `page_window_reloaded` draws a
process-unique session; the reload answers it with `next_id`, one past the
host's last admitted request. Every mutating page command (`page_open`,
`page_submit`, `page_move`, `page_discard`, `page_close`, `page_delete`)
carries the session and is not admitted under any other (`NotAdmitted`; a
delete answers `Refused`), so a command from an earlier window, owner
binding or host instance never applies. Mail carries the session it was
collected under, and the window drops mail of any other. Proof
`crates/tine-store/src/page_host/binding_tests.rs`
`e101_every_host_instance_and_reload_draws_a_fresh_session`.

**Launch on failing draft I/O (B-Q1).** The host always starts. A drafts
directory that cannot be created, synced or listed leaves draft I/O down:
the launch recovers nothing and touches no vehicle (declared deviation M1
from `storage-s3.qnt:609-619`, whose launch reads every draft), saves
proceed, and moves, renames and deletes that need an operation draft fail
with a draft error. A vehicle whose quarantine fails stays in place and
untouched, named in the status. A census that cannot be synced keeps the
recovered buffers held and at risk and retires nothing (M2). Every page a
vehicle known on disk then names is cleanup debt, kept apart from the
recovered evidence until a re-probe syncs the census: no switch or stop
finishes over it, its key is not evicted, and a stop that needs it cleared
aborts at once naming the page (a restore never waits for draft I/O under
the restore lock; a retiring host's stop re-probes on each pass). Scenario:
power loss before the census sync leaves the copy unsynced, and closing
over it would let the next launch recover a stale copy over a newer or
restored file. M1 (an unlisted census) is unchanged. `load_graph`
replies with `draft_status` (`{unavailable, unreadable}`; none with no host).
`page_drafts_retry` re-probes draft I/O in the running host, keeping the
census and live input, and finishes the cleanup launch skipped. Vehicles
first listed by a Retry are not recovered into the running host: draft I/O
stays down and the status says they are recovered when Tine restarts (a
"Restart Tine" action is P2b UI). Proof
`crates/tine-store/src/page_host/production_tests.rs` (`b_q1_*`).

**Retirement (plan v3 §3).** A released binding (window destroyed, window
switched graph, failed open or failed window creation) whose host runs is
not dropped: in the same registry-locked step it moves to `HostRetirement`,
so no opener sees its root unowned. The retirement ends the gone window's
session once (the model's `windowCrash`, `storage-s3.qnt:588-590`), lets
every admitted request apply with custody, then stops the host over that
watermark and closes the Store. A stop that cannot keep custody (a draft
failure) is aborted, the host keeps its pages, and the stop is retried with
backoff (1 s doubling to 30 s); there is no timeout. Reopening a retiring
root adopts its Store and host under a fresh binding (lease, background
work, mail retargeted to the new window; one driver per graph); the old
binding's late release finds a different lease and does nothing. The
adoption revokes the old binding under its host gate: a writer,
transaction or restore that captured it before and has not entered the
gate gets `stale-graph-binding`, never the no-host path on the adopted
Store, and one already inside finishes first. Lock order: registry, host
gate, retirement map, host state; nothing under the gate takes the registry
(`src-tauri/src/host_slot_guard_tests.rs`). Every exit is the
retirement waiter's (`exit_after_retirement`; scenario: quitting while a
host still holds unsaved input). Closing the last graph window first
unbinds its graph, retiring its host, and the window stays until the exit
(GH #28). The exit waits for every retirement, 30 s at most; past that Tine
keeps running with the host alive (the stuck-graph window is P2b). The exit
is then decided under the open lock (`graph_load`): a graph bound or
adopted meanwhile, an open still inside its load, or any other graph
window, loaded or not, cancels it (`state::exit_when_unowned`; the guard
`state::tests::every_exit_goes_through_the_retirement_waiter`). Proof
`src-tauri/src/host_retirement.rs` tests,
`crates/tine-store/src/page_host/binding_stop_tests.rs` (`an_orphan_stop_*`,
`retargeted_mail_reaches_the_adopting_window`).

**Restore gate (S8).** A backup restore holds the binding's host lock
across stop, file restore and relaunch: hosted writers, a retirement, an
adoption and another restore wait for its outcome. The reply carries the
relaunched host's session (`reloaded`) on success and on a partial restore
failure alike. Proof `src-tauri/src/backup/restore.rs`
`a_restore_holds_the_host_gate_until_one_fresh_host_runs`,
`a_retirement_waits_for_the_restore_it_raced`.

## Graph backups (og-backup-cas)

A graph's schema-4 snapshots live in `<app data>/backups/<graph-id>.cas/`,
outside the graph and beside the legacy namespace `<app data>/backups/<graph-id>/`
that schemas 2 and 3 use (`src-tauri/src/backup.rs`). Two assumptions bound
the guarantees below:

- **A-bk1.** The backup directory is private to the device. Only Tine
  processes write it, and no sync tool delivers into it. Builds without
  schema 4 (master v0.6.98, og before og-backup-cas) reach only the legacy
  namespace; they never open `<graph-id>.cas/`.
- **A-bk2.** A launch snapshot taken in an earlier launch has been through
  the OS's writeback, so after a power cut an older snapshot remains (see
  Prune).

The layout and protocol:

- **Layout.** `<graph-id>.cas/` holds `blobs/`, `snapshots/` and `lock`.
  `blobs/<sha256>` holds each distinct file content once. A snapshot
  directory `snapshots/<UTC stamp>[-<suffix>][-<n>]` holds only
  `snapshot.json`: schema 4, the graph root, the pages/journals directories,
  the graph-text policy (ADR 0062), `writer: "og"`, `files: [{path, sha256}]`,
  `complete: true` and `checksum`. Paths are schema 3's: `graph/<graph-relative path>`,
  `<assets dir>/<rel>` for `.edn` sidecars, `logseq/config.edn`.
- **Lock.** Every operation on the namespace (snapshot write, keep-count,
  partial cleanup, blob collection, restore's read and verification) holds
  one OS file lock on `lock` (`File::lock`: `flock` on Unix, `LockFileEx`
  on Windows). It excludes other Tine processes and other threads alike. A
  launch snapshot that finds it held is skipped and logged
  (`backup-skipped`): another process is backing up this graph now. A
  suffixed snapshot, a keep-count change and a restore wait up to 30 s; a
  suffixed snapshot or restore that times out is refused (refusal table).
  The process-wide `BACKUP_WORK` permit is only an in-process copy
  throttle, taken before the lock.
- **Write.** Under the lock, every included file is read through the Store
  and hashed. No mtime or size shortcut: a tool that preserves mtime would
  otherwise make a snapshot record stale content. A blob is written only when
  its name is absent, or present with bytes that differ from the content
  (torn by power loss, compared length first, then in 64 KiB chunks); an equal
  blob is reused. It is written to a `create_new` temp unique to the process
  and call (`.tmp-<sha256>-<pid>-<n>`), then renamed over its name. The
  manifest is written last into `snapshots/.partial-<name>/`, and a
  no-replace rename publishes it. A name taken in the same second gets a
  counter `-2`, `-3`, …, ordered numerically (`-10` after `-9`).
- **Manifest checksum.** `checksum` is the SHA-256 of the manifest's other
  fields serialized as compact JSON with keys sorted (unknown fields
  included). Listing and restore read a schema-4 manifest only when its
  checksum matches: a bit flip in a path, hash or scope that leaves valid JSON
  makes the snapshot damaged, not a different snapshot. Schemas 2 and 3 have
  no checksum, as before.
- **Listing is not verification.** Listing reads manifests only; it never
  opens blobs. A listed snapshot can still fail restore's verification.
- **Restore verifies, then consumes the verified bytes.** Under the lock,
  restore reads every listed blob into memory and checks its SHA-256 before
  the pre-restore safety snapshot and before any graph write. After the
  safety snapshot, it stages exactly those verified bytes in unnamed temp
  files and hands them to `Store::restore`; a blob changed after
  verification never reaches the graph. A snapshot with a missing or damaged
  blob, a torn manifest or a checksum mismatch is refused: "this backup is
  damaged; pick another snapshot". The next launch rewrites a torn blob whose
  content the graph still holds, which makes older snapshots that list it
  restorable again.
- **Durability.** A launch snapshot syncs nothing; the next launch re-takes
  it. A suffixed snapshot (`pre-restore`, `pre-journal-rename`) precedes a
  destructive operation the user asked for, so it is made durable before that
  operation runs, and a failed sync fails it and refuses the operation.

| Target | Launch snapshot | Suffixed snapshot |
|---|---|---|
| Linux | no file, directory or filesystem sync | `fsync` every listed blob, `blobs/`, the manifest and its directory before the rename; `snapshots/` after |
| Android | as Linux | as Linux |
| macOS | as Linux (no `syncfs`; not measured on Apple hardware) | as Linux (`fsync`, not `F_FULLFSYNC`, as before) |
| iOS | as macOS | as macOS |
| Windows | no flush; the one publication rename is `MoveFileExW(MOVEFILE_WRITE_THROUGH)` | `FlushFileBuffers` on every listed blob and the manifest; directory syncs are no-ops; write-through rename |

  A `syncfs` after a launch snapshot was considered and not used: its cost is
  bounded by all dirty data on the filesystem, not by the backup, and the
  repair-on-reuse above covers what it would protect.
- **Prune.** After a launch's snapshot, under the lock, the keep-count
  deletes the oldest snapshot directories this build counts, in both
  namespaces (schema 2, and schema 3/4 marked `writer: "og"`; never
  `-pre-restore` or another Tine's snapshots). The snapshot this process's
  latest launch took is never counted, so `keep + 1` routine snapshots
  remain and an earlier launch's (A-bk2) survives a power cut before this
  launch's snapshot reaches the disk. The exclusion is per process: a
  launch snapshot another running Tine process took counts as routine here
  (residual, A-bk2 does not cover it when this process has no launch
  snapshot of its own). A pruned schema-4 snapshot is first
  renamed to `.partial-<name>`; then each blob it listed that no other
  directory under `snapshots/` lists is deleted, then the directory. The
  blob store is not enumerated. Any doubt (an unreadable directory or
  manifest) deletes nothing; a manifest that is not valid JSON lists nothing
  (it never restores). Cleanup removes `snapshots/.partial-*` only under the
  lock, so each is abandoned. When it finds one, or a snapshot write fails,
  the whole blob store is collected: every entry (blob or temp) that no
  manifest under `snapshots/` lists is deleted. A steady-state launch never
  enumerates `blobs/`. The legacy namespace's `.partial-*` entries are left
  alone: an older build may be writing one.
- **Older snapshots.** Schema 3 (full copy under `graph/`) and schema 2
  (`journals/`, `pages/`) in the legacy namespace are listed, verified
  against their manifest, restored and pruned as before; nothing is migrated.
  A build without schema 4 never sees `<graph-id>.cas/` (A-bk1), so it neither
  lists, restores, counts nor deletes schema-4 snapshots or blobs, and its own
  keep-count of the legacy namespace is unchanged.

Proof `src-tauri/src/backup.rs`
`publication_follows_every_blob_write_and_restore_verifies_every_blob`,
`unchanged_launch_writes_no_blob_and_edits_write_only_theirs`,
`a_torn_blob_is_repaired_not_reused`,
`a_shared_blob_survives_pruning_one_of_its_snapshots`,
`a_prune_racing_a_snapshot_never_deletes_its_blobs`,
`a_launch_never_prunes_the_last_earlier_snapshot`,
`same_second_counters_order_numerically`,
`steady_state_launches_never_enumerate_the_blob_store`,
`a_failed_sync_fails_a_snapshot_taken_before_a_rewrite`,
`the_manifest_checksum_is_pinned`; two OS processes:
`a_prune_in_another_process_never_deletes_a_writers_blobs`,
`a_launch_in_another_process_skips_while_a_writer_holds_the_lock`,
`a_same_content_blob_write_in_two_processes_is_serialised`;
`src-tauri/src/backup/restore.rs`
`schema_4_restore_is_byte_identical_and_damage_is_refused`,
`og_full_copy_snapshot_restores_and_prunes_beside_schema_4`,
`a_bit_flip_in_a_manifest_path_is_refused`,
`a_blob_changed_after_verification_never_reaches_the_graph`.

**Unit cost:** 0 bytes, 0 files and 0 transport bytes per edit on a 1-block or
a 60-block page: snapshots are taken at launch and before a user-requested
rewrite, never per edit. Per launch backup of a copy of the anonymized graph
(1,075 text files, 1,300,622 bytes; measured 2026-10-10 with
`launch_backup_cost_on_corpus`). It counts files created that survive the
snapshot, their sizes, and sync calls. It is not full I/O accounting: temp
files, repair overwrites, directory metadata and the Windows write-through
rename are not counted.

| Launch | Before (schema 3 full copy) | After (schema 4) |
|---|---|---|
| First backup | 1,076 files, 1,455,521 bytes, 1,083 syncs | 942 files (940 distinct blobs, manifest, `lock`), 1,455,359 bytes, 0 syncs |
| Unchanged graph | 1,076 files, 1,455,521 bytes, 1,083 syncs | 1 file (manifest), 154,981 bytes, 0 syncs |
| 3 pages edited | 1,076 files, 1,455,599 bytes, 1,083 syncs | 4 files (manifest + 3 blobs), 163,373 bytes, 0 syncs |

The manifest records the graph root, so its size moves with the root path's
length. Linux wall time (release build, ext4 on NVMe, 3 alternating runs,
snapshot plus prune): before 629–771 ms first, 1,027–2,162 ms unchanged,
671–810 ms after 3 edits; after (round 2, with the lock and checksum)
99–103 ms, 59–61 ms and 59–78 ms. N
changed pages cost the manifest plus N blobs of those pages' sizes. With
the default keep-count of 12, an unchanged graph's 13 retained launch
snapshots hold the graph's bytes once plus 13 manifests (about 3.3 MB here;
schema 3 kept 12 full copies, about 17.5 MB).

## I-8 refusal scenarios

Each row below is keyed by the source file, owning function and refusal family.
The count is the number of production constructions or save-adapter outcomes;
the source guard fails when an unreviewed site appears. These scenarios involve
ordinary sync, external editors, user actions, malformed files, or graph lifecycle.

| Key | Count | In-scope scenario and required response |
|---|---:|---|
| `transaction.rs::refuse_marker_rewrite::ReadOnly` | 1 | R-VCS-MARKERS for reference rewrites and moves that rewrite references (og 21a, master a8fd4230d): an external VCS merge or a sync service left a referrer mid-conflict; rewriting `[[Old]]` inside it would edit sides of a merge the user has not adjudicated. The rename skips such referrers (byte-identical, a moved one moves verbatim) and reports them; this refuses any other caller's rewrite. A rewrite that changes nothing passes. |
| `transaction.rs::path::InvalidTarget` | 3 | A sync update changes a path into a symlink or invalid area while an operation resolves it; refuse access outside the approved graph root. |
| `transaction.rs::twin::Twin` | 1 | Sync introduces a second physical file for the same page name or journal day; refuse a write that could choose the wrong one. |
| `transaction.rs::preflight::InvalidTarget` | 10 | A legitimate caller supplies a malformed path or unsupported operation (including moving config), or an external rename changes target shape; refuse before any step changes disk. Config moves require live config publication. |
| `transaction.rs::page_save_target::InvalidTarget` | 3 | A page save names a non-page path or unsafe page nesting, or a caller attempts to save a virtual Guide DTO; refuse before any step changes disk. Guide content needs an explicit graph copy. |
| `transaction.rs::page_create_checks::Twin` | 1 | A sync-created same-name page appears between page creation and commit; refuse the ambiguous create. |
| `transaction.rs::page_open_checks::InvalidTarget` | 1 | A page host open (STEP3 §2) names a non-page path, as a malformed link or an external rename leaves it; refuse before the page is registered or anything changes disk. |
| `transaction.rs::serialize_page::ReadOnly` | 2 | An Org page no longer round-trips after an external edit; keep its bytes and the unsaved editor proposal. R-VCS-MARKERS: a VCS merge by an external writer left unresolved markers; a rewrite would re-indent them and silently lose a side. Only `SaveBase::ResolvingMarkers` (the Concord resolver) passes, and it first stages the old bytes in `logseq/.tine-trash/conflicts/`. |
| `transaction.rs::unreadable_owner::UnreadableOwner` | 1 | R-CREATE-UNREADABLE-OWNER (master 69e0a885ddf9 + 69525c055f0b, GH #543): sync delivery, an interrupted external write or malformed imported Markdown/Org leaves a page file Tine cannot read (not UTF-8 where its name lives, an oversized preamble, a permission error) or a directory it cannot list, so the file's page name is unknown; creating a page of a name it could carry would give that name two files once the bad file is repaired. Refuse a name-only creation (`CreateNew` save, `create`, `create_unique` of an ordinary page) only for a name that entry could be (`Graph::unreadable_page_could_own`: its file-name name; its preamble name when that still decodes; otherwise a name its bytes, folded as page names are, contain; any name for an unlistable directory or a file unreadable now; nothing for a FIFO, socket, device, journal or vanished entry), and name the entry. Every other creation, save, move and the watcher proceed. |
| `transaction.rs::preflight::RepeatedFile` | 1 | Two steps of one user operation choose the same unique filename; refuse the ambiguous plan. |
| `transaction.rs::apply::InvalidTarget` | 1 | A generated unique candidate is no longer a valid target after a concurrent change; refuse that candidate. |
| `transaction.rs::apply::RepeatedFile` | 1 | A unique candidate collides with another step after planning; refuse rather than overwrite. |
| `transaction.rs::apply::Undecodable` | 2 | Imported bytes fail UTF-8 validation after staging; remove the stage and keep the original. |
| `transaction.rs::commit_timed::Closed` | 1 | A graph is closed while a queued save waits; refuse its old binding. |
| `transaction.rs::commit_timed::RepeatedFile` | 1 | A multi-step action names one file twice; refuse before any write. |
| `transaction.rs::rewrite::Undecodable` | 1 | Sync makes a referrer invalid UTF-8 before rename rewrite; keep that file and refuse the rename. |
| `transaction.rs::rewrite_move::Undecodable` | 2 | Sync leaves a title-owned move source or its rewritten bytes invalid UTF-8; refuse the rename before moving the file or publishing a new title. |
| `transaction.rs::refuse_read_only_org::ReadOnly` | 1 | An Org referrer is not round-trip editable (malformed imported Org); keep its bytes instead of rewriting it, whether preflight computes the rewrite or reuses one prepared by the rename planner (GH #623). |
| `transaction.rs::rewrite_move::ReadOnly` | 1 | An imported Org page named by `#+TITLE:` or a `:title:` drawer is not round-trip editable; refuse the rename rather than rebind its title in bytes Tine cannot reproduce. |
| `transaction.rs::content_refusal::Undecodable` | 1 | An existing or imported page has invalid UTF-8; refuse the write without reporting a transient I/O failure. |
| `transaction.rs::content_refusal::InvalidTarget` | 1 | Existing or serialized page content exceeds the byte or nesting parse bound; refuse while retaining unsaved edits. |
| `transaction.rs::validate_page_content::InvalidTarget` | 1 | A raw page stream exceeds its cap; refuse before creating an unreadable page. |
| `transaction.rs::validate_config_bytes::InvalidTarget` | 1 | A config edit or create names a directory outside the graph; refuse before changing disk. |
| `store.rs::save_pages::Closed` | 1 | A queued page-save request arrives after graph close; return the closed family without writing. |
| `store.rs::save_pages::InvalidTarget` | 2 | An empty page-save request or an entry without edit kinds is refused before opening a transaction (`store_save.rs`). |
| `store.rs::save_pages::GuideEphemeral` | 1 | A bundled Guide page has no graph file; refuse the request before disk access. |
| `store.rs::from_failed_step::Closed` | 1 | A transaction closes before commit; retain all unsaved page snapshots. |
| `store.rs::from_failed_step::Conflict` | 1 | An external edit makes an entry's target revision stale; return its current disk revision for resolution. |
| `store.rs::from_failed_step::Deleted` | 1 | Sync deletes an entry's page while its editor buffer is open; retain the buffer and report deletion. |
| `store.rs::from_failed_step::ReadOnly` | 1 | A parser or format check rejects a round-trip edit, or the file carries unresolved VCS markers (R-VCS-MARKERS); retain the buffer. |
| `store.rs::from_failed_step::Twin` | 2 | A second same-name physical file appears before or after the guarded create; identify that claimant instead of presenting its revision as the target's. |
| `store.rs::from_failed_step::InvalidTarget` | 2 | A saved target becomes invalid or undecodable; retain the buffer. |
| `store.rs::from_failed_step::UnreadableOwner` | 1 | R-CREATE-UNREADABLE-OWNER: a new page's name may already belong to an unreadable file; return that file as a recovery location (wire family `unreadable-owner`), keep the buffer dirty, and let the user repair or move the file. |
| `store.rs::from_failed_step::Repeated` | 1 | Two entries name the same file; refuse the request before writing either entry. |

The following rows cover refusals outside the two constructor families counted
above. Their individual call sites are inventoried in `og/batches/E-survey-errors.md`
§I-8; the key here is the production owner and operation family. Error-return
sites that report a disk failure rather than refusing an operation are covered
by I-9's typed failure paths.

| Owner / operation family | In-scope scenario and required response |
|---|---|
| `tine-store::model` path and graph acquisition | A configured graph or asset directory is retargeted, malformed, or resolves outside the approved root after sync or external editing; refuse reads and writes through that path. |
| `tine-store::store` read, scan and handoff | Graph close revokes queued requests; a symlink, non-file (for the asset opener: neither file nor directory), or escaped path appears after the caller selected it, or sync retargets `assets/`; refuse stale bytes and OS handoff. A failed initial parse withholds an unpublished view. |
| `tine-store::model::configured_hidden` scope (og T2, master `hidden_parse_failed_closed`) | A torn or hand-broken `config.edn` (sync-service delivery, an external-editor race or crash) leaves `:hidden` malformed or over its limits (unterminated vector, bad string escape, over 256 entries or 64 KiB, nesting over 32): graph-text scope hides all graph text instead of reading "nothing hidden", which would list, index, snapshot and export text the owner excluded. Recovery is fixing `config.edn`; the watcher's config reload restores the scope. A snapshot taken meanwhile holds no graph text and records `hidden_parse_failed_closed`, so its restore retires none (proof `crates/tine-store/tests/store_read.rs` `malformed_hidden_value_hides_all_graph_text`, `src-tauri/src/backup/restore.rs` `failed_closed_hidden_snapshot_records_scope_and_retires_nothing`). A missing `:hidden`, or a non-vector value, hides nothing (OG). |
| `tine-store::watch` reconcile | A graph closes, its root disappears, or an external config edit changes directory layout; stop reconciliation rather than publishing a false view. |
| `tine-store::watch` OS watch setup (og 22a) | The OS refuses live file notifications for the graph (inotify's per-user watch limit is spent by other apps, a network or FUSE mount without events, a root replaced while open); degrade to a 3-second poll that runs the same reconcile, retry the live watch every cycle, and report the refusal and its restoration (`Subscription::observe_watch_status`, `graph-watch-refused` / `graph-watch-restored`, flight event `watcher.refused`). The graph is never silently stale (I-9). |
| `tine-store::transaction` undo of a replaced file | Disk full, a disk error, or a crash while rolling back a failed multi-file save (C3 L07): undo stages the file's pre-transaction bytes as a `tx-old` conflict-trash copy before it withdraws the transaction's bytes, and removes that copy once the old bytes are back live. If the copy cannot be written it withdraws nothing, leaves the transaction's bytes live and reports the file in `undo_failed` (proof `c3s_content_loss.rs`). |
| `tine-graph-features::conflicts::resolve_sync_conflict` pairing | Sync-service delivery (Dropbox, Syncthing, Seafile) of a conflict copy whose base page is missing or whose page name holds parentheses (C3 L01): a copy is merged only into the page its own name shadows (`sync_copy_winner`); any other pairing is refused before anything is written or trashed, and the copy stays for its real reconcile (proof `crates/tine-graph-features/tests/c3s_sync_copy_base.rs`). |
| `src-tauri::capture_target::resolve` native capture into a named graph (og H1b) | An Android photo or voice memo that finishes after the window switched graphs has its only copy in the app cache: it is written into the graph it was started in, named by the frontend, through the ordinary asset transaction, never into the new graph, and the user is told where it went. The caller must still be a live bound window. A named root that is not a known graph on this device is refused before anything is written (scenario: plugin or web content in the WebView naming an arbitrary directory; an honest capture always names a graph this device opened). A root that no longer resolves to a folder (moved or deleted by sync or the user mid-capture) is refused by path acquisition. In both cases the capture stays in the cache and the failure is reported (proof `src-tauri/src/capture_target.rs` tests, `src/components/Block.mobileCapture.test.tsx` "stopped after a graph switch"). |
| `tine-graph-features::conflicts::resolve_duplicate_journal_day` pairing (master 9dc54e4a7) | A duplicate journal day (a date-stem file plus a title-named one, left by a journal date-format change or delivered by sync) folds only a stray of the SAME day into that day's canonical file: a stale review after sync-service delivery, an external rename or a date-format change that re-sorted the day refuses before anything is written or trashed, so the command can never merge two unrelated pages; a Markdown/Org pair of one day (malformed pairing: one body would be rewritten in the other format) is refused and offers no row choices. Past the pairing guard it is the sync-copy fold (`fold_pair`): the same revision guards, Org round-trip firewall and recoverable trash (proof `crates/tine-graph-features/tests/f8_duplicate_journal.rs`). |
| `tine-graph-features::conflicts::union_pre` page-property merge (og C3W W5) | Honest multi-device divergence: both devices set the same page property (or an Org drawer line) to different values and the user resolves with the page properties on "both (merge)". The copy's other pre-block lines, list members and free text are kept; a value that cannot be kept twice refuses the resolve naming the key, before anything is written or trashed, so the user chooses mine or theirs (proof `crates/tine-graph-features/tests/c3w_union_pre.rs`). |
| `tine-graph-features::pages::rename_page_after_inventory` unreadable page (og C3W W2) | Malformed imported or sync-delivered content (non-UTF-8, over the size or outline-depth cap) in a page the rename must move or rewrite: the rename refuses naming the file, before anything is written, instead of skipping it while rewriting its referrers (proof `crates/tine-graph-features/tests/c3w_rename_unreadable.rs`). |
| `tine-store::restore` source, destination and recovery | A selected snapshot source changes type or length, a live/recovery path is retargeted, or an external writer creates the destination; refuse publication and retain displaced bytes in recovery. A whole-graph restore (og-B, ADR 0062) takes graph text only as `Area::Graph` inside the graph-text scope the snapshot recorded, and a configured-roots restore never takes `Area::Graph`: a malformed or mixed snapshot refuses before any live file moves (proof `crates/tine-store/tests/restore.rs` `graph_restore_refuses_text_outside_its_recorded_scope`). It retires only unlisted live text inside that recorded scope, into `<restore-id>/graph/`, so text a later `:hidden` edit exposed is never displaced by a snapshot that could not have held it. Power loss after the replacement is published must not lose a retired file (its only copy can be an external editor's edit newer than the safety snapshot): the recovery root, every directory created for it, and each retired entry are synced before the replacement is published, and a real directory-sync failure (disk error) stops the restore with its recovery locations reported (proof `crates/tine-store/tests/restore.rs` `restore_makes_the_recovery_tree_durable_before_publishing_replacements`). Transactions that move, trash or create into a directory they create sync every created entry the same way (`directory_durability::create_dir_all_durable`). |
| `tine-store::publish` staged site | An external writer retargets output or stage paths or wins the destination name; refuse replacement and retain the previous site. |
| `tine-graph-features::pages` rename, rescue, merge and delete | A referrer carrying VCS conflict markers (an external merge or sync left it mid-conflict) is skipped, not rewritten, and reported (`skipped_conflicted_referrers`; R-VCS-MARKERS). Sync or an external editor changes a revision, creates a twin, occupies a destination, or makes Org non-round-tripping; refuse the affected transaction and retain source bytes. A rescue also refuses a name already carried by a retained non-portable legacy filename (`pages/A:B.md` from OG on Linux/macOS), which OG would load as a second file for that page. Delete refuses a stale path if an external editor changed its title or replaced its claimant; confirmed file absence succeeds without writes (GH #620). A rename commits every step only in the view and config it was planned in (`Transaction::expect_view`): sync or an external editor delivering a referrer or `config.edn` between plan and commit makes the attempt replan, within the existing four attempts, then `WouldBlock` (A-W1 B2; proof `crates/tine-graph-features/src/pages_snapshot_tests.rs`). |
| `tine-graph-features::conflicts` resolve | A sync conflict winner or copy changes or disappears during resolution, or an Org member is not editable; preserve both sides and require retry. A `"merged"` row whose reviewed Concord-ledger base (`merge_base_rev`) is gone or different at apply time (sync delivery or an honest concurrent instance moved the ledger, or the ledger became unreadable) refuses with `merge base changed since the review` and writes nothing; every other decision never reads the base, so a ledger failure refuses nothing else (ADR 0056). |
| `tine-graph-features::live_conflict` merge and resolve (og 8e; the merge is read-only, the old engine's resolve writes its result guarded by the reviewed revision) | The file changed after the review (external-editor race, sync-service delivery, an honest concurrent instance), including a file that reappeared after an `absent` review: refuse with `live conflict changed on disk`, write nothing, and the resolver refreshes its review; the draft stays in the editor and in the draft store. A `"merged"` row whose reviewed ledger base (named by its sha256) is no longer retained refuses with `merge base changed since the review`; other decisions never read the base. An Org file on disk that does not round-trip refuses (malformed imported content). |
| `tine-graph-features::retained` census writers under a page host (STEP3 §7, R6) | Only with a running page host (none in production until lane 3b's switch). Each writer keeps today's input rule (Q3), checked under its final reservation. A refusing writer (sync-copy and duplicate-day fold, sync-copy trash, VCS-marker and live-conflict resolve, an interrupted title completion) meets a hosted page with unsaved input as today's changed-on-disk refusal (an honest edit in this instance races the writer as an external editor would) and writes nothing; a refusal names each page by its current spelling (a host key is opaque, REVIEW-3a5). A flush-first writer (rename, merge, delete, rescue, journal migration and trash, PDF highlights, the Guide copy) saves that input first, and refuses naming the page only when its save cannot complete (a conflict or a persistent disk error), as today's stuck pages refuse. A writer reserves only the pages it writes (A-R3): a journal migration reserves each still-eligible proposal on its own and skips one whose page cannot be saved, naming it, while the others migrate; a rename reserves the referrers it rewrites, not one it leaves byte-identical (unchanged, or carrying VCS conflict markers). A single page's rename is the host's operation: an Org page that does not round-trip refuses as today; a page the admitted operation could not yet write (disk error) is reported, and the operation finishes once it can. The host admits it only while the planning view and config are current, else the rename replans (B2). Its final pages are the referrers whose bytes the rename changes, held buffers included, which the planner may not list: a held referrer the rename leaves byte-identical (it already says the new name, or carries VCS conflict markers, which the report lists as skipped) is not one of them: its unsaved input or a writer holding it neither blocks nor refuses the rename, and it is not written (A-W1 R3, R4). Proof `crates/tine-graph-features/tests/retained_host.rs`, `crates/tine-store/src/page_host/binding_rename_tests.rs`. |
| `tine-graph-features::journals` migration and trash | A journal disappears or changes after selection; refuse that item and leave other journals intact. |
| `tine-graph-features::pdf` highlight and sidecar | Sidecar or notes bytes change concurrently, a malformed imported sidecar appears, or Org notes cannot round-trip; retain the source and refuse or retry within the bounded loop. |
| `tine-graph-features::config` and `assets` | Config or an asset changes repeatedly while applying a user update; stop before overwriting the external winner. |
| `src-tauri::backup` restore selection | A backup is incomplete, belongs to another graph, fails its manifest hash, loses a source file or a whole snapshot area (`graph/` for schema 3; `journals/`, `pages/` for schema 2; assets), or changes during verification; refuse restore before touching live content. Schema 4 (og-backup-cas): power loss or a disk error tore its manifest, flipped a bit in it that leaves valid JSON (its checksum no longer matches), or tore, removed or damaged a blob it lists (`blobs/<sha256>` that no longer hashes to its name); refuse ("this backup is damaged; pick another snapshot") before the safety snapshot and any graph write (proof `src-tauri/src/backup/restore.rs` `schema_4_restore_is_byte_identical_and_damage_is_refused`, `a_bit_flip_in_a_manifest_path_is_refused`). A failed pre-restore safety snapshot also refuses publication. A schema-2 snapshot made under different `:pages-directory`/`:journals-directory` settings still refuses (it names roots, not paths); schema 3 places text at its recorded graph-relative path and ignores them (ADR 0062). |
| `src-tauri::backup` namespace lock (og-backup-cas) | Concurrent honest instances: another Tine process (a second window's process, master beside og, a CLI) holds the graph's backup lock `backups/<graph-id>.cas/lock` for longer than 30 s. A restore refuses ("backups are busy in another Tine window or process; try the restore again in a moment") before reading the snapshot, the safety snapshot and any graph write; a pre-restore or pre-rewrite snapshot fails and refuses the operation it precedes; a lowered keep-count is applied at the next launch instead. A launch snapshot is skipped, not refused, and logged (`backup-skipped`). An OS error opening or locking the file refuses the same way (`backup-failed:lock:<kind>`). Proof `src-tauri/src/backup.rs` `a_launch_in_another_process_skips_while_a_writer_holds_the_lock`, `a_same_content_blob_write_in_two_processes_is_serialised`. |
| `src-tauri::backup::restore_hosted` restore stop (STEP3 §7, R7) | Only with a running page host. A drained edit whose save fails (disk error) or conflicts (external-editor race), an unretired draft, or a known draft copy whose census could not be synced (power loss, M2), at the restore stop: restore nothing, keep the host, and name the pages (today's restore aborts when its flush fails). The binding's host lock is held throughout (S8), so a writer, a retirement or another restore waits for this outcome. A fresh host that cannot start after the restore (an app-data disk error) is reported, and writers then run as with no host. Proof `src-tauri/src/backup/restore.rs` `a_restore_holds_the_host_gate_until_one_fresh_host_runs`. |
| `src-tauri::data_home::ensure_usable` app-data home (og I1a, master 8e1ea0bfd) | A disk error or a filesystem the user cannot write (a root-owned `~/.local/share`, a read-only mount): the app-data home is relocated for this launch to the first writable fallback (`~/.tine-data`, `$XDG_RUNTIME_DIR/tine-data`, `$TMPDIR/tine-data-<uid>`) and the frontend says where, once, stickily. Only when none is writable does the launch refuse: one sentence naming the `ErrorKind`, exit 1, instead of Tauri's setup panic. |
| `src-tauri::state` graph binding | Two windows try to own overlapping roots, or a queued command carries an old binding generation, or a queued worker holds a binding a reopen has since adopted (`review_p1_late_old_slot_writer_cannot_bypass_adopted_reservation`, `review_p1_a_queued_restore_cannot_enter_after_slot_adoption`); refuse a wrong-graph write. A graph whose page host is still retiring (saving after its window closed) owns its root too: an open of an overlapping root first waits for it outside the registry lock, up to 10 s, telling the window it waits (`graph-open-waiting`), and is refused only past that bound, naming the graph still saving (`host_retirement` `a_stuck_retirement_refuses_an_overlapping_open_naming_the_saving_graph`). The same root is adopted, never waited for. |
| `src::carry` destination day and capture destination (og I1e, og J1, master 7bd793bd0 family) | Sync-service delivery or a journal date-format change leaves two files for today (a duplicate day), and the second one is open path-pinned under today's name. Carry and capture (quick capture, `appendToTodayJournal`, capture into a page) load the file the name resolves to (`admitPageFile`): a second file holding the name with no unsaved input is replaced by it and the write lands in the real file; one WITH unsaved input (the in-scope harm: replacing it would discard that input) refuses before any block moves or is appended, naming both files. A refused capture keeps its text in the capture window. A source page whose replacement the working set declines stops carry the same way. The duplicate-day resolver folds the pair. |
| `src::document::workingSet` name slot (og J1, master 7bd793bd0) | The working set is keyed by name, so a second file holding a name — a duplicate day left by sync-service delivery or a journal date-format change, or a same-named page opened by path — can occupy the slot while it has uncommitted input (an edit, a save in flight, a conflict, an active editor, a component draft). `ensurePageLoaded`, `loadRoutedPage`, `loadSingle`, `loadFeed`, `appendFeed` and `restoreTodayJournalInFeed` then return a typed `PageLoadRefusal` instead of replacing it, and nothing is published under the requested name: the journals feed keeps its previous window (on first load it shows why in place) and fills in place once the holder is replaceable (`whenPageReplaceable`), a sidebar item says why and retries, a route shows why. Replacing would discard that input; publishing would show, and save edits into, the wrong file. The PDF-notes refresh declined for an active editor, a move or a draft pin waits the same way instead of being dropped. Consumption is guarded by `src/refusedReplacement.guard.test.ts`. |
| `src::persistence` frontend save gate | A page is tombstoned, conflicted, held as the source of a cross-page move, or the graph switch still has pending writes; retain the editor buffer and refuse the unsafe completion. An alias draft already appended to its owner page is retried by replacing that landed tail, never by appending again; when the owner's tail no longer matches what landed (an external editor or sync client changed the owner in between), refuse with `conflict` (grouped path: `alias-owner-busy`) and keep the draft (og 22a, L13). A watcher observation of a page with unsaved edits still raises a disk-changed conflict unconditionally; an existing disk-changed conflict is lifted when the file provably returns to the baseline the editor loaded (an external editor's temp+rename or a mid-delivery sync gap): the frozen edit is re-armed and saved against the baseline (`applyObservedDivergence`, og I1c, master c68c0b6e7). |

Watcher reconciliation distinguishes a successful page read, an intentionally
excluded nonregular or escaped path, and a failed read. A successful read can
advance the file baseline only when its bytes match the observed revision.
Intentional exclusions never enter the page cache. A failed re-read after
hashing leaves the old cached page in place, lists the path as unreadable, and
keeps it eligible for the next scan even if its metadata does not change.
Deletion still forgets the cached page; a timestamp-only touch updates its
observed time without reparsing. Tests cover these neighboring outcomes in
`watch.rs` and `tests/watch.rs`.

The watcher drops events under `.git/`, `.stfolder/` and other noise
directories before reconciling. A burst of more than 32 changed paths is
reconciled as one full diff and published as one revision; the desktop adapter
forwards more than 32 page changes in one publication as a single
`graph-changed-bulk` event. Returning focus asks for `rescan_graph_now`, whose
completion is signalled only after the dispatch thread has emitted every change
up to the rescanned revision. None of these paths writes: a reload never
replaces a page holding an unsaved draft, and the frontend's "always ask"
policy only holds clean changes it would otherwise apply silently. Tests cover
these in `watch.rs`, `tests/watch.rs` and `src-tauri/src/watcher.rs`.

Full runtime graph-text scans with unchanged configuration enumerate metadata outside the page/save writer.
They validate the cache generation and publication revision after acquiring the
writer, retrying an observation crossed by an own edit or another publication.
Only changed or racy paths are sorted for reconciliation; unchanged files retain
their known revisions. Config changes enumerate and reparse under the writer.
Failed-load recovery retains its synchronous full parse followed by writer-ordered
reconciliation. Applying detected changes remains writer-ordered, including
racy rehashes, unreadable subtree retention and ordinary external publication.

Search page rows read authored properties through the shared `page_facets`
producer for selected owners rather than initializing graph-wide query facts.
Exact page-name query constraints select every physical owner from the existing
name claimant and page-position maps; simple, advanced and IR execution use the
same evaluator and page-fact producer. OR and negation remain conservative;
property coercions and used-as-tag retain their global facts. This changes no
stored index, cache, checkpoint format, query membership or save protocol.

Review rule: a new refusal must identify a reachable scenario involving an honest
local user, sync or external editor. Source scans cannot prove reachability;
the reviewer traces the path and records the scenario here before accepting it.

| `transaction.rs::check_orphan_asset::AssetReferenced` | 1 | The published graph still references the asset: an external-editor/sync reference arrived after the orphan listing, or another page uses a file the user just dropped from one block (GH #623). Retain the asset; `tine-graph-features::assets::trash_asset` returns it as the normal `TrashOutcome::Referenced` result (never an error), and the `trash_asset` command reports `referenced`. A partial reference inventory reports an IO failure rather than granting trash. |

| `transaction.rs::check_orphan_asset::InvalidTarget` | 1 | An orphan-asset action is given a page/config/trash target; refuse without touching it. Ordinary trash remains available for intentional page or PDF artifact removal. |

Launch config metadata and its read failure come from the same bounded read. A second successful read cannot clear the failure while leaving directories taken from the earlier fallback. A repaired config is applied by the existing watched refresh or a reopen.

### Query publication (OG-R3C2)

`Store::publish(PublishDest::{GraphSite, QueryLeaf, External}, emit)` is the one
publication door; its graph-site and query-leaf arms share one stage/commit.
A query leaf is `published-queries/<portable-folder>/`; the shared discovery
predicate excludes the whole directory from pages, watching and graph backups.
Review (`query_publication_destination`) creates nothing, reports collisions and
suggests a free suffix. Commit uses the explicitly chosen name, never reallocates.
Create uses no-replace installation. Replace retires the current leaf into
`logseq/.tine-trash/conflicts/<stamp>__previous-publish/previous`, and reports that
path even on success. It preserves whatever legitimate directory arrived between
review and commit; a late install winner remains untouched. Files and directory
entries are synced before install; recovery directory entries are synced before
installing the new leaf. Windows retains the existing directory-sync limitation.
An interruption may leave an unpublished hidden stage; it never exposes a partial
leaf. After retirement the previous leaf remains in recovery, even if installation
has not happened. Callers inspect output/recovery on any post-rename I/O failure.

`tine-graph-features::publish_query::publication_assets` takes names from
`tine_core::Corpus::asset_refs` (the orphan detector's collector), reads each
through `Store::file_id(Area::Assets)` + `Store::read`, and bounds reads during
copying. Its caller supplies the cumulative budget and a warning collection. Query exports use one Rust default of 1 GiB, optionally
replaced by the device-local Settings limit. `TooLarge` becomes typed
`AssetBudgetExceeded` / IPC `assetBudget`; no leaf or recovery is touched on that
refusal. A missing asset is a visible warning. Live/CLI limits remain unchanged.

Refusals defend these in-scope scenarios: imported invalid folder names or output
aliases (invalid destination); an external editor/provider retargeting a stage or
parent (identity mismatch); an honest concurrent instance winning a destination
(create collision); disk failures during emit/sync/rename; and an imported local
asset unexpectedly exceeding the device's chosen disk-work budget. Successful
replacement reporting preserves and reveals concurrent content, rather than
claiming an atomic compare-and-replace that the filesystem cannot provide.

The query fingerprint binds request/query rows and held selected documents.
Explicit destination/replace and device-budget choices do not affect source
membership or content and are excluded from that fingerprint. A separate folder
choice is displayed explicitly and commit never recomputes a suffix. This keeps
og's reviewed-source fingerprint while permitting the master's collision UX.

Unit cost: no per-edit publication records or writes; export emits one staged
file per final output file, plus copied asset bytes. Replace renames the old leaf
once into existing recovery (no copying of old bytes). 1/60-block measured artifact
bytes and files are recorded in the OG-R3C2 receipt; transport per edit is zero.

## Case-only page moves (GH #609)

`Transaction::move_file` accepts a destination spelling that resolves to its
unique source, but refuses a separately listed destination entry even when it
is a hard link to the same inode (external-editor/sync-delivery collision).
The feature layer uses the ordinary RenamePage transaction for filename,
explicit title, references, namespace descendants and config home spelling.
Identity stays normalized; there is no second page entity for a casing change.

Case-only moves first try the existing atomic no-replace primitive on Linux,
Windows, macOS, Android and iOS. Only an already-exists refusal permits an
alias check: the requested destination spelling must be absent from the
directory listing and resolve to the same file as the source (`same-file`:
Unix device/inode, Windows volume serial/file index). For that unique folded
alias, one plain `std::fs::rename` uses the atomic-write path's platform rename
(rename(2) / MoveFileExW with REPLACE_EXISTING). A separately listed entry,
including a hardlinked twin, stays an ordinary collision. After a case move,
a fresh directory listing must contain the destination spelling and omit the
source spelling (NFC/NFD normalization is accepted); failure uses transaction
undo. These checks defend external-editor/sync-delivery collisions and a
filesystem/provider silently retaining the old spelling.

Before the atomic rename, the source is live; after it, original bytes are
live at the new spelling. There is no intermediate Trash-only window or
manual page restoration step. After the existing atomic rewrite, updated
title/reference bytes are live there. Retrying after the rename finishes an
old explicit title in place, through `Transaction::rewrite_refs` with
`TitleRebind::Own` under the same revision guard; `move_file` onto its own
source names one file twice and is refused (`RepeatedFile`). An I/O failure attempts ordinary transaction undo;
failed undo keeps live or recoverable bytes. Changed directories use the
existing sync policy. The no-replace primitive retains Windows write-through;
the folded-alias fallback uses the same replacement/durability policy as
atomic writes. The existing check-to-rename race with honest external writers
and flag-refusal fallback policy remain.

Unit cost: one namespace rename instead of two for case-only moves; no staging
directory, added payload copy or retained record on success. If content changes,
the existing rename old-byte trash copy remains O(page bytes), on both 1-block
and 60-block pages; no sync transport. Process-abort tests exercise before
rename, after rename and after rewrite; Linux fault injection exercises the
folded-alias refusal, plain rename and failed spelling post-check. Actual
folding-filesystem platform runtime proof remains separate. The retained
payload test measures 10 B live plus 10 B old for 1 block and 600 B live plus
600 B old for 60 blocks, with exactly one old-byte copy and zero staging copies.

## Rename publication work (GH #623)

Final publication indexes transaction plans and undo records once and matches
each observed file only against its records. A successful own reference rewrite
whose final bytes equal the guarded output parses the changed document once;
there is no old-document serialization/parse comparison. Failed transactions
and external bytes keep ordinary reconciliation. The revision observations,
publication error reporting, path locks, ordered writes and undo remain the same.
`rename_cost.rs` checks bounded record probes and changed-document parses at the
literal feature/store entry. The frontend matches touched paths against loaded
pages once; it still reloads only clean rewritten pages and discards moved pages.

Unit cost: unchanged full referrer payload per file, one temporary payload file
and two syncs per rewritten referrer, plus the source move's directory sync.
The 1-/60-block referrer fixtures write 22/1,320 bytes respectively, measured by
the temporary-payload counter; no new persisted record or transport bytes.

### Rename reads (GH #623, QF3b)

A rename opens each rewritten referrer four times: the planner's read, the
preflight base-revision stage, the final pre-rename guard inside
`atomic_write_with_check`, and the publication read. The planner names only
the renamed page's own files (`WholeGraph::inventory(InventoryScope::FilesAtOrUnder)`), not the
whole-graph inventory. It queues each unmoved, marker-free referrer with
`Transaction::rewrite_refs` and the text it read; the call reports whether the
rewrite (`tine_core::refs::rename_rewrite`, the one rewriter both the planner
and the store use) changes it, queues nothing for an unchanged non-title
referrer, and keeps a changing rewrite in the queued step. Moved and
marker-bearing referrers are asked of `rename_rewrite` directly, since a move
rewrites inside its own step and a marker-bearing file must not be queued
before the planner's VCS-marker skip. Preflight still stages the file against
the expected revision and
reuses the kept bytes only when the staged bytes are byte-identical to the
prepared old bytes and the filename format is unchanged, otherwise it
recomputes; the read-only Org and
VCS-marker refusals run on the staged bytes either way. A changing reference
rewrite has no separate stage-2 read: the final pre-rename guard compares the
same file with the same baseline and refuses a mismatch as the same conflict
(external-editor or sync race), after writing only a temporary file.
Publication names the page from the bytes it read, and the name index reuses
that entry unless the transaction changed `config.edn`. The own-write stamp
takes its metadata from the publication read's handle; the watcher re-hashes
the file only when a later `symlink_metadata` differs from it, and a racy
stamp is reread on the next poll as for every other file.

Unit cost: unchanged bytes, files and syncs per edit (22/1,320 bytes on the
1-/60-block fixtures, one temporary file and two syncs per referrer); per
rewritten referrer four whole-file opens instead of eight and one metadata
stamp, no preamble open and no own-write re-hash, measured by
`rename_io.rs` cost counters; no persisted record or transport bytes.
