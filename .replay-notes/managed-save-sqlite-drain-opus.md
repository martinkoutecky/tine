# Managed ordinary save: the SQLite drain's graph-sized work is one catalog decode

Branch `perf/managed-save-sqlite-drain`, base `982a3ef4`.

**Outcome: the assigned hypothesis is falsified, the real cause is measured
exactly, and it is outside the permitted write set.** 92% of the drain's
scaling is one decode of the whole page-catalog CRDT document per save, inside
`ShardedHotEngine`. The two pieces of repeated work that *are* in the write set
are cut, proved, and committed; they do not change the scaling and this note
does not claim they do.

| Commit | Subject |
| --- | --- |
| `b0b2aaac` | `perf(oplog): cut the ordinary SQLite drain's repeated work` |
| `56859dd0` | `test(oplog): bound one ordinary save's SQLite work to its own delta` |

Diff `982a3ef4..56859dd0`: 2 files, +735 / −25. No schema, DDL, wire, version
constant, durable format, archive object, encoding, index, or public query
surface changed.

## The assigned hypothesis is wrong

The dossier attributed the drain's scaling to "SQLite apply plus
reference-catalog stamp: the reference-catalog root and coverage digest span all
sources", and asked for it to be isolated rather than assumed. Isolated, it is
not the cause. At 1,000 pages the entire reference-catalog attach is **0.264 ms**
of a 44.769 ms drain and contributes **+0.04 ms** of the +34.62 ms of scaling.

## Exact cause

Release, local mode, per-save means over 10 warm one-page content saves on an
existing page, `TINE_MANAGED_ORDINARY_BLOCKS_PER_PAGE=10`:

| SQLite drain sub-phase | 100 pages | 1,000 pages | Δ | share of scaling |
| --- | ---: | ---: | ---: | ---: |
| **drain total** | **10.149** | **44.769** | **+34.62** | 100% |
| frontier proof (`authenticate_exact_frontier`) | 0.444 | 0.747 | +0.30 | 0.9% |
| plan required frontier | 0.010 | 0.011 | +0.00 | 0% |
| accepted-event reconstruction | 0.252 | 0.276 | +0.02 | 0.1% |
| **materialize accepted event** | **5.177** | **38.320** | **+33.14** | **95.7%** |
| — event authentication | 0.179 | 0.518 | +0.34 | 1.0% |
| — effect decode/re-encode | 0.001 | 0.001 | +0.00 | 0% |
| — accepted-root authentication | 0.060 | 0.062 | +0.00 | 0% |
| — **materialize one page** | **4.768** | **37.439** | **+32.67** | **94.4%** |
| —— **catalog document load** | **4.056** | **35.759** | **+31.70** | **91.6%** |
| —— catalog shape proof (`get_value`) | 0.036 | 0.301 | +0.27 | 0.8% |
| —— page document load | 0.512 | 1.164 | +0.65 | 1.9% |
| —— block home document loads | 0.023 | 0.053 | +0.03 | 0.1% |
| —— page assembly | 0.088 | 0.096 | +0.00 | 0% |
| reference-catalog attach | 0.223 | 0.264 | +0.04 | 0.1% |
| — of which posting lookups | 0.171 | 0.203 | +0.03 | 0.1% |
| **physical apply** | **3.988** | **5.075** | **+1.09** | **3.1%** |
| — prologue (claim, frontier, lowering of the batch) | 0.085 | 0.126 | +0.04 | 0.1% |
| — lowering of the materialization | 0.228 | 0.240 | +0.01 | 0% |
| — preflight | 0.125 | 0.193 | +0.07 | 0.2% |
| — frontier-transition validation | 0.052 | 0.110 | +0.06 | 0.2% |
| — SQL transaction (`apply`) | 1.892 | 2.743 | +0.85 | 2.5% |
| — projection checkpoint write | 1.625 | 1.550 | −0.08 | 0% |

Operation counts are **identical at both scales**, which is what rules out a
noisy timer as the diagnosis:

| per-save counter | 100 pages | 1,000 pages |
| --- | ---: | ---: |
| accepted events drained | 1 | 1 |
| SQLite applies | 1 | 1 |
| exact document loads | 2 | 2 |
| catalog document loads | 1 | 1 |
| reference posting lookups | 1 | 1 |
| projection writes | 1 | 1 |
| archive reads (whole save) | 161.6 | 161.1 |

