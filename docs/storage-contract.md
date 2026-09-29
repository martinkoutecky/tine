# og plain-file storage contract

The graph files are the durable record. A save compares the caller's revision with
the current file, then publishes whole bytes through a synced temporary file.
A create uses a no-clobber rename; replacing an existing page uses an ordinary
rename after the final guard. Another local writer can
replace that file between the final comparison and rename; mandatory cross-process
locks are outside this plain-file contract. The editor retains unsaved content
on every refusal and offers conflict resolution or retry.

The store discovers regular `.md`, `.markdown`, and `.org` pages (case insensitive
extensions) throughout the graph, except
hidden and reserved folders such as `assets/`, `publish/`, and `node_modules/`.
The graph-relative file path remains the write identity. An ordinary page's
nonempty preamble `title::` (or Org title directive) is its logical name;
without one, the configured filename decoder supplies the name. Name lookup,
the published inventory, references, and direct page reads use that one
effective name. A file named for a logical page wins over a second file that
claims the same name through `title::`. Editing a title rekeys the published
name without moving the file. New page filenames use a reversible, injective
Windows-safe codec; existing noncanonical paths remain pinned.

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
surface is 36 combined operations: 28 `Store` methods and eight `Transaction`
methods. The graph-command boundary guard lives at
`crates/tine-store/tests/graph_command_boundary.rs`; the client path guard is
`crates/tine-store/tests/client_root_boundary.rs`.

