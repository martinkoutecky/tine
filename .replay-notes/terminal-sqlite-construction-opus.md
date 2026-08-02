# Terminal SQLite construction (design chunk 1)

Branch `perf/terminal-sqlite-construction`, base `272a3726`
(`style(storage): apply canonical Rust formatting`).

## Exact cause

Fresh activation authored one correct terminal detached engine, published the
immutable bootstrap parts, and then rebuilt the device-local SQLite projection
from scratch by walking the accepted index and, for every accepted sequence:

1. `RebuildSource::from_inactive_bootstrap` -> `store.load_bootstrap_part`
   reloaded that part's manifest and every payload object out of the immutable
   publication, revalidated it through `PreparedBatch::new` /
   `AcceptedBatchEvent::from_validated` (manifest re-encode and digest, semantic
   effect decode/re-encode/digest, and one encode of every object to recompute
   `retained_bytes`), and re-authenticated it against the engine;
2. `materialize_inactive_bootstrap_event_bulk` materialized that part's affected
   pages at *that part's* accepted root, and
   `attach_authenticated_reference_catalog` derived that part's reference rows;
3. `apply_candidate_with_materialization_and_stats` wrote them through ordinary
   event DML, which for every replacement page first ran `delete_page` cleanup
   (an existence probe, an FTS owner scan, and ten DELETE statements) and
   `validate_preserved_page_metadata` (one SELECT per page).

Every one of those passes exists to reconstruct, incrementally, a state the
same process had already computed exactly once.

Separately, and only visible once the above was measured: every materialized row
insert re-prepared its SQL. A graph-sized build runs the same dozen statements
once per page, block, and facet, so statement preparation alone was well over
half of the terminal row insert time (2.14 s -> 0.92 s at 2,000 pages).

## Commits

| Commit | Subject |
| --- | --- |
| `bcfb2b72` | `perf(oplog): build fresh-activation SQLite from terminal accepted state` |
| `8594c9d3` | `perf(storage): reuse prepared statements for materialized row writes` |
| `d96b6295` | `test(oplog): prove terminal SQLite construction against clean archive replay` |

## Diff

`272a3726..d96b6295`, 10 files, +2237 / -305.

| File | What changed |
| --- | --- |
| `tine-core/src/oplog/import.rs` | `TerminalBootstrapConstructionMaterial` (move-only, non-`Clone`, non-serializable, `Drop`-removed); retain each authored part's `AcceptedBatchEvent`; relocate the existing operation spool out of the working directory before it is removed; `take_terminal_construction_material`. |
| `tine-core/src/oplog/sqlite.rs` | `AcceptedBatchEvent::from_authored_bootstrap_part`; `validate_terminal_construction_material`; `SqliteFrontier::terminal_stream` / `seed_terminal_rows` / `seed_terminal_chunk`; shared `finish_fresh_candidate`; `collect_reference_source_rows` shared by per-event and terminal lowering; `build_candidate` / `publish_candidate` split with terminal-refusal fallback; terminal counters and the three interruption cuts. |
| `tine-core/src/oplog/sqlite_materialization.rs` | `TerminalMaterializationChunk` + `lower_terminal_chunk` (same per-page/posting/alias/coverage validation as per-event lowering, no per-event effect); `lower_reference_postings` / `lower_alias_declarations` / `lower_source_coverage` extracted and shared. |
| `tine-core/src/oplog/local_active.rs` | Thread the one-shot material through `InactiveBootstrapRuntimeSession::open`; every reopen/retry path passes `None`. |
| `tine-core/src/sync_runtime.rs` | Take the material once, immediately before `SqliteOpenBuild`; assert the terminal counters in the activation scale receipt. |
| `tine-storage/src/sqlite_materialization.rs` | `begin_terminal_construction_in_open_candidate` / `seed_terminal_chunk_in_open_candidate` / `finish_terminal_construction_in_open_candidate` over the unchanged schema; `insert_reference_posting` / `insert_alias_declaration` extracted; `execute_cached`; `row_digests_by_table` (test-support). |
| `tine-storage/src/sqlite_database.rs` | Narrow candidate-scoped forwarding for the three terminal calls; prepared-statement cache capacity; test-support per-table row digests. |
| `tine-storage/src/lib.rs` | Export the three new physical DTOs. |
| `tine-core/src/oplog/local_active/tests.rs` | `terminal_construction` and `terminal_construction_interruption` suites (+611). |
| `tine-core/src/oplog/exact_external_feed.rs` | One call-site `None`. |