**One operation, ten times more expensive.** `AcceptedRootMaterializer::materialize_page`
(`crates/tine-core/src/oplog/hot_engine.rs:4389`) resolves the edited page's home
document through the catalog, which means `load_document(catalog_document_id)`
→ `ShardedHotEngine::reconstruct_projection_frontier` →
`document_state::load_external_exact` (`crates/tine-core/src/oplog/document_state.rs:794`).
That reads the catalog document's whole state checkpoint out of the scratch
store, SHA-256s it, and imports it with `LoroDoc::from_external_store`. The
catalog holds one entry per page, so the bytes read, the digest, and the import
are all O(total graph pages) — to learn one `page_id -> home_document_id` pair.

### The catalog being decoded is usually the same catalog

A probe on the catalog's `causal_state_digest` across the ten consecutive saves:

| per-save | 100 pages | 1,000 pages |
| --- | ---: | ---: |
| catalog causal-state **changes** | 0.10 | 0.10 |
| catalog causal-state **repeats** | 0.90 | 0.90 |

A content-only edit does not touch the page catalog. Nine of ten saves decode a
**byte-identical** document that the same process decoded moments earlier. This
is the single most important number in this report: it means the fix is a decode
cache, not a redesign.

## The boundary

The cause is one call, and it is not in the write set:

- `crates/tine-core/src/oplog/hot_engine.rs` — the materializer that loads the
  catalog. The dossier says "Avoid `hot_engine.rs` production edits: another
  active worker owns the draft path there. Stop and report if the actual fix
  fundamentally requires that file."
- `crates/tine-core/src/oplog/document_state.rs` — the decode itself. Not named
  in the write set at all.

Three in-scope escapes were considered and each is refuted by the same
measurement:

1. **Use a different engine materializer.** The only alternative,
   `bootstrap_bulk_materializer`, also loads the catalog document *and* then
   runs `validate_catalog` → `read_all_pages` over every entry. Strictly worse
   for one page.
2. **Cache the decoded document from the SQLite side.** The decoded `LoroDoc`
   lives inside the materializer the engine constructs per call and is never
   exposed. Seeding or retaining it needs a parameter on
   `accepted_root_materializer` — a `hot_engine.rs` change.
3. **Skip engine materialization and derive the new rows in the SQLite lane**
   from the accepted event's semantic effect plus the rows already stored at the
   prior root. The effect is a delta (`BlockDelta`/`MembershipDelta`/`PageDelta`),
   so this is not a deletion of repeated work; it is a second, independent
   materialization authority in a lane that has none today, and its divergence
   risk lands exactly on the concurrent/external cases the contract protects.
   That is an architecture decision, not a narrow cut.

### Recommended fix, for the manager to route

Reuse the decoded document, keyed by what already determines its bytes.
`AcceptedRootMaterializer::load_document` already computes
`AcceptedDocumentCacheKey { frontier_state_digest, document_id, causal_state_digest }`.
The document's *content* is fixed by `(document_id, causal_state_digest)` alone;
`frontier_state_digest` is what pins *which* causal state is authoritative at
this root, and that pinning (`accepted_frontier_document`) is cheap and would
still run per save. A small engine-owned cache keyed on the content pair — one
entry would already capture 90% of the observed saves — removes 35.8 ms of the
44.8 ms drain at 1,000 pages without weakening any proof: every row still
receives the authority it receives today, and only a pure decode is reused.

This is ~50 lines in `hot_engine.rs` plus, at most, an eviction bound. It
belongs to whoever owns that file.

## What was cut, and honestly what it bought

Both cuts are real deletions of repeated work in the permitted files. Neither is
graph-sized, so neither changes the scaling, and the receipts below say so.

**1. One accepted-event reconstruction per drained batch.** `drain_ready`'s
frontier proof reconstructs the terminal accepted event out of the archive; the
apply loop then reconstructed the identical bytes from the identical objects.
`authenticate_exact_frontier_retaining_terminal_event` returns its own event and
the loop takes it when the sequence matches. Measured: 2 accepted-manifest
inspections and 4 object inspections per drain become 1 and 2.

**2. The per-apply whole-table reference-coverage count.** Every ordinary apply
ran `SELECT COUNT(*) FROM reference_source_coverage` — the only graph-wide read
in an otherwise point-sized apply. The storage layer already had the inductive
alternative (`CoverageValidation::FreshInductive`) that the fresh-bootstrap
build uses; nothing in `tine-storage` needed to change except a doc comment.
`SqliteFrontier` now retains `InductiveReferenceCoverage { applied_through, rows }`
and offers it only to the immediately following accepted sequence.

