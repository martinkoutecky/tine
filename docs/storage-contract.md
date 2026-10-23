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
remain a window the Store writer cannot exclude. Both the unused-asset inventory
and the trash guard include block and preamble references from physical page
files set aside by duplicate-day journal presentation. These files use the same
bounded parser and asset-reference collector as indexed pages; a failed read or
analysis refuses the inventory and trash, naming the file. Only set-aside files
are read and parsed again; they are not added to backlinks or query indexes.

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
`TxOutcome::NotCommitted`. A transaction carries rollback failures in
`undo_failed` and omitted final-state files in `publication_errors`, both as
graph-relative locations, and its caller tells the user which files need
inspection. Page saves are the page host's single-file renames (STEP3 §3.2),
which have no multi-file rollback; a failed one keeps the input in custody.

A held `WholeGraph` view does not wait for later writers. Acquiring the first
view with `whole_graph()` can wait for the initial parse; with a launch
checkpoint (ADR 0070) it answers from the checkpoint until the launch diff ends.
`whole_graph_reconciled()` never serves the checkpoint: it waits for the launch
diff, for graph-wide destructive operations and for a one-shot lookup that
missed in the checkpoint (a cold-start `tine://` link, GH #181). The public operation
surface is 47 operations: the methods of `Store` and `Transaction` plus every
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
published, published absent, or whose identity is unknown (B1) has no row,
with one exception (amended 2026-10-11, PERF-FIXES D4): a page open's hold
of a file that holds the bytes its installed row was derived from starts
with those bytes indexed as the owner's. The row stays and the open
publishes nothing. The host's Open read comes later: if the file changed in
between, that read's publication replaces the row, and until it applies the
row shows the file's previous state, as for any external change not yet
reconciled. A file that changed behind the index before the hold is held
unindexed until the owner publishes.
Every ownership change (hold, release, respelling, owner absence, a first
publication that fails) is one transition under the cache lock: it settles
every row whose authority it changes, the key's spellings and the colliding
entries that become unknown, with their error rows, and the writer section
that ran it publishes a snapshot when it settled any, so a view acquired
after it never shows the old row. A release rereads the files it withheld,
and a released page's file unless its installed row is the revision the
watcher last observed there, unchanged since; a change the watcher observed
while the page was held always rereads (GH #623: a rename releases every
referrer).
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

Every graph window binding runs a page host (step 3b P2b,
`src-tauri/src/graph.rs` `start_host`): the window's page edits, moves,
renames and deletions go through it, and its crash drafts live under
`<app data>/drafts-v2/<graph id>`. One process runs one host per graph: a
graph opened again routes to the window already bound to it
(`load_graph_for_label` → `route_bound_root`). On desktop one app identity
runs one process (`tauri_plugin_single_instance`; on Linux it needs a D-Bus
session bus, and without one a second process starts); stable and Beta are two
identities with separate app data, so separate drafts, and their page saves
meet the base-revision guard like any other concurrent writer. No draft lock
exists. A host that cannot start fails the open and the Store closes; the
one such failure is a platform that names no app data directory (`start_host`).
Scenario: crash or power loss; with nowhere to write crash drafts, the
window's unsaved input could not survive one. Draft I/O failures (a full or
failing app-data disk) never fail the start: the graph opens with draft I/O
down and `draft_status` says why. Unit cost: none beyond the drafts below; sessions, the orphan
flag, the retirement map, a caller's reply slot and a running rename's order
are memory only.

**Rename order (option 2, SPEC-s3 s3.2).** While Tine runs, a host rename
publishes its destination first, then its rewritten referrers, then its
source's deletion. A referrer's save waits until the destination's own save
has published durably (directory sync) at the operation's version, and the
deletion waits for every other page (`page_host/order.rs`). Scenario: a disk
error (a full disk, an I/O error) keeps failing the destination's save; the
old file and every `[[Old]]` link keep resolving, and the rename finishes
once the write succeeds. The order is memory only:
- A stop saves nothing gated; the operation's draft carries those pages.
- A launch completes the rename from its drafts, with no order. Across a
  crash only no loss and completion hold. Accepted effect: a crash while the
  destination's write is also failing can leave the page with no file until
  the write succeeds; its bytes are in the trash or the drafts.