No schema, DDL, wire, version constant, durable format, encoding, cache, pack,
or index changed. Shadow projection, migration backup, and enrollment were not
touched.

## What the uninterrupted path now does

1. During detached authoring, each part's `AcceptedBatchEvent` is built from the
   same prepared bytes and the same accepted evidence the replay path
   reconstructs, via the same constructor. Nothing is asserted: the evidence is
   the detached engine's own.
2. Before the working directory is removed, the existing operation spool file is
   renamed under a fresh random name in the same preparation prefix. It is never
   fsynced or sealed, is named by no durable artifact, and is removed on `Drop`.
   Crash residue is ordinary incomplete-preparation garbage.
3. `sync_runtime` moves the capability out of the preparation exactly once,
   immediately before `SqliteOpenBuild`, and passes it by value into
   `InactiveBootstrapRuntimeSession::open`.
4. `validate_terminal_construction_material` binds it to the archive authority
   before any row is written: workspace, lineage, import id, event count equal to
   both the aggregate part count and the engine's accepted batch count, and for
   every ordinal the aggregate descriptor's batch id / acceptance sequence /
   evidence ordinal, the engine's indexed accepted evidence at that sequence,
   the manifest fingerprint, the event binding digest, and an unbroken
   prior/post accepted-root chain that ends exactly at the engine's terminal
   `AcceptedFrontierRoot`.
5. Inside one candidate transaction under the same `SqliteApplierSlot`:
   - each part is applied with `materialization: None` after
     `authenticate_event_for_engine`, so the complete authenticated
     accepted-prefix rows (`applied_batches`, causal clock nodes, batch map,
     `frontier_documents`, frontier row) are written exactly as before;
   - the engine's authenticated current-path catalog is traversed in bounded
     pages at the terminal accepted frontier, materialized once in chunks of
     `BOOTSTRAP_MATERIALIZATION_CHUNK_PAGES` at that one root, each page bound to
     its catalog row's page id / path / kind, lowered through the same
     `materialized_page_input` rules, and seeded with the terminal reference
     postings, aliases, and coverage collected by the shared
     `collect_reference_source_rows`;
   - the alias bindings are derived from the complete declarations, the
     accepted-prefix `materialization_batches` provenance rows are written, and
     the single `materialization_stamp` is published authenticated to
     `AcceptedFrontierRoot::reference_catalog_root`.
6. `finalize_fresh_bootstrap`, the semantic projection digest, and the
   materialized row digest run exactly as before while the file is still an
   unpublished candidate; then commit, WAL checkpoint, atomic publication,
   reopen, projection checkpoint, and the unchanged
   `VerifiedBootstrapSqliteProjection`.

### The one documented deviation

`materialization_batches` rows for a terminal build carry the digest of the
*empty* `MaterializationChange` this construction actually applied at that
sequence, and leave the optional reference-transition group NULL — the same
shape a page materialization with no catalog transition already writes. This is
deliberate and is the only place a terminal database differs from a replayed one
in a row the schema stores:

- the column is `NOT NULL`, and the honest value for "this build replayed no
  intermediate change at this sequence" is the digest of that empty change,
  computed rather than fabricated;
- the non-NULL variant additionally requires `catalog_change`, the postcard
  encoding of the whole catalog transition, which for a terminal build would be
  a graph-sized blob against a 64 MiB column bound — exactly the unbounded
  in-memory cache the invariant forbids;
- the only production reader is `recorded_digest`, on the duplicate-re-apply
  path, which is additionally gated by `ensure_stamp` at that same sequence and
  so is reachable only for the newest applied batch; the promoted `TailOverlay`
  drain only ever applies sequences strictly greater than the applied count, so
  it never re-applies a bootstrap part.

The authenticated catalog root, coverage digest, and extractor stamp are still
published in `materialization_stamp`, which is what
`authenticated_reference_catalog_root` and every terminal proof read.

## Fail-before / pass-after