The induction is anchored, not assumed: the inductive branch checks the carried
count against the authenticated catalog's *prior* source count, moves it by the
rows this apply actually replaced and inserted (both derived from real table
probes), and ends at the same equality against the *post* source count that the
full scan ended at. A fresh open, a rebuild, a published-candidate adoption, a
duplicate, or any gap in the accepted chain leaves the state absent, which
selects the scan. No database inherits an unproved count and the scan remains
the fallback, as the contract requires.

## Before / after receipts

Same machine, release, `cargo test --release -p tine-core --lib
managed_ordinary_save_manual_release_receipt -- --ignored`, 10 edits per
configuration, one page edited, 10 blocks per page.

| per-save mean, local mode | 100 pages before | after | 1,000 pages before | after |
| --- | ---: | ---: | ---: | ---: |
| public `save_application_page` p50 | 66.906 ms | 65.654 ms | 245.811 ms | 256.047 ms |
| **SQLite drain** | **10.149 ms** | **9.626 ms** | **44.769 ms** | **44.873 ms** |
| — accepted-event reconstruction | 0.252 ms | **0.000 ms** | 0.276 ms | **0.000 ms** |
| — physical apply | 3.988 ms | 3.889 ms | 5.075 ms | 4.742 ms |
| — of which SQL transaction | 1.892 ms | 1.823 ms | 2.743 ms | 2.595 ms |
| — catalog document load | 4.056 ms | 3.941 ms | 35.759 ms | 34.515 ms |

Read this plainly: **−5.2% of the drain at 100 pages, nothing measurable at
1,000.** The cuts remove fixed work, and at 1,000 pages the drain is 80% one
catalog decode, so fixed work has stopped mattering. The 1,000-page run also
shared the machine with another worktree's release benchmark; the `p50`,
`authenticate_source` and `materialize_authenticate` deltas at that scale are
contention, not signal, and no conclusion here rests on them.

Scaling shape, unchanged by this work: the drain is ~9.6 ms fixed plus
~0.039 ms per graph page.

## Fail-before / pass-after

`cargo test --release -p tine-core --lib oplog::sqlite::tests::ordinary_`, all
three verified failing on the parent commit's behavior and passing on
`56859dd0`:

| Test | Before | After |
| --- | --- | --- |
| `ordinary_drain_reconstructs_each_accepted_event_once` | `(2, 4)` manifest/object inspections | `(1, 2)` |
| `ordinary_applies_count_the_reference_coverage_table_once_per_open` | `[(1,0), (1,0), (1,0)]` | `[(0,1), (0,1), (0,1)]` |
| `ordinary_drained_saves_match_clean_archive_replay_across_rich_shapes` | passes (no-divergence oracle) | passes |

The coverage proof runs at two graph sizes — 1 reference source and 41 — and
asserts the per-save accounting is *identical*, which is the dossier's
"proportional to the changed page/reference delta, not total graph pages"
stated as an exact identity. It also asserts a reopened database re-proves the
count instead of inheriting it.

The differential proof drives eight ordinary drained saves covering Markdown and
Org sources, page and block properties, tags, a task with priority and
`SCHEDULED`, an Org properties drawer, an alias declaration, references gained
and lost, a rename that moves name and path and rewrites a surviving referrer,
and a page deletion. It then compares the drained database against a clean
archive replay of the same accepted history on:

- exact `AcceptedFrontierRoot`, semantic projection digest, authenticated
  reference catalog root;
- per-table row digests for all materialized tables, and the materialized row
  digest;
- `pages` unfiltered and per kind, `page`, `pages_by_path`, `pages_by_name`,
  `blocks_on_page`, `block`, `properties` for every page and block owner,
  `properties_named`, `tags`, `tasks`, `referrers_to`, quoted FTS `search`, and
  the reference-catalog name/alias candidate surfaces and coverage row count;
- `diagnose_full_integrity` on both.

It then reopens the drained database and repeats the whole comparison, which is
the interruption/reopen dimension that matters for the changed code: the
inductive coverage state is deliberately not carried across an open. The test
asserts the observed shapes are non-empty before comparing, so two empty
observations cannot agree.

## Commands and results