`Store::is_graph_ready()` reports initial graph loading without waiting.
`Ok(false)` means graph-wide answers can still block. After `Err(Failed(reason))`,
`page()` can read and parse an existing file, but saves and observed edits do
not publish a graph generation. `whole_graph()` returns the load error. A
successful `scan_refresh()` retries the load and publishes a fresh generation;
the answer becomes `Ok(true)`. `Err(Closed)` is terminal for that store.

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
| `transaction.rs::preflight::InvalidTarget` | 13 | A legitimate caller supplies a malformed path, unsafe page nesting, or unsupported operation (including moving config), an external rename changes target shape, or a caller attempts to save a virtual Guide DTO in a transaction; refuse before any step changes disk. Config moves require live config publication; Guide content needs an explicit graph copy. |
| `transaction.rs::preflight::Twin` | 1 | A sync-created same-name page appears between page creation and commit; refuse the ambiguous create. |
| `transaction.rs::preflight::ReadOnly` | 2 | An Org page no longer round-trips after an external edit; keep its bytes and the unsaved editor proposal. R-VCS-MARKERS: a VCS merge by an external writer left unresolved markers; a rewrite would re-indent them and silently lose a side. Only `SaveBase::ResolvingMarkers` (the Concord resolver) passes, and it first stages the old bytes in `logseq/.tine-trash/conflicts/`. |
| `transaction.rs::preflight::RepeatedFile` | 1 | Two steps of one user operation choose the same unique filename; refuse the ambiguous plan. |
| `transaction.rs::apply::InvalidTarget` | 1 | A generated unique candidate is no longer a valid target after a concurrent change; refuse that candidate. |
| `transaction.rs::apply::RepeatedFile` | 1 | A unique candidate collides with another step after planning; refuse rather than overwrite. |
| `transaction.rs::apply::Undecodable` | 2 | Imported bytes fail UTF-8 validation after staging; remove the stage and keep the original. |
| `transaction.rs::commit::Closed` | 1 | A graph is closed while a queued save waits; refuse its old binding. |
| `transaction.rs::commit::RepeatedFile` | 1 | A multi-step action names one file twice; refuse before any write. |
| `transaction.rs::rewrite::Undecodable` | 1 | Sync makes a referrer invalid UTF-8 before rename rewrite; keep that file and refuse the rename. |
| `transaction.rs::rewrite_move::Undecodable` | 2 | Sync leaves a title-owned move source or its rewritten bytes invalid UTF-8; refuse the rename before moving the file or publishing a new title. |
| `transaction.rs::rewrite::ReadOnly` | 1 | An Org referrer is not round-trip editable; keep its bytes instead of rewriting it. |
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
| `tine-graph-features::conflicts::resolve_duplicate_journal_day` pairing (master 9dc54e4a7) | A duplicate journal day (a date-stem file plus a title-named one, left by a journal date-format change or delivered by sync) folds only a stray of the SAME day into that day's canonical file: a stale review after sync-service delivery, an external rename or a date-format change that re-sorted the day refuses before anything is written or trashed, so the command can never merge two unrelated pages; a Markdown/Org pair of one day (malformed pairing: one body would be rewritten in the other format) is refused and offers no row choices. Past the pairing guard it is the sync-copy fold (`fold_pair`): the same revision guards, Org round-trip firewall and recoverable trash (proof `crates/tine-graph-features/tests/f8_duplicate_journal.rs`). |
| `tine-graph-features::conflicts::union_pre` page-property merge (og C3W W5) | Honest multi-device divergence: both devices set the same page property (or an Org drawer line) to different values and the user resolves with the page properties on "both (merge)". The copy's other pre-block lines, list members and free text are kept; a value that cannot be kept twice refuses the resolve naming the key, before anything is written or trashed, so the user chooses mine or theirs (proof `crates/tine-graph-features/tests/c3w_union_pre.rs`). |
| `tine-graph-features::pages::rename_page_after_inventory` unreadable page (og C3W W2) | Malformed imported or sync-delivered content (non-UTF-8, over the size or outline-depth cap) in a page the rename must move or rewrite: the rename refuses naming the file, before anything is written, instead of skipping it while rewriting its referrers (proof `crates/tine-graph-features/tests/c3w_rename_unreadable.rs`). |
| `tine-store::restore` source, destination and recovery | A selected snapshot source changes type or length, a live/recovery path is retargeted, or an external writer creates the destination; refuse publication and retain displaced bytes in recovery. A whole-graph restore (og-B, ADR 0062) takes graph text only as `Area::Graph` inside the graph-text scope the snapshot recorded, and a configured-roots restore never takes `Area::Graph`: a malformed or mixed snapshot refuses before any live file moves (proof `crates/tine-store/tests/restore.rs` `graph_restore_refuses_text_outside_its_recorded_scope`). It retires only unlisted live text inside that recorded scope, into `<restore-id>/graph/`, so text a later `:hidden` edit exposed is never displaced by a snapshot that could not have held it. |
| `tine-store::publish` staged site | An external writer retargets output or stage paths or wins the destination name; refuse replacement and retain the previous site. |
| `tine-graph-features::pages` rename, rescue, merge and delete | A referrer carrying VCS conflict markers (an external merge or sync left it mid-conflict) is skipped, not rewritten, and reported (`skipped_conflicted_referrers`; R-VCS-MARKERS). Sync or an external editor changes a revision, creates a twin, occupies a destination, or makes Org non-round-tripping; refuse the affected transaction and retain source bytes. A rescue also refuses a name already carried by a retained non-portable legacy filename (`pages/A:B.md` from OG on Linux/macOS), which OG would load as a second file for that page. |
| `tine-graph-features::conflicts` resolve | A sync conflict winner or copy changes or disappears during resolution, or an Org member is not editable; preserve both sides and require retry. A `"merged"` row whose reviewed Concord-ledger base (`merge_base_rev`) is gone or different at apply time (sync delivery or an honest concurrent instance moved the ledger, or the ledger became unreadable) refuses with `merge base changed since the review` and writes nothing; every other decision never reads the base, so a ledger failure refuses nothing else (ADR 0056). |
| `tine-graph-features::live_conflict` resolve (og 8e) | The file changed after the review (external-editor race, sync-service delivery, an honest concurrent instance), including a file that reappeared after an `absent` review: refuse with `live conflict changed on disk`, write nothing, and the resolver refreshes its review; the draft stays in the editor and in the draft store. A `"merged"` row whose reviewed ledger base is gone or different refuses with `merge base changed since the review`; other decisions never read the base. An Org file on disk that does not round-trip refuses (malformed imported content). |
| `tine-graph-features::journals` migration and trash | A journal disappears or changes after selection; refuse that item and leave other journals intact. |
| `tine-graph-features::pdf` highlight and sidecar | Sidecar or notes bytes change concurrently, a malformed imported sidecar appears, or Org notes cannot round-trip; retain the source and refuse or retry within the bounded loop. |
| `tine-graph-features::config` and `assets` | Config or an asset changes repeatedly while applying a user update; stop before overwriting the external winner. |
| `src-tauri::backup` restore selection | A backup is incomplete, belongs to another graph, fails its manifest hash, loses a source file or a whole snapshot area (`graph/` for schema 3; `journals/`, `pages/` for schema 2; assets), or changes during verification; refuse restore before touching live content. A failed pre-restore safety snapshot also refuses publication. A schema-2 snapshot made under different `:pages-directory`/`:journals-directory` settings still refuses (it names roots, not paths); schema 3 places text at its recorded graph-relative path and ignores them (ADR 0062). |
| `src-tauri::data_home::ensure_usable` app-data home (og I1a, master 8e1ea0bfd) | A disk error or a filesystem the user cannot write (a root-owned `~/.local/share`, a read-only mount): the app-data home is relocated for this launch to the first writable fallback (`~/.tine-data`, `$XDG_RUNTIME_DIR/tine-data`, `$TMPDIR/tine-data-<uid>`) and the frontend says where, once, stickily. Only when none is writable does the launch refuse: one sentence naming the `ErrorKind`, exit 1, instead of Tauri's setup panic. |
| `src-tauri::state` graph binding | Two windows try to own overlapping roots, or a queued command carries an old binding generation; refuse a wrong-graph write. |
| `src::carry` destination day (og I1e, master 7bd793bd0 family) | Sync-service delivery or a journal date-format change leaves two files for today (a duplicate day), and the second one is open path-pinned under today's name: carry refuses before any block moves, naming both files, instead of moving the tasks into a file the journals feed does not show for today. A source or today page whose replacement the working set declines stops carry the same way. The duplicate-day resolver folds the pair. |
| `src::persistence` frontend save gate | A page is tombstoned, conflicted, held as the source of a cross-page move, or the graph switch still has pending writes; retain the editor buffer and refuse the unsafe completion. An alias draft already appended to its owner page is retried by replacing that landed tail, never by appending again; when the owner's tail no longer matches what landed (an external editor or sync client changed the owner in between), refuse with `conflict` (grouped path: `alias-owner-busy`) and keep the draft (og 22a, L13). A watcher-observed disk-changed conflict is raised only when the file's revision differs from the editor's baseline (or a loaded file is gone), and is lifted when the file provably returns to that baseline (an external editor's temp+rename or a mid-delivery sync gap): the frozen edit is re-armed and saved against the baseline (`applyObservedDivergence`, og I1c, master c68c0b6e7). |

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

Review rule: a new refusal must identify a reachable scenario involving an honest
local user, sync or external editor. Source scans cannot prove reachability;
the reviewer traces the path and records the scenario here before accepting it.