- No chaining: an operation over any page of a running order waits until
  that order ends. A deletion answers `Waiting`. A rename waits up to
  `OPERATION_WAIT` (10 s) and is then refused `Busy`, changing nothing.
  A retained writer (§7: a merge, a namespace or spelling move) waits for
  its reservation, holding no lock, as for a busy page (REVIEW-3b-P2b R2);
  writers over other pages are not held up. Its wait over a rename still
  finishing is bounded (M6): `OPERATION_WAIT` after the writer started, one
  deadline across its rediscoveries and flushes, it is refused `Busy`
  ("Another rename of these pages is still finishing; try again in a
  moment.") with nothing reserved. A wait that ends while the host is asking it
  takes that ask's outcome, waiting for it up to `ASK_GRACE` (1 s): an ask
  that admits is the caller's answer; one that does not admit is dropped,
  and the caller is refused `Busy`, never told `Uncertain` about an
  operation that will not run (CI 38436208770; proof `driver_tests.rs`
  `an_unadmitted_ask_across_the_deadline_is_withdrawn_not_late`,
  `an_answer_given_across_the_deadline_reaches_the_caller`). A rename
  still finishing is a running
  order, or a rename's operation from its admission: before its order
  installs (its draft still being made durable) and, for a
  references-only rename (the source has no file), through its operation
  until its pages are handed back. A deletion and every other busy page are
  waited for, unbounded. Every wait and check of the reservation (the
  fence, the input check after a rediscovery, and the flush, which a rename
  admitted meanwhile can take over) carries that deadline (REVIEW-3b-P2b-2
  N1), and a rename holding any page of the writer's set bounds it, even
  while the flush waits on another page of the set (REVIEW-3b-P2b-3;
  `a_flush_over_a_mixed_set_is_told_busy_when_a_rename_holds_its_clean_page`).
  A retained writer waiting on a rename gets an answer within
  `OPERATION_WAIT`, except while the host is inside a disk operation; a
  stuck disk call delays it until that call returns. Data safety is
  unaffected. Accepted residual: REVIEW-3b-P2b-3 N1-entry (acquiring the
  host's state mutex, at entry or after a condition wait, is not bounded;
  `reserve` and the retained writers retire in the next storage step, when
  the page host becomes the only page writer). A host that cannot answer, at entry or during any wait (its driver
  ended, its state is poisoned, or it is down), also answers `Busy`, never
  a panic, with every page it had fenced released (a poisoned host keeps
  no state to release them from or to serve them; proof
  `binding_order_tests.rs` `a_flush_handed_to_a_rename_is_told_busy_after_the_bound`,
  `a_writer_entering_a_poisoned_host_is_told_busy`,
  `a_writer_entering_an_ended_driver_is_told_busy`,
  `a_host_down_after_the_fence_refuses_busy_and_releases_the_pages`; shape
  guard `the_reserve_path_has_no_unchecked_lock_or_bare_wait`).
  Scenario: the destination's save keeps failing (disk error), the
  operation's draft keeps failing (app-data disk error), or a referrer is
  conflicted (an external editor or sync delivery changed it).
- A Discard of the destination before its witness cancels the rename in
  memory: the source takes its disk bytes again and is never trashed, and
  newer input typed into it stays. The caller hears `Superseded`.
- A caller's wait ends `Uncertain`, never success, when its window session
  changes or the host fails, or when the operation makes no progress for
  `OPERATION_WAIT` (no answer, then no further page published), or when an
  ask in flight at its deadline has not finished within `ASK_GRACE`; the
  operation still completes once it can. The bound is on a stall, not on
  the operation: a rename that keeps publishing its referrers is waited for
  however many it has (a thousand on hosted Windows outlast 10 s; Beta had
  no bound). Proof `binding_rename_tests.rs`
  `a_long_rename_that_keeps_publishing_is_waited_for`.
  Every caller exit removes its reply slot.
- The pages an operation applies for its caller (a single page's rename or
  a deletion) save without the edit debounce (400 ms, 1 s at most): the
  caller waits for their publication. A failed save's backoff still
  applies. A window Move is an ordinary edit and keeps the debounce. A
  window Submit skips it: the window already waited for the end of the
  typing burst (`client.ts` `SEND_DEBOUNCE_MS`, `SEND_MAX_WAIT_MS`), so the
  typing wait has one owner (PERF-FIXES D1). Proof
  `progress_tests.rs`
  `a_caller_operations_pages_skip_the_debounce_but_not_a_failed_saves_backoff`,
  `a_window_move_keeps_the_debounce_and_a_submit_does_not`,
  `a_window_moves_pages_publish_400_ms_after_their_last_change`,
  `a_window_submit_saves_at_once_without_the_host_debounce`,
  `rename_cost.rs` `a_small_rename_is_not_floored_by_the_edit_debounce`.
Unit cost: none (memory only). Proof
`crates/tine-store/src/page_host/binding_order_tests.rs`,
`crates/tine-graph-features/tests/retained_over_running_rename.rs`, the model's
`orderHolds` (`storage-s3.qnt` s3.2, mutants MDF, MRD, MPR).

**Ordinary edit unit cost (I-25, step 3b P2b).** An ordinary `page_submit`
applies in memory with no draft record (a submit is not at risk; only a
conflict, a failed save or a host operation drafts). One guarded save
publishes it: one temporary file of exactly the page's bytes (10/541 B on the
1-/60-block fixtures), renamed over the page, and two syncs (temp file and
directory). No app-data file, no directory walk, no transport bytes. Submits
answered before the save starts coalesce into that one save (five back-to-back
submits on the 60-block page: 1 file, 541 B, 2 syncs). Measured by
`crates/tine-store/tests/host_edit_cost.rs` with every host write, byte and
sync counted at the `HostIo` seam. A save also costs 2 whole-file reads and
3 parses (1 of the old source) per edit.

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

**Window close (Q-TS5, J6).** The window sends no `page_close` for a page
while the host owes it a save: a failed attempt (the notice), a conflict, or
any version the host took from this window that has not published. That last
is the client's one publication-debt predicate (`HostClient.owes`): a taken
version no true `page_wait` answer has discharged. Only true discharges it.
A false answer under a terminal notice (conflict, failed save, failed index
publication) keeps the page while the notice stands. Any other false (a failed
call, a host that ended) keeps it and asks again after a pause. A new session
or the client's retirement ends the debt with the page; retirement settles a
pending wait false and asks no more. Every let-go path consults the predicate:
the close, working-set eviction (`unsettledPages`), the recovery panel and
close prompt (`unsavedDrafts`, state "Saving") and reload blocking
(`hostBlocksReload`). Natively, `page_wait` answers false once the driver has
ended, the host is down or its state is poisoned. A down host dropped its
pages, which would otherwise read as owing nothing. Proofs
`src/document/host/client.test.ts` "keeps a page open while a version the host
took from it is unpublished (J6)", "lets go when the session ends (J6)", the
B1/R3 cases beside them, `src/document/host/wiring.publicationDebt.test.ts`
(R1), the census `src/document/host/publicationDebt.guard.test.ts`, and
`crates/tine-store/src/page_host/binding_tests.rs`
`wait_published_refuses_once_the_driver_ended_or_the_host_is_down`.

**Trash custody durability (A4, RO3 (a), Martin 2026-10-11).** A deletion
makes the trash *name* durable: the custody marker, then the move, then a sync
of `pages/` and each existing ancestor up to the graph root, then the source
directory. It does not sync the trashed file's data: Tine's own saves synced
those bytes already. Residual R-UNFLUSHED: bytes another program wrote into
the page without flushing them may be lost at a power cut after Tine deletes
the page, even though the trash name survives. That includes an unflushed
rewrite with the same text as Tine's last save (a sync tool redelivering it,
an editor re-saving an unchanged buffer): the other program's write replaced
Tine's bytes; they had no durability from
their writer either (ModelFs `cuts()`; proof
`page_host::production_tests::trashed_external_payload_is_named_durably_before_the_deletion_witness`).