| Command | Result |
| --- | --- |
| `cargo test --release -p tine-core --lib -- oplog::sqlite::tests::ordinary_` | 3 passed |
| `cargo test --release -p tine-core --lib -- oplog::sqlite oplog::local_active oplog::import oplog::shadow_projection oplog::enrollment oplog::exact_external_feed oplog::projection oplog::reconciliation_scan sync_runtime::` | 636 passed, 1 failed (pre-existing, below), 11 ignored |
| `cargo test --release -p tine-storage --lib` | 130 passed, 1 ignored |
| `cargo fmt --all` | clean |
| `git diff --check` | clean |

`RUST_MIN_STACK=134217728` is set for the `tine-core` runs, as in the two
preceding notes.

`oplog::import::tests::detached_bootstrap_conflicting_abandoned_content_address_fails_closed`
fails identically on `982a3ef4` with this work stashed. It is the pre-existing
failure recorded as limitation 4 of the chunk-1 note.

## Preserved invariants

- The accepted oplog event, prefix, frontier, and every durable archive object
  are byte-identical: neither cut authors, encodes, or stores anything. Cut 1
  reuses bytes the same constructor produced from the same objects; cut 2
  changes only how a row count that is checked against the same authenticated
  value is obtained.
- SQLite schema, schema version, oplog wire format, archive format, provider
  protocol and projection format unchanged.
- Every public query surface is proved semantically identical against clean
  archive replay, per table and per query, including FTS.
- Restart, cold reopen, archive replay, rollback/refusal, interruption recovery,
  shared-provider mode and external reconciliation unchanged. Shared mode is
  covered by the release receipt (both modes measured, both tracking within
  noise) and by the 137-test `sync_runtime` suite.
- Durability unchanged: same transaction, same candidate proofs, same WAL
  checkpoint, same projection-checkpoint write, same base-revision gating. The
  save is still durably committed before success is published.
- No graph-wide scan was moved into another phase; one was removed.
- Fallback and refusal remain available on both cuts. Cut 1 falls back to
  reconstructing the event whenever the retained one is absent or is not the
  sequence being applied. Cut 2 falls back to the full scan whenever the
  inductive precondition is not proved.
- Terminal SQLite construction, its structural accounting invariant, and the
  window-bounded catalog authority from `982a3ef4` are unchanged and still
  asserted.

## Limitations and follow-ups

1. **The measured cause is untouched and is 80% of the drain at 1,000 pages.**
   It needs the decoded-catalog reuse described above, in `hot_engine.rs` /
   `document_state.rs`. *Blocked on the write set. Needs manager routing.*
2. **The same decode is very likely paid more than once per save.** The prior
   lane measured the CRDT draft (29% of total save scaling) and the projection
   drain as separate graph-sized costs; both traverse the same catalog. A cache
   at the decode would likely cut all of them at once, but this lane measured
   only the SQLite drain and does not claim the others. *Follow-up.*
3. **`write_projection_checkpoint` is 1.6 ms per save and flat.** It is 34% of
   the physical apply at 100 pages and is pure fixed cost. Not graph-sized, so
   out of this cut's contract, but it is the largest remaining fixed item in the
   drain. *Follow-up.*
4. **The instrumentation is not committed.** The sub-phase timers, the object
   store inspection counters, and the catalog causal-digest probe were temporary
   and were removed before the production commits, as the dossier requires. To
   reproduce the tables above from `56859dd0`: `git cherry-pick --no-commit
   5ab8037e` for the benchmark, then re-add RAII timers around
   `TailOverlay::drain_ready`, `materialize_accepted_event_with_stats`,
   `attach_authenticated_reference_catalog_at`,
   `apply_with_materialization_transaction_policy`, and
   `AcceptedRootMaterializer::materialize_page`, plus a thread-local comparing
   the catalog's `causal_state_digest` across `load_document` calls. Counters are
   read inside the actor because they are thread-locals and the actor owns its
   thread.
5. **1,000-page numbers were taken under CPU contention** with another
   worktree's release benchmark on the same machine. The sub-phase *shares* and
   the operation counts are unaffected; absolute `p50` at that scale is not a
   basis for any claim here.
6. **Process note.** While stopping my own benchmark I used a `pkill` pattern
   that also matched an identically named benchmark running in the
   `perf-managed-save-hot-draft` worktree, and killed it. No files outside this
   worktree were touched and that worker restarted its run, but the interruption
   was mine. Later kills were scoped by PID.