There is no failing-then-passing assertion here: this is a performance cut whose
contract is that nothing observable changes. The proof is therefore
differential, and it does fail before the implementation is correct — the
differential suite caught two real divergences while it was being written (an
unsorted terminal chunk, and FTS owner rowids).

Release, same machine, both fixtures of a run set to the same scale so the two
receipts are a repeat rather than a cache-warmth comparison,
`cargo test -p tine-core --release activation_scaled_manual_phase_receipt --
--ignored`:

| Scale | `SqliteOpenBuild` before | after | delta | total before | total after |
| --- | --- | --- | --- | --- | --- |
| 1,000 pages (1003 files, 10002 blocks, 4 parts) | 2633 / 2595 ms | 2108 / 2052 ms | **-20%** | 9540 / 9409 ms | 9278 / 9108 ms |
| 2,000 pages (2003 files, 20002 blocks, 7 parts) | 5660 / 5691 ms | 4445 / 4366 ms | **-22%** | 20942 / 21695 ms | 19655 / 20262 ms |

The two commits contribute unevenly, and the honest split at these scales is:

| 2,000-page configuration | `SqliteOpenBuild` |
| --- | --- |
| base `272a3726` | 5660 / 5691 ms |
| terminal construction only (`bcfb2b72`) | 5683 / 5638 ms |
| terminal construction + statement cache (`8594c9d3`) | 4445 / 4366 ms |

At 2,000 pages the terminal cut alone is a wash: the per-part work it deletes
(physical part reload and revalidation, per-part intermediate materialization,
per-page cleanup DML) is very nearly balanced by materializing the whole page
set at the *terminal* accepted root and reading the terminal reference catalog,
both of which are the largest roots in the run, where the replay path did that
work incrementally against smaller ones. The per-part costs it deletes grow with
part count and with table size, so the balance is expected to move at scale;
that is what the 10,000-page receipt below is for.

Measured phase composition of the 2,000-page terminal build (4.4 s):

| Phase | ms |
| --- | --- |
| authenticated accepted-prefix seed (7 events, ~14k document-map upserts) | 660 |
| terminal row seed | 2570 |
| — one bulk materialization of 2003 pages at the terminal root | 420 |
| — lowering + terminal reference posting lookups | 710 |
| — SQL row/FTS/reference inserts | 920 |
| — alias bindings, provenance, stamp, catalog cursor | ~520 |
| candidate proof scans (`finalize_fresh_bootstrap`, semantic digest, row digest) | 495 |
| commit, WAL checkpoint, publication, reopen, checkpoint proof | ~620 |

That composition is the instrumentation gate the design note asked for, and it
is the reason the win is 22% rather than the note's speculative range: the work
this cut deletes (physical part reload and revalidation, per-part intermediate
materialization, and per-page cleanup DML) was real but smaller than the
indispensable terminal row/FTS/reference construction plus the two full
candidate proof scans, which are unchanged and now dominate.

10,000-page release receipt: see "10,000-page scale" below.

## Commands and results

| Command | Result |
| --- | --- |
| `cargo test -p tine-core terminal_construction` | 8 passed |
| `cargo test -p tine-core -- oplog::sqlite oplog::import oplog::local_active oplog::shadow_projection oplog::enrollment oplog::exact_external_feed` | 354 passed, 1 failed (pre-existing, see limitations), 7 ignored |
| `cargo test -p tine-core sync_runtime::` | 137 passed, 3 ignored |
| `cargo test -p tine-storage` | 130 passed, 1 ignored |
| `cargo fmt --all` | clean |
| `git diff --check` | clean |

`RUST_MIN_STACK=134217728` is set for the `tine-core` runs; see limitations.

## Proof

`oplog::local_active::tests::terminal_construction` builds the same authority
twice — terminal and forced clean archive replay — and compares:

- exact `AcceptedFrontierRoot`, accepted batch count, semantic projection
  digest, every stored accepted semantic effect, and
  `authenticated_reference_catalog_root`;
- every public materialized query surface: `pages` (unfiltered and per kind),
  `page`, `pages_by_path` / `_by_name` / `_by_name_key` / `_by_name_key_and_kind`,
  `blocks_on_page`, `block`, `properties` for every page and block owner,
  `referrers_to` for every page and block, `tasks`, and a quoted FTS `search`
  for every page's logical name;
