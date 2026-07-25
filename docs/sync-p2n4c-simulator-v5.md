# P2N4C simulator-v5 corpus

The v5 operational scenarios are deliberately serialized (`--test-threads=1`)
and use the production coordinator, archive, SQLite materializer, projection
receipt index, and graph renderer.  Fault injection only changes the durable
boundary being exercised; it does not use a second importer, SQLite model, or
projection implementation.

## Stable scenario matrix

| Scenario test | Seed(s) | Journey and asserted evidence |
| --- | ---: | --- |
| `coordinator_v5_fixture_replays_real_storage_retry` | fixture | Replays the checked-in production-storage retry fixture. |
| `coordinator_v5_nested_success_and_projection_fault_are_replayable` | 50_005 | Nested supported-layout Page journey, projection fault, retry, scenario encode/decode. |
| `coordinator_v5_blocked_and_noop_imports_preserve_all_durable_evidence` | 50_051 | A duplicate requested path is blocked and a clean import is a no-op. Both compare the full checkpoint: archive, receipts, SQLite files/rows, rendered graph, tail, pending work, and handoff. |
| `coordinator_v5_sqlite_stale_frontier_delete_truncate_and_corruption_rebuild_exactly` | 50_102 | A stale-but-well-formed frontier, delete, truncation, and byte corruption each close the read gate. Reopen rebuilds to the checkpoint's accepted sequence/frontier/batches and normalized SQLite sequence/frontier/rows. |
| `coordinator_v5_stale_plan_forces_recapture_before_publication` | 50_203 | Stale-plan recapture pins the safe prepublication outcome and changed managed bytes; after recapture, a no-op compares accepted history/frontier, archive, receipt, SQLite rows/files, projection, tail, and handoff byte-for-byte. |
| `coordinator_v5_stale_draft_and_changed_capture_block_before_publication` | 50_204 | Changed source after draft and withheld receipt after capture each stop before publication; restored recapture and a no-op compare every durable/derived surface exactly. |
| `coordinator_v5_rename_then_deletion_reconciles_exact_managed_paths` | 50_304 | Rename removes the old path and renders the exact new Page base. Deletion reconciles after its bounded failure; two no-effect retries preserve archive, receipt, SQLite, projection, graph, tail, and handoff evidence. |
| `coordinator_v5_acceptance_sequence_is_not_batch_id_order` | 50_305 | Pins the three accepted batch IDs, accepted sequence/frontier, SQLite frontier/type-exact rows, then proves a no-op preserves all archive, receipt, SQLite-file, projection, tail, and handoff evidence. |
| `coordinator_v5_projection_failure_after_acceptance_recovers_without_reaccepting` | 50_406 | `DuringProjection` fails after authoritative acceptance: operation and SQLite are current, one projection item is pending, then retry drains it without reaccepting; the no-op checkpoints pin archive, receipt, SQLite files/rows, managed projection, tail, and handoff. |
| `coordinator_v5_crash_reopen_reconstructs_every_durable_boundary_idempotently` | 50_400–50_407 | Crashes/reopens at objects, manifest, stage, SQLite apply, and projection boundaries, then exact no-op checkpoints prove the recovered archive/history, receipt, SQLite, projection, tail, and handoff. |
| `coordinator_v5_failure_capsule_keeps_exact_durable_witness` | 50_512 | Exact coordinator witness capsule, including accepted/archive, receipt, SQLite rows/files, managed projection, tail, handoff, and frozen candidate identity. |
| `coordinator_v5_projection_fault_capsule_records_authoritative_pending_work` | 50_514 | Minimizes and replays a `DuringProjection` capsule with deterministic seed, expected pending-work oracle, all durable witnesses, typed durable boundary, and exact frozen candidate identity. |

## Oracle and failure policy

Every coordinator action remains subject to the simulator's campaign global
oracles. Scenario-local assertions pin the failure outcomes and exact managed
bytes where an external edit is expected; the stable recovery/no-op checkpoints
explicitly compare accepted history/frontier, immutable archive, receipt files,
type-exact SQLite rows and file evidence, managed projection, pending work,
tail, read gate, and durable handoff. Materialization checkpoints deliberately
allow SQLite page layout to change on rebuild, while requiring a recreated file,
the exact type-tagged row digest, and all non-SQLite surfaces to match.

The exercised fault boundaries are `AfterDraft`, `AfterCapture`, stale SQLite
frontier/delete/truncate/corrupt read-gate faults, `DuringProjection`, and the
crash/reopen sweep from `AfterObjects` through `AfterProjection`. Intentional
failure capsules are self-contained: each includes the seed, minimized trace,
invariant identity, expected and observed durable evidence, and typed boundary,
and is encode/decode/replay checked.

## Failure capsule v6

Scenario schema remains v5. Failure capsules advance from v5 to v6 because a
capsule now requires `frozen_candidate`: a typed full lowercase Git-object ID
or SHA-256 frozen-patch digest supplied by the caller that constructs the
capsule. There is no environment fallback and no placeholder serialization.
Replay requires the same typed identity. Tests reject absent/invalid identities,
reject a v5 capsule before required-field decoding, reject future versions, and
reject replay against a different frozen candidate.

## Pending P2N4D fixture seam

This packet intentionally leaves graph-wide nonstandard-layout policy to
P2N4D. In particular, external-path Markdown and Org journeys (including
graph-wide sparse-ID/CRLF/code-literal fixtures) remain pending there. P2N4C
only exercises the existing supported managed-layout Page paths and must not be
interpreted as enabling graph-wide external-path discovery or rendering.