**Trash collisions (K6).** A deletion's move into the trash never replaces
what is there. Only that no-replace move's own target-exists is a collision.
The host retires the unused custody marker and retries under a fresh name.
Any other `AlreadyExists`, such as a file where the trash directory belongs,
is an ordinary I/O failure. The third consecutive collision on one save job
fails the save, so a persistent collision goes through the failure bound
(backoff, then the save-error notice) and no retry loop runs uncounted.
Scenario: sync delivery or an external tool keeps the name or the directory
taken. Proof `page_host::production_tests`
`a_file_at_the_trash_directory_fails_the_delete_instead_of_colliding` and
`page_host::mutation_tests` `a_persistent_trash_collision_fails_the_save_on_the_third`.
Unit cost: none (one memory counter per job).

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
replies with `draft_status` (`{unavailable, unreadable, unsaved}`; none with
no host). `unsaved` is the persistent indicator at each open (SPEC-s2 §4.11,
STEP3 §9): every page whose text is not on disk, by spelling, with
`recovered` (launch installed it from a draft and it is still at that
version), `conflict` (changed on disk) and `failing` (its save fails). The
window names recovered pages in a sticky notice ("Recovered unsaved edits …
Tine is saving them") and any other held page with why, as an error when one
needs the user. Wire-only; unit cost: none (nothing persisted). Proof
`crates/tine-store/src/page_host/binding_tests.rs`
`s9_draft_status_names_recovered_and_unsaved_pages`,
`src/graphDraftStatus.test.tsx`.
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
never exits over the input (B-QA): it keeps the host and opens a window on
each graph still saving, which adopts its host and names the pages not on
disk and why (`unsaved` above; `b_qa_*`), before the closed window goes. The exit
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

## I-8 refusal scenarios

Each row below is keyed by the source file, owning function and refusal family.
The count is the number of production constructions or save-adapter outcomes;
the source guard fails when an unreviewed site appears. These scenarios involve
ordinary sync, external editors, user actions, malformed files, or graph lifecycle.

| Key | Count | In-scope scenario and required response |
|---|---:|---|
| `transaction.rs::refuse_marker_rewrite::ReadOnly` | 1 | R-VCS-MARKERS for reference rewrites and moves that rewrite references (og 21a, master a8fd4230d): an external VCS merge or a sync service left a referrer mid-conflict; rewriting `[[Old]]` inside it would edit sides of a merge the user has not adjudicated. The rename skips such referrers (byte-identical, a moved one moves verbatim) and reports them; this refuses any other caller's rewrite. A rewrite that changes nothing passes. |
| `transaction.rs::path::InvalidTarget` | 3 | A sync update changes a path into a symlink or invalid area while an operation resolves it; refuse access outside the approved graph root. Only a target proved outside is refused. One that cannot be inspected now (Windows refuses to open a page for a moment while a save replaces it) is re-probed for under 50 ms (in `canonical_existing_ancestor`, so the submit's OS-path check and the write-target check both do), an ordinary entry resolving through its folder; a link that does not resolve is refused at once. A target still uninspectable fails as that I/O error: the submit answers `failed` and the window keeps the text unsaved, rather than refusing it as outside (REG-WIN-WRITE-TARGET-TRANSIENT, proof `crates/tine-store/src/model.rs` `write_target_probe_tests`). |
| `transaction.rs::twin::Twin` | 2 | Sync introduces an alternate-extension or indexed second physical file for the same page name or journal day; refuse a write that could choose the wrong one. The disk check also catches a twin delivered since the index, and exempts a move's own source. |
| `transaction.rs::preflight::InvalidTarget` | 10 | A legitimate caller supplies a malformed path or unsupported operation (including moving config), or an external rename changes target shape; refuse before any step changes disk. Config moves require live config publication. |
| `transaction.rs::page_save_target::InvalidTarget` | 3 | A page save names a non-page path or unsafe page nesting, or a caller attempts to save a virtual Guide DTO; refuse before any step changes disk. Guide content needs an explicit graph copy. |
| `transaction.rs::page_create_checks::Twin` | 1 | A sync-created same-name page appears between page creation and commit; refuse the ambiguous create. |
| `transaction.rs::page_open_checks::InvalidTarget` | 1 | A page host open (STEP3 §2) names a non-page path, as a malformed link or an external rename leaves it; refuse before the page is registered or anything changes disk. |
| `transaction.rs::serialize_page::ReadOnly` | 2 | An Org page no longer round-trips after an external edit; keep its bytes and the unsaved editor proposal. R-VCS-MARKERS: a VCS merge by an external writer left unresolved markers; a rewrite would re-indent them and silently lose a side. Only `SaveBase::ResolvingMarkers` (the Concord resolver) passes, and it first stages the old bytes in `logseq/.tine-trash/conflicts/`. |
| `transaction.rs::unreadable_owner::UnreadableOwner` | 1 | R-CREATE-UNREADABLE-OWNER (master 69e0a885ddf9 + 69525c055f0b, GH #543): sync delivery, an interrupted external write or malformed imported Markdown/Org leaves a page file Tine cannot read (not UTF-8 where its name lives, an oversized preamble, a permission error) or a directory it cannot list, so the file's page name is unknown; creating a page of a name it could carry would give that name two files once the bad file is repaired. Refuse a name-only creation (`CreateNew` save, `create`, `create_unique` of an ordinary page) only for a name that entry could be (`Graph::unreadable_page_could_own`: its file-name name; its preamble name when that still decodes; otherwise a name its bytes, folded as page names are, contain; any name for an unlistable directory or a file unreadable now; nothing for a FIFO, socket, device, journal or vanished entry), and name the entry. Every other creation, save, move and the watcher proceed. |
| `transaction.rs::preflight::RepeatedFile` | 1 | A unique filename collides with a fixed step of the same operation; refuse the ambiguous plan. Earlier unique allocations are reserved and later unique steps choose the next free candidate. |
| `transaction.rs::apply::InvalidTarget` | 1 | A generated unique candidate is no longer a valid target after a concurrent change; refuse that candidate. |
| `transaction.rs::apply::RepeatedFile` | 1 | A unique candidate collides with another step after planning; refuse rather than overwrite. |
| `transaction.rs::apply::Undecodable` | 2 | Imported bytes fail UTF-8 validation after staging; remove the stage and keep the original. |
| `transaction.rs::preflight_names::Closed` | 1 | A graph is closed while a queued save or plan waits; refuse its old binding. |
| `transaction.rs::preflight_names::RepeatedFile` | 1 | A multi-step action names one file twice; refuse before any write or operation. |
| `transaction.rs::plan::InvalidTarget` | 6 | A plugin or frontend defect passes a non-page effect, a read-only expectation or an unsupported shape to a page host planner (the existing host supports one move), or imported oversized page bytes cannot be staged within the existing parse cap; refuse before publication and keep every source. A byte-preserving non-page-to-page move stages only the page destination; its source cleanup belongs in a separate guarded transaction (§3.1c C2, P3 re-review rescue correction). |
| `transaction.rs::rewrite::Undecodable` | 1 | Sync makes a referrer invalid UTF-8 before rename rewrite; keep that file and refuse the rename. |
| `transaction.rs::rewrite_move::Undecodable` | 2 | Sync leaves a title-owned move source or its rewritten bytes invalid UTF-8; refuse the rename before moving the file or publishing a new title. |
| `transaction.rs::refuse_read_only_org::ReadOnly` | 1 | An Org referrer is not round-trip editable (malformed imported Org); keep its bytes instead of rewriting it, whether preflight computes the rewrite or reuses one prepared by the rename planner (GH #623). |
| `transaction.rs::rewrite_move::ReadOnly` | 1 | An imported Org page named by `#+TITLE:` or a `:title:` drawer is not round-trip editable; refuse the rename rather than rebind its title in bytes Tine cannot reproduce. |
| `transaction.rs::content_refusal::Undecodable` | 1 | An existing or imported page has invalid UTF-8; refuse the write without reporting a transient I/O failure. |
| `transaction.rs::content_refusal::InvalidTarget` | 1 | Existing or serialized page content exceeds the byte or nesting parse bound; refuse while retaining unsaved edits. |
| `transaction.rs::page_content_bytes<'a>::InvalidTarget` | 1 | A raw page stream exceeds its cap; refuse before creating an unreadable page. Shared validation supplies staged bytes to planning too. |
| `transaction.rs::validate_config_bytes::InvalidTarget` | 1 | A config edit or create names a directory outside the graph; refuse before changing disk. |
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
| `tine-store::page_host` request answers (step 3b, `Refusal`) | A submit or Discard for a page the host does not hold (`NotHeld`; a frontend or plugin defect, or a request from an earlier window state): refuse and change nothing. A Discard whose disk read fails (`ReadFailed`, SPEC-s2 §4.11 "discard failed"; a disk error), including the read of a running rename's source that the Discard would revert: refuse whole, change neither page, keep the rename's order (proof `crates/tine-store/src/page_host/command_tests.rs` `an_answer_that_did_not_take_its_request_names_why`, `a_discard_whose_partner_source_read_fails_changes_nothing`). A move whose versions are no longer current (`Stale`; newer input in this window raced it): refuse both pages. A move whose receiver draft cannot be written (`DraftFailed`; a full disk or an app-data disk error): refuse and change nothing. |
| `tine-store::page_host` operations (step 3b P2b, `RenameRefusal`, `PageOperation`) | An operation's draft cannot be written (a full disk or an app-data disk error): the rename, merge move or deletion answers `DraftFailed`, nothing changed on disk or in memory, no order is installed and no reply slot is left (proof `binding_order_tests.rs` `a_rename_whose_draft_fails_changes_nothing`). An operation over a running rename's pages is `Waiting` or `Busy` (above, "Rename order"). A page the rename would rewrite has unsaved input (`Unsaved`; an honest edit in this window races it) or cannot be rewritten (`Unwritable`; malformed imported Org or non-UTF-8 bytes): refuse before anything changes. |
| `tine-store::page_host` trash custody (A4, `HostIo::trash_sync`) | A deleted page's trash name cannot be made durable: the payload's stat or a trash-chain directory sync fails (a disk error, R-STORAGE-ERROR). The next saves of that page fail while its custody is owed, so a recreation never lands before the deletion's trash name is durable. The third consecutive failure escapes: the save goes ahead with a sticky error, and the unfinished custody stays as debt that launch retries. Discard never waits for custody (proof `crates/tine-store/src/page_host/production_tests.rs` `unsyncable_trash_escapes_after_three_failures_and_never_blocks_discard`, `scheduler.rs` `failing_custody_fails_saves_until_the_three_failure_escape`). |
| `tine-store::page_host::delete` target (step 3b, `page_host/binding.rs`) | Web content, a plugin or a frontend defect passes `page_delete` a path that is not a page file of this graph (outside the root, not a graph text file): refuse before the deletion is registered or anything is written, as `page_open_checks::InvalidTarget` does for an open (proof `crates/tine-graph-features/tests/page_delete_missing.rs` `a_delete_outside_the_graph_refuses_without_writes`). |
| `tine-store::page_host::apply` (host-only-writer P2, `HostOp`, `ApplyRefusal`) | A feature's planned write runs as one host operation, admitted only if every page still holds the bytes the feature planned from (`Expected`). A page that no longer does, or that the host cannot read (`Changed`; the window, an external editor or a sync service changed the file since the plan): refuse whole, change nothing, and the feature replans (`retry_on_conflict`). An absent destination's name slot is rechecked on the driver under the operation's path locks, using the transaction's claimant/twin and unreadable-owner rules (raw create checks the filename, CreateNew save also checks the DTO name, move exempts its own source). An arriving claim refuses `Changed` before the inner `Host::apply`; no operation or draft is recorded. This binding refusal is an s33 pre-admission stutter: the one-path-per-page model, its `opApply` conformance and pins are unchanged. C3 is proven by the real Guide API regressions for an arriving twin and an unreadable owner. Unsaved input on a page (`Unsaved`; an honest edit in this window races the writer): a `Refuse` writer refuses naming the page; a `Flush` writer saves it first and refuses only when that save cannot complete (a conflict or a persistent disk error). Store reads answer saved bytes, so a `Flush` feature saves its pages' input before it plans (`PageHost::flush_input`, `tine-graph-features::retained::operation`): a page that exists only as typed input would otherwise plan as absent. The save-first wait is a reservation's flush wait: past `OPERATION_WAIT` only a page a rename holds refuses `Busy`. A path that is not a page of this graph or reaches outside it through a symlink, a page named twice, or a move that is not one (`Refused`; a plugin or frontend defect, or an imported graph holding a symlinked folder): refuse before the operation is admitted or anything is written (pages named earlier in the operation may already be registered; registration writes nothing). A path that only collides with a registered entry, unproved, is `Changed` (an external rename or case-variant twin since the plan). Not admitted within `OPERATION_WAIT` (`Busy`; another operation, reservation, rename or path lock holds its pages, as above); after a flush the admission gets a whole `OPERATION_WAIT` again. A draft that cannot be written (`DraftFailed`; a full disk or an app-data disk error): nothing changed. After admission it answers as a rename does (`Unwritten`, `Superseded`, `Uncertain`). A deleted page keeps its bytes in the graph trash, as every host deletion does (proof `crates/tine-store/src/page_host/binding_apply_tests.rs`). |
| `tine-store::watch` reconcile | A graph closes, its root disappears, or an external config edit changes directory layout; stop reconciliation rather than publishing a false view. |
| `tine-store::watch` OS watch setup (og 22a) | The OS refuses live file notifications for the graph (inotify's per-user watch limit is spent by other apps, a network or FUSE mount without events, a root replaced while open); degrade to a 3-second poll that runs the same reconcile, retry the live watch every cycle, and report the refusal and its restoration (`Subscription::observe_watch_status`, `graph-watch-refused` / `graph-watch-restored`, flight event `watcher.refused`). The graph is never silently stale (I-9). |
| `tine-store::transaction` undo of a replaced file | Disk full, a disk error, or a crash while rolling back a failed multi-file save (C3 L07): undo stages the file's pre-transaction bytes as a `tx-old` conflict-trash copy before it withdraws the transaction's bytes, and removes that copy once the old bytes are back live. If the copy cannot be written it withdraws nothing, leaves the transaction's bytes live and reports the file in `undo_failed` (proof `c3s_content_loss.rs`). |
| `tine-graph-features::conflicts::resolve_sync_conflict` pairing | Sync-service delivery (Dropbox, Syncthing, Seafile) of a conflict copy whose base page is missing or whose page name holds parentheses (C3 L01): a copy is merged only into the page its own name shadows (`sync_copy_winner`); any other pairing is refused before anything is written or trashed, and the copy stays for its real reconcile (proof `crates/tine-graph-features/tests/c3s_sync_copy_base.rs`). |
| `src-tauri::capture_target::resolve` native capture into a named graph (og H1b) | An Android photo or voice memo that finishes after the window switched graphs has its only copy in the app cache: it is written into the graph it was started in, named by the frontend, through the ordinary asset transaction, never into the new graph, and the user is told where it went. The caller must still be a live bound window. A named root that is not a known graph on this device is refused before anything is written (scenario: plugin or web content in the WebView naming an arbitrary directory; an honest capture always names a graph this device opened). A root that no longer resolves to a folder (moved or deleted by sync or the user mid-capture) is refused by path acquisition. In both cases the capture stays in the cache and the failure is reported (proof `src-tauri/src/capture_target.rs` tests, `src/components/Block.mobileCapture.test.tsx` "stopped after a graph switch"). |
| `tine-graph-features::conflicts::resolve_duplicate_journal_day` pairing (master 9dc54e4a7) | A duplicate journal day (a date-stem file plus a title-named one, left by a journal date-format change or delivered by sync) folds only a stray of the SAME day into that day's canonical file: a stale review after sync-service delivery, an external rename or a date-format change that re-sorted the day refuses before anything is written or trashed, so the command can never merge two unrelated pages; a Markdown/Org pair of one day (malformed pairing: one body would be rewritten in the other format) is refused and offers no row choices. Past the pairing guard it is the sync-copy fold (`fold_pair`): the same revision guards, Org round-trip firewall and recoverable trash (proof `crates/tine-graph-features/tests/f8_duplicate_journal.rs`). With a page host, the duplicate-day fold is one host operation (P3): winner write and stray delete, with `moved = (winner, stray)`. After a crash, relaunch completes the stray's deletion. |
| `tine-graph-features::conflicts::union_pre` page-property merge (og C3W W5) | Honest multi-device divergence: both devices set the same page property (or an Org drawer line) to different values and the user resolves with the page properties on "both (merge)". The copy's other pre-block lines, list members and free text are kept; a value that cannot be kept twice refuses the resolve naming the key, before anything is written or trashed, so the user chooses mine or theirs (proof `crates/tine-graph-features/tests/c3w_union_pre.rs`). |
| `tine-graph-features::pages::rename_page_after_inventory` unreadable page (og C3W W2) | Malformed imported or sync-delivered content (non-UTF-8, over the size or outline-depth cap) in a page the rename must move or rewrite: the rename refuses naming the file, before anything is written, instead of skipping it while rewriting its referrers (proof `crates/tine-graph-features/tests/c3w_rename_unreadable.rs`). |
| `tine-store::restore` source, destination and recovery | A selected snapshot source changes type or length, a live/recovery path is retargeted, or an external writer creates the destination; refuse publication and retain displaced bytes in recovery. A whole-graph restore (og-B, ADR 0062) takes graph text only as `Area::Graph` inside the graph-text scope the snapshot recorded, and a configured-roots restore never takes `Area::Graph`: a malformed or mixed snapshot refuses before any live file moves (proof `crates/tine-store/tests/restore.rs` `graph_restore_refuses_text_outside_its_recorded_scope`). It retires only unlisted live text inside that recorded scope, into `<restore-id>/graph/`, so text a later `:hidden` edit exposed is never displaced by a snapshot that could not have held it. Power loss after the replacement is published must not lose a retired file (its only copy can be an external editor's edit newer than the safety snapshot): the recovery root, every directory created for it, and each retired entry are synced before the replacement is published, and a real directory-sync failure (disk error) stops the restore with its recovery locations reported (proof `crates/tine-store/tests/restore.rs` `restore_makes_the_recovery_tree_durable_before_publishing_replacements`). Transactions that move, trash or create into a directory they create sync every created entry the same way (`directory_durability::create_dir_all_durable`). |
| `tine-store::publish` staged site | An external writer retargets output or stage paths or wins the destination name; refuse replacement and retain the previous site. |
| `tine-graph-features::pages` rename, rescue, merge and delete | A referrer carrying VCS conflict markers (an external merge or sync left it mid-conflict) is skipped, not rewritten, and reported (`skipped_conflicted_referrers`; R-VCS-MARKERS). Sync or an external editor changes a revision, creates a twin, occupies a destination, or makes Org non-round-tripping; refuse the affected transaction or host operation and retain source bytes. A rescue also refuses a name already carried by a retained non-portable legacy filename (`pages/A:B.md` from OG on Linux/macOS), which OG would load as a second file for that page. Delete refuses a stale path if an external editor changed its title or replaced its claimant; confirmed file absence succeeds without writes (GH #620). A rename commits every step only in the view and config it was planned in (`Transaction::expect_view`): sync or an external editor delivering a referrer or `config.edn` between plan and commit makes the attempt replan, within the existing four attempts, then `WouldBlock` (A-W1 B2; proof `crates/tine-graph-features/src/pages_snapshot_tests.rs`). |
| `tine-graph-features::conflicts` resolve | A sync conflict winner or copy changes or disappears during resolution, or an Org member is not editable; preserve both sides and require retry. A sync copy that changes after the winner's operation applied is kept. The winner holds the reviewed merge, and a retry re-reviews. Sync-service delivery (Syncthing or Dropbox) after the review can trigger this refusal. The copy's transaction guard now runs after the host operation. A `"merged"` row whose reviewed Concord-ledger base (`merge_base_rev`) is gone or different at apply time (sync delivery or an honest concurrent instance moved the ledger, or the ledger became unreadable) refuses with `merge base changed since the review` and writes nothing; every other decision never reads the base, so a ledger failure refuses nothing else (ADR 0056). |
| `tine-graph-features::live_conflict` merge (og 8e; read-only: the window submits the merged page through its page host with the reviewed disk revision, STEP3 R6) | The file changed after the review (external-editor race, sync-service delivery, an honest concurrent instance), including a file that reappeared after an `absent` review: the merge refuses with `live conflict changed on disk` and the resolver refreshes its review; a submit whose reviewed revision is no longer the disk's leaves the page conflicted, and its save's base-revision guard writes nothing. The draft stays in the editor and in the page host's drafts. A `"merged"` row whose reviewed ledger base (named by its sha256) is no longer retained refuses with `merge base changed since the review`; other decisions never read the base. An Org file on disk that does not round-trip refuses (malformed imported content). |
| `tine-graph-features::retained` census writers under a page host (STEP3 §7, R6) | With the binding's running page host (every graph window binding runs one). Each writer keeps today's input rule (Q3), checked at host admission or under its final reservation. A refusing writer (sync-copy and duplicate-day fold, VCS-marker and live-conflict resolve, an interrupted title completion) meets a hosted page with unsaved input as today's changed-on-disk refusal (an honest edit in this instance races the writer as an external editor would) and writes nothing; a refusal names each page by its current spelling (a host key is opaque, REVIEW-3a5). A flush-first writer (rename, merge, delete, rescue, journal migration and trash, PDF highlights, the Guide copy) saves that input first, and refuses naming the page only when its save cannot complete (a conflict or a persistent disk error), as today's stuck pages refuse. A writer fences only the pages it writes (A-R3): a journal migration runs each still-eligible proposal on its own and skips one whose page cannot be saved, naming it, while the others migrate; a rename reserves the referrers it rewrites, not one it leaves byte-identical (unchanged, or carrying VCS conflict markers). Moving a journal to trash and merging two pages (P2), and rescuing a stray file to a page, migrating a journal filename and creating a Guide page (P3), are host operations (`HostOp`). Each builds the same transaction as its no-host path, and calls `Transaction::plan` inside a read-only planner. Move metadata comes only from a move_file step or a feature's explicit post-plan declaration of content transfer (merge, fold, PDF migration), validated against the effects at admission; effect count never establishes a move. Creation checks remain those of the original transaction step, independent of that declaration. Unique allocations reserve names chosen by preceding unique steps in the transaction. Planning shares commit's preflight (target, claimant, unreadable owner, twin, nesting, markers, round-trip firewalls and serialization), stages the old bytes, preserves `Why`, and publishes nothing. Non-page effects stay outside the operation; `Store::as_page` decides page-ness. A byte-preserving move from a non-page source plans only its page destination using the move's staged bytes and slot checks; other non-page steps are refused by `plan`. `Store::page_bytes` has been removed. A page changed since the plan is refused `Changed`, and the writer replans (row above). A rescue and a migration move bytes unchanged. The target is written over no file, then the source is deleted from the planned bytes, in a rename's order. A crash between them completes at relaunch. A non-page rescue source (including a sync-conflict file) instead stays outside the operation: its destination is staged unchanged by the original move preflight, including invalid UTF-8, without raw-create content validation, and its source is trashed only after Applied, guarded by the read revision. Uncertain keeps the source; a crash before cleanup keeps both copies. A target that appeared since the plan (sync delivery, an external editor or a saved window draft) is never overwritten. The rescue refuses `AlreadyExists` when its claimant check sees it. The migration skips the proposal with its transaction refusal. Each proposal is its own operation; one that cannot run is skipped with its reason while the others migrate. A Guide page is created over no file (`Expected::Absent`). A file or folder at its path, including one that appears after the plan through an external editor or sync delivery, is reported skipped and never written. Guide assets stay their own transactions. The sync-copy and duplicate-day folds are host operations too (P3). A sync copy is trashed after the fold's operation applied, guarded by the bytes it read. A copy changed since is kept, and the resolve refuses as changed on disk. The sync-copy trash fences nothing: the copy is not a page. Writing PDF highlights is a host operation too (P3). The sidecar EDN holds the highlight data and is not a page. It is saved first in its own guarded transaction outside the planner. The read-only planner reads the annotation page bytes before checking that the saved sidecar revision still holds; if another saver changed it, it returns to the outer loop to redo the merge and sidecar save, including on a page replan. A later saver that replaces both sidecar and page invalidates the staged page base at planning or admission. Then the annotation page and any migrated legacy-key page's deletion run as one host operation, planned from the page bytes and ordered as a move when migrating. The operation names and admits the page under the spelling the plan resolves, including an encoded file name. A refused or unfinished page operation returns its error with the sidecar saved. A subsequent successful save can add missing annotation blocks from highlights retained in the sidecar. If a deletion reaches the sidecar but the process crashes before recording the page operation, a later save may no longer have the baseline needed to remove that annotation; automatic deletion catch-up is not guaranteed. The legacy sidecar and deleted crops are retired best effort after the operation applies. Resolving a file's VCS conflict markers is a host operation too (P3). It is planned from the marker bytes with the resolving parse. A copy of those bytes is staged into the conflict trash outside the read-only planner, before the operation. A refused operation leaves one harmless extra copy there; the file is unchanged. A page changed since the plan is refused under the `Refuse` policy and never overwritten. With no host, these writers keep the old transaction paths. A single page's rename is the host's operation: an Org page that does not round-trip refuses as today; a page the admitted operation could not yet write (disk error) is reported, and the operation finishes once it can. The host admits it only while the planning view and config are current, else the rename replans (B2). Its final pages are the referrers whose bytes the rename changes, held buffers included, which the planner may not list: a held referrer the rename leaves byte-identical (it already says the new name, or carries VCS conflict markers, which the report lists as skipped) is not one of them: its unsaved input or a writer holding it neither blocks nor refuses the rename, and it is not written (A-W1 R3, R4). A writer over a page of a rename still finishing (its destination's save or its draft keeps failing on a disk error, or a referrer is conflicted by an external editor or sync delivery) is refused `WouldBlock` "Another rename of these pages is still finishing; try again in a moment." once `OPERATION_WAIT` has passed since it started, with nothing written or reserved; other busy pages are waited for (M6; proof `crates/tine-graph-features/tests/retained_over_running_rename.rs`, `binding_order_tests.rs` `a_retained_writer_over_an_unapplied_rename_is_told_busy_after_the_bound`). Proof `crates/tine-graph-features/tests/retained_host.rs`, `crates/tine-store/src/page_host/binding_rename_tests.rs`, `crates/tine-graph-features/tests/host_moves.rs`, `crates/tine-graph-features/tests/guide_host_op.rs`, `crates/tine-graph-features/tests/pdf_host.rs`, `crates/tine-graph-features/tests/host_op_conflicts.rs`. |
| `tine-graph-features::journals` migration and trash | A journal disappears or changes after selection; refuse that item and leave other journals intact. |
| `tine-graph-features::conflicts::trash_copy` non-page writer | A sync copy changed since the review (Syncthing or Dropbox delivery after the review): refuse and keep the copy. For a fold, this guard runs after the winner's host operation applied. The winner holds the reviewed merge, and a retry re-reviews. Sync copies are not pages and need no page fence. |
| `tine-graph-features::pdf` highlight and sidecar | Sidecar or notes bytes change concurrently, a malformed imported sidecar appears, or Org notes cannot round-trip; retain the source and refuse or retry within the bounded loop. With a page host the annotation page is a host operation. A page changed since the plan is refused `Changed`, and the writer replans (the window, an external editor or a sync service changed it). If the saved sidecar changed, the page replan returns to the outer merge and guarded sidecar-save loop; it does not reuse stale merged highlights. |
| `tine-graph-features::config` and `assets` | Config or an asset changes repeatedly while applying a user update; stop before overwriting the external winner. |
| `src-tauri::backup` restore selection | A backup is incomplete, belongs to another graph, fails its manifest hash, loses a source file or a whole snapshot area (`graph/` for schema 3; `journals/`, `pages/` for schema 2; assets), or changes during verification; refuse restore before touching live content. A failed pre-restore safety snapshot also refuses publication. A schema-2 snapshot made under different `:pages-directory`/`:journals-directory` settings still refuses (it names roots, not paths); schema 3 places text at its recorded graph-relative path and ignores them (ADR 0062). |
| `src-tauri::backup::restore_hosted` restore stop (STEP3 §7, R7) | Only with a running page host. A drained edit whose save fails (disk error) or conflicts (external-editor race), an unretired draft, or a known draft copy whose census could not be synced (power loss, M2), at the restore stop: restore nothing, keep the host, and name the pages (today's restore aborts when its flush fails). The binding's host lock is held throughout (S8), so a writer, a retirement or another restore waits for this outcome. A fresh host that cannot start after the restore (an app-data disk error) is reported, and writers then run as with no host. Proof `src-tauri/src/backup/restore.rs` `a_restore_holds_the_host_gate_until_one_fresh_host_runs`. |
| `src-tauri::portable` portable Windows app data (GH #719) | A disk error or a filesystem the user cannot write (a write-protected stick, a read-only share): a portable `tine-data` folder that cannot take a file stops the launch in `portable::preflight()` (first in `main` and `run`), before the debug log, CLI, settings or Tauri write anything, with a native error naming the folder and saying to move Tine somewhere writable. Refusal, not recovery: a fallback to the installed profile would split app state across two homes, and the next launch from a writable folder would silently leave the newer half behind. The graph is never touched. Portable identity (manual updates) holds either way. Portable mode never reads, moves or deletes existing `%APPDATA%` / `%LOCALAPPDATA%` state. |
| `src-tauri::data_home::ensure_usable` app-data home (og I1a, master 8e1ea0bfd) | A disk error or a filesystem the user cannot write (a root-owned `~/.local/share`, a read-only mount): the app-data home is relocated for this launch to the first writable fallback (`~/.tine-data`, `$XDG_RUNTIME_DIR/tine-data`, `$TMPDIR/tine-data-<uid>`) and the frontend says where, once, stickily. Only when none is writable does the launch refuse: one sentence naming the `ErrorKind`, exit 1, instead of Tauri's setup panic. |
| `src-tauri::state` graph binding | Two windows try to own overlapping roots, or a queued command carries an old binding generation, or a queued worker holds a binding a reopen has since adopted (`review_p1_late_old_slot_writer_cannot_bypass_adopted_reservation`, `review_p1_a_queued_restore_cannot_enter_after_slot_adoption`); refuse a wrong-graph write. A graph whose page host is still retiring (saving after its window closed) owns its root too: an open of an overlapping root first waits for it outside the registry lock, up to 10 s, telling the window it waits (`graph-open-waiting`), and is refused only past that bound, naming the graph still saving (`host_retirement` `a_stuck_retirement_refuses_an_overlapping_open_naming_the_saving_graph`). The same root is adopted, never waited for. |
| `src::carry` destination day and capture destination (og I1e, og J1, master 7bd793bd0 family) | Sync-service delivery or a journal date-format change leaves two files for today (a duplicate day), and the second one is open path-pinned under today's name. Carry and capture (quick capture, `appendToTodayJournal`, capture into a page) load the file the name resolves to (`admitPageFile`): a second file holding the name with no unsaved input is replaced by it and the write lands in the real file; one WITH unsaved input (the in-scope harm: replacing it would discard that input) refuses before any block moves or is appended, naming both files. A refused capture keeps its text in the capture window. A source page whose replacement the working set declines stops carry the same way. The duplicate-day resolver folds the pair. |
| `src::document::workingSet` name slot (og J1, master 7bd793bd0) | The working set is keyed by name, so a second file holding a name — a duplicate day left by sync-service delivery or a journal date-format change, or a same-named page opened by path — can occupy the slot while it has uncommitted input (an edit, a save in flight, a conflict, an active editor, a component draft). `ensurePageLoaded`, `loadRoutedPage`, `loadSingle`, `loadFeed`, `appendFeed` and `restoreTodayJournalInFeed` then return a typed `PageLoadRefusal` instead of replacing it, and nothing is published under the requested name: the journals feed keeps its previous window (on first load it shows why in place) and fills in place once the holder is replaceable (`whenPageReplaceable`), a sidebar item says why and retries, a route shows why. Replacing would discard that input; publishing would show, and save edits into, the wrong file. The PDF-notes refresh declined for an active editor, a move or a draft pin waits the same way instead of being dropped. Consumption is guarded by `src/refusedReplacement.guard.test.ts`. |
| `src::document::host` delete input (audit L9-06) | From sending a delete until its answer, the page accepts no input through the shared `pageWritable` store boundary; the editor and sheet affordances read the same gate. A refused, waiting, pending, uncertain, superseded or thrown delete unlocks the page with its text intact. Only an applied delete retires it. Proof `src/document/deleteInput.test.ts`, `src/components/Block.deleteInput.test.tsx`. |
| `src::document::host` window writes (step 3b P2b: the page host is the only writer of page edits; the frontend save engine is gone) | An alias draft (a page with no file whose name became an alias, L13) lands in its owner only while the owner is open in the host, idle and not conflicted, and no editor is open on it (an IME composition is DOM-local input the store does not hold yet); otherwise it stays, unsaved and listed. The landing is recorded when the owner's live text receives the copy, before submission and publication are proved; if either fails, the draft stays recoverable. A retry after an unproved publication or a mid-save edit replaces the landed copy instead of appending the draft again; when the owner's text no longer holds that copy (an external editor or sync client changed the owner meanwhile) the landing refuses, says so, and keeps the draft (proof `src/document/save/aliasDraft.test.ts`, `src/document/save/aliasDraftRetry.test.ts`; audit L9-05). A watcher observation of a page with unsaved input conflicts in the host; the conflict lifts when the file provably returns to the baseline the editor loaded (an external editor's temp+rename or a mid-delivery sync gap), and the frozen edit is saved against it (og I1c, master c68c0b6e7; proof `src/document/conflictLift.test.ts`). |

The writer census distinguishes read-only operation planners from reservation
and restore write closures. Transaction construction and `plan` are read-only;
a commit or non-page write in a planner fails the guard, including through a
helper or a writer exemption. Imported helpers and aliases are resolved across
modules; an unresolved free call receiving a Store/Transaction parameter fails
the planner guard. The negative fixtures cover inline and named
planners, direct, imported and helper-mediated commits, and keep protected write closures
and edit-kind checks meaningful.

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
`rename_cost.rs` checks changed-document parses at the literal feature/store
entry. A rename now publishes through the page host (step 3b), so its linear
publication work is the host's (REVIEW-3b-P2b R5): per page publication, the
reply slots and running orders examined (`Order::published`); per index
publication, one probe; and per wake of the caller waiting for the operation's
witnesses, a count comparison plus the host's stuck pages (conflicted, or past
their third failed save, kept by `Progress::reconcile`), never a rescan of every
answered page. `rename_cost.rs` bounds these `host_publication_probes` at
12 per written page for 8 and 64 referrers (measured about 5). A host step's own
scheduling follows what changed (M4): `Progress::reconcile` visits the pages
whose buffer or live logical draft changed since the last step and the pages
its new events name; the draft index is kept per page; the operation worker
locks only the pages its current draft file names. Proved envelope: a
rename's host work through its cleanup is linear in its pages for each counted
primitive (reconcile visits, draft records, save, draft and retirement
candidates, worker keys, Book) at 100, 200 and 400 referrers against fixed
per-page constants, and an edit's reconcile, draft-index and worker work does
not grow with the pages the window holds (`rename_cost.rs`
`rename_host_work_is_linear`, `host_edit_cost.rs`
`an_edit_among_many_held_pages_visits_what_changed`). Named remaining
dimensions, walked per step and not bounded by these proofs: the window's
subscribed pages S (retirement candidates per poll, quiet notices per
collection), queued requests Q (checked per retirement candidate), surfaced
draft errors E (checked per visited page), and a released retained writer's
handover keys, rechecked per step until each is observed (follow-up F1). The frontend matches touched paths against loaded
pages once; it still reloads only clean rewritten pages and discards moved pages.

Unit cost (step 3b P2b, a single page's rename through the page host, Q-P2b-2;
re-measured after RENAME-COST 2026-10-11): the 1-/60-block referrer fixtures
write 4 files and 300/4,137 bytes with 16 syncs: the operation draft, the
referrer, the destination page and the source's custody marker (17 before
RO3 (a), which stopped syncing the trashed source's data). The
referrer's old and new payloads are each written once into the operation
draft, and the new one into the page (three payload copies). Each extra page
of an operation adds one file (its page) and two syncs (the file and its
directory). The operation draft retires whole after the operation's last
publication: one unlink and one draft-directory sync, with no per-page draft
copy, unless a page of the operation changes its draft state alone first
(then the remaining records are copied out before that change). Measured by
`rename_cost.rs` with every host write, byte and sync counted at the `HostIo`
seam (`page_host/production.rs`, `cost_counters`). No transport bytes.

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
The watcher's re-reads of an operation's own writes (the racy re-read,
storage spec §5.4, and on Windows also the echo event's path-scoped diff)
are an accepted cost, on the watcher thread after publication: once the
watcher settles they are at most the files the operation wrote plus a small
constant (measured on 31 written pages: one read each on Linux, two on
Windows). They are linear in the files written and do not grow with the
graph (D-10). An echo of a page the host still holds is forwarded to the
host, which re-reads it as an owed observation (§5); that read has the same
cause and the same bound, whatever thread makes it. The rename cost tests
measure through an acknowledged drain (`PageHost::quiesce_for_tests`,
`watch/test_drain.rs`; REVIEW-3b-P2b-2 N3, REVIEW-3b-P2b-3), not through quiet
counters: each drain request writes its own sentinel, named by the request's
id, after the operation's writes; it reaches the watcher behind their
notifications (one watch delivers in order; checked on Linux and hosted
Windows), and the watcher acknowledges the id the sentinel names only after
the cycle that drained them has reconciled, store writer included. A late or
duplicate notification of an older sentinel acknowledges only that older
request. A due racy follow-up runs first, and then the host has no owed
observation, page, job or request left. A lost sentinel fails the test; a
watcher blocked on the writer is never declared drained, nor is a drain
answered by an older sentinel
(`n3_a_watcher_blocked_on_the_writer_is_not_drained`,
`n3_an_older_sentinel_does_not_acknowledge_a_newer_drain`,
`n3_a_late_notification_of_a_completed_drain_acknowledges_only_it`). The
tests bill publication only for the operation's own reads, and bound the
settled re-reads separately: the watcher's (`watcher_reads`,
`watcher_stamps_by_path`) and the host's re-reads of forwarded echoes
(`observation_reads`, at most the files written plus a small constant).
The same holds for files someone else delivered (a sync tool, an external
editor; REVIEW-3b-P2b R4(b)): the racy rehash is linear in the files changed
or delivered within the racy window, not in the graph. An aged unrelated file
is not reread, and each delivered file is opened a bounded number of times per
delivery (measured on Linux: its reconciliation's whole read and two preamble
opens, then one racy rehash once its stamp leaves the 2 s window; one by-path
stamp), after which the settled watcher reads nothing. `racy_delivery_cost.rs`
pins four opens per delivered file plus a constant, the same count on 60 and
600 pages, and none once settled. A delivery that leaves the whole graph fresh
(a first sync) rehashes each of its files once. An edit's publication reads and
stamps the edited page only, on aged and freshly delivered graphs of 60 and 600
pages alike (`host_edit_cost.rs`, R4(a)).

Unit cost: unchanged bytes, files and syncs per edit (22/1,320 bytes on the
1-/60-block fixtures, one temporary file and two syncs per referrer); per
rewritten referrer four whole-file opens instead of eight and one metadata
stamp, no preamble open and no own-write re-hash, measured by
`rename_io.rs` cost counters; no persisted record or transport bytes.