- a complete per-table row observation of all 15 materialized tables.

Excluded, each for a reason proved inside each database rather than assumed:
`materialization_batches` and `materialization_stamp` (construction provenance,
above), and `search_fts_owners.rowid` (SQLite's insertion-order surrogate — the
authoritative `(entity_type, entity_id, page_id)` triple is compared through
both `search_fts` and the rest of the owner row, and `finalize_fresh_bootstrap`
proves the owner-to-FTS join is exact and total in each database separately).

Both builds additionally run `diagnose_full_integrity` and
`freshly_verify_inactive_bootstrap`, and both are asserted to use exactly one
candidate transaction, one candidate durability barrier, zero ordinary
transactions, and one each of the two final equivalence proofs.

Source matrix (reusing `Fixture` / `local_active_shape_fixtures`):

| Test | Shape |
| --- | --- |
| `..._for_zero_one_and_multipart_shapes` | zero sources, one source, genuine multipart (4096 operations), and `rich_fixture`: nested 80-deep Unicode path, `.markdown`, CRLF, BOM, empty journal, configured pages/journals directories and title formats, same bytes with distinct identity in Markdown and Org |
| `..._for_a_huge_page_split` | one 3000-block page that genuinely splits across parts |
| `..._for_rich_semantic_layout` | aliases, page references, block reference and embed by UUID, properties, tags, TODO/DONE with priority and SCHEDULED, Org drawer properties and Org tags, CRLF+BOM+`.markdown`, empty file, nested journal |
| `..._for_duplicate_uuid_collapse` | the same `id::` claimed by three pages across Markdown and Org |

`oplog::local_active::tests::terminal_construction_interruption`:

| Test | Boundary | Asserted behavior |
| --- | --- | --- |
| `interrupted_terminal_candidate_falls_back_to_archive_replay` | before the candidate transaction commit; after the commit but before the atomic publication | private candidate discarded (no `*candidate*` residue), `terminal_construction_refusals == 1`, `terminal_constructions == 0`, `bootstrap_part_reads == parts`, observations identical to a clean replay |
| `interruption_after_publication_refuses_and_a_restart_rebuilds` | after the atomic publication, before the checkpoint proof | the open refuses outright, so the unproved published file authorizes nothing; a restart *without* the process artifact rebuilds it as `RebuiltPreservingEvidence` and matches a clean replay |
| `forced_rebuild_over_a_terminal_database_replays_the_archive` | forced rebuild over an already-published terminal database | rebuilt from the archive alone, `terminal_constructions == 0`, observations identical |
| `substituted_terminal_material_refuses_and_falls_back` | material from a different preparation | refuses to bind, falls back, observations identical |

The one-shot property is asserted directly: a second
`take_terminal_construction_material` returns `None`.

Counters (`BootstrapSqliteRebuildInstrumentation`), asserted both in the
differential suite and in `sync_runtime::tests::
activation_progress_is_ordered_exact_byte_and_structurally_near_linear`:

- terminal path: `terminal_constructions == 1`, `terminal_materializations == 1`,
  `terminal_pages_materialized == source files`, `bootstrap_part_reads == 0`,
  `bootstrap_object_reads == 0`, `intermediate_page_materializations == 0`,
  `peak_terminal_bulk_pages <= BOOTSTRAP_MATERIALIZATION_CHUNK_PAGES`;
- replay path: `terminal_constructions == 0`, `bootstrap_part_reads == parts`,
  `intermediate_page_materializations == parts`, `terminal_materializations == 0`.

The replay path stays exercised separately and keeps reporting the old work:
the `local_active`, `local_active::bounded_admission`, and
`exact_external_feed` fixtures all open their bootstrap session with `None`, and
every differential test builds a replay database beside the terminal one.

Ordinary post-activation apply is covered end to end by the `sync_runtime`
suite, which now activates through the terminal path in all 137 tests —
including the editor create/edit/move/reorder/delete journeys, promotion,
crash-reopen takeover, and shared-provider behavior — with the graph snapshot
asserted unchanged across `VerifiedLocal`, `LocalActive`, and promotion.

## Preserved invariants

- Source capture, source-protocol construction, operation spooling,
  partitioning, detached authoring, immutable part/history publication,
  migration backup, shadow reconstruction, enrollment, promotion, and every
  durable format are unchanged.
- The retained material is move-only, non-`Clone`, non-serializable, absent
  from every aggregate/history/enrollment/proof byte, never fsynced or sealed,
  removed on drop, and unusable by a new process.
- It is bound to the retained candidate, aggregate, engine-indexed accepted
  evidence, durable history root, and accepted frontier before use; it grants no
  acceptance of its own.
- Every current terminal proof still runs: exact accepted count and frontier
  chain, semantic projection digest, materialized row digest, reference catalog
  root/coverage/stamp, `finalize_fresh_bootstrap`, WAL checkpoint, file and
  directory durability, atomic candidate publication, post-publication reopen,
  and the unchanged typed `VerifiedBootstrapSqliteProjection`.
- No accepted-prefix schema row is omitted: duplicate detection, ancestry, tail
  apply, diagnostics, and restart all read rows the terminal builder writes from
  the same authenticated values the replay path derives.
- Ordinary live apply, promoted rebuild, another-device and cold-process replay,
  corruption fallback, shadow projection, enrollment, and every public
  query/read semantic are unchanged. The prepared-statement cache changes
  statement preparation only, not SQL text, parameters, transaction ownership,
  or row semantics.
- No malicious-third-party hardening was added.

## Fallback behavior

Absent, unbindable, out-of-bounds, or interrupted terminal material never
becomes an error the user sees. `build_candidate_and_publish` discards the
private candidate and rebuilds through the unchanged
`RebuildSource::from_inactive_bootstrap` + `rebuild_stream` path over the same
durable evidence, recording `terminal_construction_refusals`. Only an
interruption *after* the atomic publication refuses outright, because at that
point the file on disk is unproved and no fallback may silently adopt it; the
next open rebuilds it while preserving the evidence.

## Limitations and follow-ups

1. **Peak memory.** The retained events hold each part's semantic effect twice
   (`AcceptedBatchEvent` keeps an authored and an effective copy, which for a
   bootstrap part are byte-identical), for the whole accepted prefix, from
   authoring until the SQLite build. That is bounded by the graph's semantic
   effect size and is a real increase over the replay path's one-part-at-a-time
   peak. Collapsing the two copies would need `AcceptedBatchEvent` itself to
   change and was out of this cut's write set. *Follow-up.*
2. **The remaining SQLite phase is now dominated by work this cut does not
   touch**: the accepted-prefix document-map upserts, the terminal row/FTS
   inserts, and the two full candidate proof scans. Chunk 2 (source-baseline
   shadow deletion) is where the next graph-sized pass disappears. *Expected.*
3. **The operation spool is retained but has no chunk-1 consumer.** It is part
   of the same one-artifact capability the contract names, and chunk 2's
   manifest-intent sink is its consumer. It costs one rename and one unlink.
   *Follow-up (chunk 2).*
4. **Two pre-existing debug-build failures in this worktree**, both reproduced
   on the base commit `272a3726` with the working tree stashed:
   `sync_runtime::tests::editor_parser_authority_matrix_...` and
   `...::new_markdown_and_org_pages_are_born_...` overflow the default 8 MiB
   test stack, and
   `oplog::import::tests::detached_bootstrap_conflicting_abandoned_content_address_fails_closed`
   fails on an abandoned Patricia node the packed-publication work no longer
   leaves behind. The stack overflows are worked around with
   `RUST_MIN_STACK=134217728`; the import failure is left as found.
   *Follow-up, not this cut.*
5. **Prepared-statement caching is a neighbor, not chunk 1.** It lives entirely
   inside the permitted `tine-storage/src/sqlite_materialization.rs` write set
   and changes no behavior, but it also speeds up ordinary live applies and
   promoted rebuilds. It is called out separately (`8594c9d3`) so it can be
   reverted independently if the manager wants chunk 1 in isolation; without it
   the 2,000-page SQLite phase is 5.6 s instead of 4.4 s, i.e. the terminal cut
   alone is roughly break-even at that scale and the measured win comes from the
   two together.
