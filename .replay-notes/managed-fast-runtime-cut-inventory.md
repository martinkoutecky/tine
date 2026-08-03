# Managed fast-path runtime cut inventory

Revision: `89e5d8f6209220faa72f1c0a27224a0305398175`

Scope: the ordinary trusted-local application save only. Remote frames and
external Markdown/Org reconciliation must continue through the existing
validation, archive, tail, SQLite, receipt, and projection pipeline.

## Decision summary

The safe cut is **after local authorization, current-state/base validation,
transaction preparation, and complete deterministic projection preparation,
but before archive publication**. The foreground must durably append one
authenticated canonical `PreparedBatch` journal record, then publish every
guarded Markdown/Org target described by that record and complete the required
file/directory durability barriers. It may acknowledge only after both are
durable. Archive expansion, engine acceptance, tail admission, SQLite drain,
projection-receipt adoption, authorship/provider publication, and cold reload
are rebuildable work and must not remain in the foreground.

This cannot be implemented as a small rerouting of
`OperationalCoordinator::execute_local`. Four architectural prerequisites are
missing today:

1. There is no persistent authoritative local-journal format. The record must
   contain the fully prepared canonical batch, not merely the user-level
   `OperationTransaction`; the latter lacks allocated identities, CRDT/frontier
   results, annotated bases, and manifested projection targets.
2. The only production graph projection writer consumes receipt-backed
   `ProjectionMutationAuthority`. A journal-backed projection proof and an
   asynchronous bridge that later adopts the already-exact graph target into
   projection receipts are required.
3. The single-threaded actor has no committed semantic overlay. After an early
   acknowledgement, the next save/read would otherwise consult a stale engine
   and stale SQLite projection. A journal-applied hot-engine overlay (including
   startup replay) is required before the cut is usable.
4. Existing-page lookup/materialization is SQLite-first, and rename planning
   calls the archive/SQLite-backed `frontier_reference_query`. A strict
   no-SQLite/no-archive foreground needs direct authenticated hot-engine page
   materialization and reference planning. Until that exists, rename/title
   saves must remain on the slow path rather than weakening their atomic
   referrer rewrite semantics.

Micro-optimizing the current unified pipeline does not change these ownership
and recovery requirements.

## 1. Exact current synchronous call graph and ownership boundaries

### Public application save through the actor

The complete ordinary save remains under the caller's synchronous request:

```text
SyncRuntimeHandle::save_application_page                    sync_runtime.rs:2724
  lock HandleInner.operation: Mutex<()>                     sync_runtime.rs:1969
  validate request bounds/shape
  send ActorRequest::SaveApplicationPage
  wait for synchronous reply
    run_actor_loop                                           sync_runtime.rs:5180
      RuntimeActor::save_application_page                    sync_runtime.rs:6600
        prepare_editor_turn
          advance any PendingLocalMutation once
          tick/drain pending exact watcher feed (bounded)
        existing page:
          load_application_exact_ready
            load_application_page_id_ready
              load_source_authenticated_application_page
                load_projected_editor_page                  sync_runtime.rs:11650
                  SqliteMaterializedRead + frontier check
                Graph::load_by_path + parse
                join_application_page
          compare application revision/read-only/DTO shape
          translate frontend block IDs to editor keys
        new page:
          validate DTO
          editor_name_state (engine name/path ownership)
          new_editor_revision
        build SyncEditorSaveRequest
        RuntimeActor::save_editor_page                       sync_runtime.rs:6907
          existing page:
            load_source_authenticated_editor_page
              SQLite materialization + graph parse
            direct engine materialization and equality check
            compare existing_editor_revision
            read_projection_input + render/parse current/next
            resolve parser-owned final name and kind
            if renamed:
              runtime.database().frontier_reference_query(
                  runtime.engine(), archive_store)
              plan_page_rename
            build_existing_editor_transaction
          new page:
            editor_name_state + compare new_editor_revision
            allocate PageId/DocumentId/BlockIds
            render/parse and resolve final identity/kind
            build_new_editor_transaction
          unchanged:
            load_current_editor_page (SQLite) and return
          changed:
            execute_editor_transaction                      sync_runtime.rs:7213
              PromotedLocalRuntime::admit_promoted_mutation
              OperationalCoordinator::execute_local
              if Active:
                record_local_authorship_receipt             sync_runtime.rs:8544
                record_provider_publication                sync_runtime.rs:10306
                load_current_editor_page (SQLite)
              if Published/Blocked/Revoked:
                retain PendingLocalMutation in actor
        if publication was retained:
          settle_application_publication
            repeatedly advance_local_mutation_once
        reload_application_page (SQLite + graph parse/join)
        return Saved/Unchanged PageDto and revision
```

`existing_editor_revision` (`sync_runtime.rs:11889`) hashes the page ID, home
document ID, revision-free editor DTO, and authoritative block identity/origin
data under the v2 domain. `new_editor_revision` hashes the logical name, path,
and kind. `join_application_page` preserves parser-owned path, preamble, outline,
and read-only semantics while replacing fallback name/kind and frontend IDs with
authoritative engine identity. Consequently, the post-save response cannot be
invented from the input DTO; it must be prepared from the finalized post-state
and parsed rendered target.

The `HandleInner.operation` mutex is held until the reply is received. The actor
loop is also serial. `RuntimeActor` owns `Graph`, `ProjectionReceiptStore`,
`LocalActiveAuthority`, `PromotedLocalRuntime`, exact external-feed state, and
the optional retained mutation. Its `PhantomData<Rc<()>>` makes it deliberately
`!Send`/`!Sync`. Thus moving work past the acknowledgement does not by itself
make that work asynchronous; the actor needs an owned pending-journal queue and
idle drain scheduling, or a separately authorized worker.

### Local operational coordinator

`OperationalCoordinator::execute_local`
(`oplog/operational_coordinator.rs:1197`) obtains the promoted session parts and
enters `execute_local_inner` (`:1287`):

```text
authorize_coordinator / reprove workspace authority
verify_bindings(graph, receipts, engine, archive)
Graph::mint_handoff_safe
HandoffSafe -> HandoffSafeGuard -> publisher guard
admission.mint_local_author_authority
engine.draft_admitted_local_author_transaction
capture_local_author_transaction
  for each required graph path:
    Graph::read_projection_input
    authenticate prior receipt/base authority
    compare exact current bytes and identity
  mismatch => drop guard and return ReconciliationRequired
finalize_captured_author_transaction
  recheck receipt authority
  create annotated base objects
  create manifested projection intents and targets
  return PreparedBatch
publish_and_drain
  retain encoded batch bytes
  TailOverlay::reserve_bound_mutation
  encode manifest/digest and reprove publication authority
  turn guard into PublishedHandoffLatch
  ObjectStore::publish_prepared
    publish every immutable object first
    publish manifest last (first irreversible coordinator boundary)
  PublishedContinuationCore::resume
    authenticate ready archive batch/digest/retained bytes
    engine.stage_archive_batch_bounded
    TailOverlay::enqueue_reserved/try_enqueue
    TailOverlay::drain_ready(database, RebuildSource, budget)
    assert SQLite frontier == engine accepted frontier
    execute manifested projection work under handoff
      projection intent/attempt/completion receipts and work index
    execute receiver-local foreign projection intents
    release PublishedHandoffLatch only on complete drain
```

`AuthorTransactionDraft` in `oplog/hot_engine.rs` already owns the prospective
documents/pages, requirements, semantic effect, post-frontier, and portable
root. Finalization turns those into the complete `PreparedBatch`, including the
canonical archive objects, annotated bases, manifest, projection intents, and
target bytes. That is the earliest artifact sufficient for deterministic crash
replay without redrafting against possibly changed state.

After coordinator completion, the runtime synchronously persists a local
authorship receipt. That operation authenticates the ready archive manifest and
accepted engine evidence and uses `model::atomic_update`. It then creates the
provider pending/publication state. Both necessarily occur after the old
archive/acceptance pipeline and are rebuildable from the proposed journal.

### Graph locking and durable filesystem operations

`Graph` (`model.rs:1652`) retains the graph root through a capability directory
and maintains:

- a resource-scoped `ManagedTextWriteGate` shared through the canonical graph
  binding;
- per-path locks in `page_locks: Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>`;
- caches and disk revisions, ordered after the writer permit and page lock.

The documented lock order is managed writer permit, page lock, cache, then disk
revision state. `mint_handoff_safe` reserves the graph-wide managed-text gate
only when no managed writer is active. The guard is converted to a
`PublishedHandoffLatch` at the archive publication boundary. Dropping an
unpublished guard releases it; dropping a published latch intentionally does
not. Only explicit cancellation before publication or successful terminal
completion opens the gate.

The graph-wide handoff spans coordinator capture, archive publication, engine,
tail, SQLite, and projection drain. A per-path lock does **not** currently span
base capture to archive commit to graph publication: `read_projection_input`
takes and releases it for the read, and the projection writer later reacquires
it. The projection writer therefore performs a last-moment exact base and file
identity check.

`write_page_projection_with_attempts` consumes a receipt-derived
`ProjectionMutationAuthority`, takes the page lock, verifies the exact base and
serializer target, creates and syncs recovery evidence, syncs and captures the
old target identity, retires it with a no-replace rename, creates and file-syncs
the new target, publishes it with a no-replace rename, calls
`sync_projection_chain_required`, and rereads the exact result before updating
cache state. Failure paths withdraw, validate, restore, or retain recovery
objects without silently clobbering a concurrent writer.

The audited primitives are already suitable building blocks:

- `rename_projection_noreplace` / `rename_managed_noreplace` in `model.rs` use
  platform-specific no-replace operations and retained handles;
- `sync_projection_chain_required` delegates required directory barriers to
  `tine_storage::sync_dir_required`;
- `tine_storage::publish_immutable_exact` (`filesystem.rs:438`) stages to a
  validated directory, writes and file-syncs, no-replace renames, and directory
  syncs; an exact pre-existing winner is authenticated;
- `ObjectStore::publish_prepared` (`object_store.rs:2097`) publishes every
  object before the manifest, making manifest presence the archive commit
  predicate.

The fast path should reuse these audited operations, not add a raw `std::fs`
writer. It does need a new authority type because receipt attempt authority does
not exist before the asynchronous receipt projection stage.

### Watcher ownership and external reconciliation

`src-tauri/src/watcher.rs` forwards graph-relative sparse observations except
the `.tine-sync` subtree. It does not filter ordinary local projection events.
`SyncRuntimeHandle::observe_watcher` serializes on the same `operation` mutex and
hands observations to `ExactExternalFeedState`, which classifies, queues,
rescans when required, and submits authenticated external changes through the
existing slow coordinator.

There is a separate legacy `Graph::recent_writes` exact-content marker used by
the older graph watcher/cache path. It suppresses a matching echo there, but it
does not suppress the Tauri sparse observation before it reaches the runtime's
exact external feed. A local projected file may therefore be observed and
reconciled as an exact no-op today.

The safe initial fast path is to leave that callback asynchronous and let it
authenticate the exact journal target as a no-op. An optimization may consume a
journal-backed projected-path marker only if it verifies the commit, exact path
and bytes, and advances the external-feed fence. Blindly dropping a path would
lose a racing external edit.

### Startup, shutdown, and retained mutation recovery

`RuntimeActor::open` opens and revalidates the graph, enrollment, receipt store,
baseline, and promoted runtime. A Safe enrollment reopens normally; an Unsafe
one takes over the crashed runtime. The exact external feed then starts with a
full scan. `PromotedLocalRuntime` owns the retained enrollment/session lease,
hot engine, leased SQLite/archive runtime, `TailOverlay`, bootstrap state,
watcher queue, and revocation state.

Current `PendingLocalMutation` continuations are process-memory affine values.
A crash loses the continuation; recovery succeeds because archive publication
and the Unsafe enrollment let startup rediscover/stage/drain accepted work. A
journal fast path changes that source of truth: startup must authenticate and
replay committed local journal records **before** importing external graph
observations. It must first apply the semantic overlay, finish any pending graph
projection, and only then open the exact external feed. Otherwise a committed
operation can disappear from the hot state, or its projected bytes can be
misclassified as an unrelated external edit.

`clean_shutdown` currently refuses while a retained local publication remains,
drains provider and exact-feed work, quiesces graph/watcher activity, proves
device-local drains, and only then commits the enrollment Safe. The new Safe
predicate must additionally prove that every committed journal record has its
graph durability marker and is either fully expanded/adopted or durably covered
by a startup-replay checkpoint. Dropping the handle without clean shutdown must
continue to leave the enrollment Unsafe.

## 2. Foreground cut table

“Before ack” means before the agreed boundary: canonical operation durable plus
all guarded Markdown/Org projections durable.

| Operation | Current reason/owner | Before ack? | New owner |
|---|---|---:|---|
| Request bounds, DTO shape, read-only checks | Public/application safety | Yes | `SyncRuntimeHandle` and actor admission |
| Per-handle request serialization | Prevent overlapping actor operations | Yes | Existing `HandleInner.operation`; release at reply |
| Prior exact watcher classification needed to establish the base | Avoid saving over an already-observed external edit | Yes, bounded; otherwise return deferred | Existing exact-feed admission gate |
| Workspace/enrollment/session authorization and binding proofs | Prevent cross-resource publication | Yes | Promoted local admission + trusted-local coordinator |
| Current page lookup and source-authenticated materialization | Establish authoritative identities and response semantics | Yes | Direct hot-engine/graph reader; remove SQLite dependency |
| Base revision comparison | Optimistic concurrency contract | Yes | Trusted-local coordinator under path guard |
| Parser-owned name/kind/path resolution and transaction construction | Preserve Markdown/Org/editor semantics | Yes | Existing editor builders |
| Rename/reference planning | Atomic referrer rewrite | Yes | New authenticated hot-engine reference planner; rename stays slow until present |
| Engine draft, graph capture, finalize, render, annotated bases, projection intents | Produce replay-complete deterministic operation | Yes | Refactored prepare phase returning `PreparedLocalCommit` |
| Graph-wide managed-text handoff and ordered per-path locks | Exclude in-process writers and bind captured bases to projection | Yes | `TrustedLocalPathGuard` |
| Last-moment no-follow target/parent identity and exact-base verification | Detect external races | Yes | `TrustedLocalPathGuard` immediately before journal commit |
| Canonical local journal append, file sync, no-replace commit, directory barrier | Durable authoritative operation | Yes | `TrustedLocalJournal` |
| Guarded graph retire/write/rename/file+directory sync/final reread | Durable user-visible Markdown/Org projection | Yes | Journal-authorized graph writer under same guard |
| Construct returned PageDto/editor DTO/revision | Avoid cold reload while preserving exact response | Yes | Prepared post-page result |
| Archive object and manifest publication | Old canonical operation durability | No | Journal expansion worker, idempotently from prepared bytes |
| Engine archive staging/accepted history | Feed old engine acceptance model | No | Journal expansion worker |
| Tail reservation/admission/enqueue | Order accepted work toward SQLite | No | Journal expansion worker; no reservation in foreground |
| SQLite drain/frontier/catalog projection | Rebuild query state | No | Background tail drainer |
| Projection intent/attempt/completion receipts and work-index advance | Prove old manifested projection execution | No | Journal-to-receipt adoption worker after exact-target authentication |
| Local authorship receipt | Bind local authorship to accepted archive evidence | No | Background adoption worker |
| Provider pending marker/publication/head work | Remote publication | No | Existing provider worker after authorship adoption |
| Editor/application reload | Refresh from SQLite and graph | No | Eliminate from save; use prepared result, cold reload only for later explicit reads |
| Watcher echo/no-op reconciliation | Maintain exact external feed fence | No | Exact-feed idle work or authenticated self-write adoption |
| Remote frame ingestion | Validate untrusted/remote canonical input | N/A: retain slow path | Existing `OperationalCoordinator` |
| External Markdown/Org reconciliation | Parse and validate uncontrolled files | N/A: retain slow path | Existing exact feed + `OperationalCoordinator` |

An unchanged save need not append a journal record, but it must produce its
response from the already authenticated direct current state, not from a new
SQLite reload.

## 3. Minimal internal APIs and types

Names below are proposed internal contracts, not claims that the types already
exist.

### Replay-complete prepared operation

```rust
struct PreparedLocalCommit {
    batch: PreparedBatch,
    post_pages: BTreeMap<PageId, PreparedPostPage>,
    projection_plans: Vec<TrustedLocalProjectionPlan>,
}

struct PreparedPostPage {
    page_id: PageId,
    materialized: MaterializedPage,
    editor: SyncEditablePageDto,
    application: PageDto,
    revision: String,
    projected_path: ManagedPath,
    projected_bytes: Vec<u8>,
}
```

`PreparedBatch` remains the single canonical semantic/wire preparation result.
`PreparedPostPage` is derived from its prospective page/document state and by
parsing and joining the exact rendered target. It is not reconstructed from the
request and does not read SQLite. The requested page is returned to the caller;
rename referrers remain in `projection_plans` even when no response DTO is
needed for them.

The hot engine needs direct, authenticated APIs equivalent to:

```rust
fn materialize_current_page_at_path(&self, path: &ManagedPath)
    -> Result<Option<MaterializedPage>, Error>;

fn plan_authenticated_reference_rewrite(&self, rename: &PageRename)
    -> Result<ReferenceRewritePlan, Error>;

fn apply_trusted_journal_commit(&mut self, commit: &DurableLocalCommit)
    -> Result<CommittedOverlayAdvance, Error>;
```

The third call advances rebuildable in-memory semantic state before the reply,
so a subsequent request using the returned revision sees the commit even while
archive/SQLite are behind. Startup rebuilds the same overlay from journal
records before serving requests.

### Durable journal record and proof

```rust
struct TrustedLocalJournalRecordV1 {
    workspace_binding: WorkspaceBinding,
    lineage: LineageId,
    endpoint: EndpointId,
    local_commit_id: LocalCommitId,
    prepared_batch_bytes: Vec<u8>,
    prepared_batch_digest: ContentDigest,
    ordered_projection_bindings: Vec<JournalProjectionBinding>,
}

struct DurableLocalCommit { /* non-forgeable resource + record proof */ }
```

`prepared_batch_bytes` must use the same canonical encoding later published to
the object archive. Each projection binding identifies the managed path,
annotated exact base/absence precondition, target digest and bytes, and batch
intent. The journal commit ID is deterministic or protected by a caller
idempotency token; retrying expansion cannot create a second operation.

The storage API should expose capability-relative open/scan/append/checkpoint
operations and return `DurableLocalCommit` only after the record file and
containing-directory barrier have succeeded. Corrupt, truncated, divergent, or
cross-workspace records fail closed. Garbage collection is legal only after an
authenticated checkpoint proves archive acceptance, graph projection/adoption,
and all required durable derivative coverage.

### Guard spanning base verification, journal commit, and graph publication

A callback API avoids returning self-referential `MutexGuard`s:

```rust
fn Graph::with_trusted_local_projection_paths<R>(
    &self,
    binding: &GraphRuntimeBinding,
    plans: &[TrustedLocalProjectionPlan],
    f: impl FnOnce(VerifiedLocalPaths<'_>) -> Result<R, Error>,
) -> Result<R, Error>;

impl VerifiedLocalPaths<'_> {
    fn commit_journal(
        self,
        journal: &mut TrustedLocalJournal,
        prepared: PreparedLocalCommit,
    ) -> Result<JournalCommittedPaths<'_>, PreCommitError>;
}

impl JournalCommittedPaths<'_> {
    fn publish_all(
        self,
    ) -> Result<GraphDurableLocalCommit, CommittedPendingProjection>;
}
```

The implementation acquires the graph-wide handoff/writer exclusion and sorted,
deduplicated path locks, validates retained parent/target identities and exact
bases, and holds them through journal durability and all graph publications.
The typestate consumes `VerifiedLocalPaths` at journal commit, making a graph
write before canonical durability unrepresentable. `JournalCommittedPaths` is
non-`Clone`, non-serializable, and can authorize only the bound paths/targets.
It reuses the audited recovery/no-replace/fsync machinery. External processes
cannot be locked, so every target still gets the final identity/base checks and
post-sync exact reread.

For a multi-file rename, all paths—including source, destination, and every
rewritten referrer—must be known before entry and locked in canonical path
order. No path may be discovered after journal commit.

### Committed-but-not-yet-projected outcome

Once journal commit succeeds, an error is not a normal save failure and the
transaction must never be redrafted or resubmitted:

```rust
enum TrustedLocalCommitOutcome {
    Unchanged(PreparedPostPage),
    Committed(GraphDurableLocalCommit),
    CommittedPendingProjection(CommittedPendingProjection),
    ReconciliationRequired(Vec<ManagedPath>), // precommit only
    BlockedPrecommit(BlockReason),
    RevokedPrecommit,
}

struct CommittedPendingProjection {
    commit: DurableLocalCommit,
    post_page: PreparedPostPage,
    remaining_paths: Vec<ManagedPath>,
    last_failure: ProjectionFailure,
}
```

The actor retains `CommittedPendingProjection`, refuses to reinterpret its graph
bytes as external input, and finishes it from the journal. It can acknowledge
only after it becomes `GraphDurableLocalCommit`; if the process dies earlier,
startup performs the same completion.

An additional `JournalProjectionAdoptionAuthority` should authenticate a
durable journal commit plus an exact graph target. The asynchronous old-pipeline
adapter uses it to publish receipt completion/work-index evidence without
rewriting the file. This closes the authority gap between the new foreground
writer and existing manifested projection accounting.

## 4. Crash-state transitions

| Durable state at crash | Was operation committed? | Recovery transition and client meaning |
|---|---:|---|
| Request rejected, base mismatch, draft/capture/finalize only in memory | No | Discard volatile state. A retry may redraft. No graph or archive effect is permitted. |
| Journal temp created or file-synced, but no authenticated no-replace commit/directory barrier | No | Ignore/quarantine and safely clean the uncommitted temp. Exact graph remains at the verified base. |
| Journal record and directory are durable; graph is untouched | Yes | Replay record, apply semantic overlay, enter `CommittedPendingProjection`, and project before external-feed import. Never redraft. |
| Old graph target retired and durable recovery evidence exists; new target not durably published | Yes | Use the journal-bound recovery state to finish target publication or restore only as an intermediate step, then retry. The commit remains authoritative. |
| New target is visible but file or parent-chain durability barrier did not complete | Yes | Authenticate exact target and identities; complete the required barrier if possible, otherwise withdraw/restore through audited recovery and retry. Do not acknowledge based on visibility alone. |
| Every target is file/directory durable and reread exact, but journal projection-complete marker is absent | Yes; acknowledgement boundary is satisfied | Startup authenticates journal + exact targets, records graph-durable state, then schedules derivative work. If the reply was lost, the client outcome is unknown, not failed. |
| Reply was sent; archive has no objects/manifest | Yes | Serve next reads/saves from the committed overlay. Background worker expands the exact prepared batch. |
| Some archive objects exist, manifest absent | Yes | Idempotently authenticate/reuse exact objects and publish remaining objects then manifest. Divergent winners fail closed. |
| Archive manifest durable, engine has not accepted it | Yes | Stage the ready batch into the hot engine. Do not republish or reproject the user operation. |
| Engine accepted, tail reservation/admission or SQLite drain pending | Yes | Recreate/admit rebuild source and drain. Foreground reads continue through hot state/overlay. |
| SQLite current, projection receipts/work index absent | Yes | Authenticate journal and exact graph targets, publish adoption completion, and advance work index without rewriting targets. |
| Authorship receipt/provider marker/publication pending | Yes | Reconstruct from journal plus accepted archive evidence and resume existing provider publication. |
| All derivatives and checkpoint durable | Yes | Journal record may be compacted only when the checkpoint covers archive acceptance, graph projection/adoption, SQLite rebuild position, and required provider recovery evidence. |

The crash-after-boundary/before-reply case exposes a public protocol gap: the
current application request has no caller idempotency token. A retry carrying
the old revision will correctly conflict even though the first operation
committed. At minimum, the runtime must expose/retain `LocalCommitId` and define
reload-on-unknown behavior; robust transparent retry requires an idempotency key
in the request contract. This is an acknowledgement-ambiguity blocker, not a
filesystem correctness blocker.

## 5. Smallest safe production-commit sequence

These commits are intentionally file-disjoint where parallel delegated work is
safe. One integration owner must own each shared module declaration and
`sync_runtime.rs`; workers should not make overlapping edits there.

1. **Durable journal primitive and format.** Add
   `crates/tine-storage/src/local_journal.rs` with capability-relative record
   publication, scan, authentication, checkpoint, and crash-cut tests. A single
   integration owner edits `crates/tine-storage/src/lib.rs`. Freeze the V1
   workspace binding, canonical batch encoding, commit-ID, and GC rules here.
2. **Journal-authorized graph path guard.** One worker owns
   `crates/tine-core/src/model.rs`: sorted multi-path guard, typestate transition,
   journal proof binding, audited projection reuse, and exact-target proof. It
   must not alter the receipt-authorized public production entrypoint used by
   the slow path.
3. **Prepared post-state and committed semantic overlay.** A different worker
   owns `crates/tine-core/src/oplog/hot_engine.rs`: expose prepared post-pages,
   direct authenticated materialization/reference planning, apply/replay the
   journal commit, and verify revision equivalence. This is the prerequisite
   that makes an early reply composable with the next save.
4. **Trusted-local coordinator.** Add
   `crates/tine-core/src/oplog/trusted_local_commit.rs` implementing prepare,
   journal commit, guarded projection, and committed-pending-projection. The
   integration owner alone edits `oplog/mod.rs`. Do not edit or weaken
   `operational_coordinator.rs`; it remains the remote/external slow pipeline.
5. **Journal expansion and adoption worker.** Add a separate
   `crates/tine-core/src/oplog/local_journal_drain.rs` that idempotently expands
   `PreparedBatch` into archive/engine/tail/SQLite, adopts exact projections into
   receipts, and emits authorship/provider work. Give any necessary
   `object_store.rs` bridge to this worker alone. This commit can be exercised
   independently from the public fast path.
6. **Runtime routing and lifecycle integration.** One owner edits
   `crates/tine-core/src/sync_runtime.rs`: route eligible ordinary local saves to
   the new coordinator; build replies only from `PreparedPostPage`; schedule
   background drain; replay journal/overlay and finish projections before exact
   external-feed startup; extend Safe shutdown proof. Existing remote/external
   dispatch remains unchanged. Keep rename/title saves slow until commit 3's
   engine-native reference planner is complete.
7. **Public idempotency contract, if transparent retry is required.** One owner
   updates the application command/DTO boundary and frontend caller with a
   request token and committed-result lookup. This is separable from durability,
   but release criteria must explicitly choose it or document unknown-outcome
   reload behavior.

Commits 1–3 can be delegated concurrently because their production ownership is
disjoint. Commits 4–6 are integration-ordered. No delegated worker other than
the runtime owner should touch `sync_runtime.rs`, and no fast-path worker needs
to modify the old coordinator.

## 6. Test reuse and required structural proof

### Existing fixtures/builders to reuse

In `sync_runtime.rs`, retain the application/editor test vocabulary and extend
its instrumentation rather than constructing a second runtime harness:

- `RuntimeHostFixture::safe`, `active_handle`, `drive_initial_feed`,
  `admit_external_page`, `load_application_exact`,
  `load_application_logical`, `accepted_application_save`,
  `new_application_page`, `settle_local_mutation`, `settle_exact_feed`,
  `snapshot_graph_files`, and `assert_parser_dto_semantics`;
- `application_gateway_saves_remap_new_ids_and_use_page_local_revisions`;
- `application_gateway_settles_current_retained_publication_before_returning_saved`
  (replace its foreground-settlement expectation with journal+graph durability
  and derivative-pending assertions);
- `application_gateway_does_not_admit_request_behind_prior_retained_publication`;
- `editor_stale_base_after_external_watcher_edit_has_zero_save_effects`;
- `editor_title_content_and_referrer_changes_share_one_atomic_user_authoritative_save`;
- `editor_parser_authority_matrix_covers_markdown_org_title_and_kind_transitions`;
- `editor_retained_publication_retries_once_and_refreshes_by_bounded_reload`
  (the new equivalent must prove no bounded reload occurs);
- `published_local_failure_is_retained_and_retried_without_republication`;
- `clean_shutdown_refuses_until_retained_local_publication_resolves`;
- `dropping_without_shutdown_leaves_unsafe_and_fresh_open_must_take_over`;
- `unsafe_reopen_repairs_accepted_batch_after_pending_marker_creation_failure`;
- `concurrent_watcher_observation_and_local_submission_are_linearly_reconciled`.

In `operational_coordinator.rs`, reuse `Fixture`, `local_edit`, `settle_local`,
the `expect_local_*` helpers, and all existing bounded-failure hooks. Preserve
coverage from:

- `admitted_local_semantic_mutation_commits_history_sqlite_and_projection_once`;
- `local_exact_path_drift_requests_reconciliation_without_publication`;
- `local_late_failure_retries_exact_publication_without_a_second_writer`;
- `local_continuation_drop_stays_closed_and_completion_releases_once`;
- `sqlite_budget_boundary_retains_handoff_and_resumes_without_republication`.

`local_and_external_mutations_enter_the_identical_terminal_pipeline` must be
intentionally replaced: assert that external/remote operations still enter the
old identical terminal pipeline, while trusted-local saves enter the journal
path and converge to the same canonical archive/engine/SQLite end state.

Reuse the graph crash/race corpus in `model.rs`, especially
`projection_exact_proof_binds_path_bytes_digest_and_exact_preconditions`,
`projection_exact_never_clobbers_pre_publish_or_proves_post_publish_changes`,
`projection_boundary_race_is_rejected_before_displacement`,
`projection_sync_failure_requires_recovery_and_stale_write_remains_conflict`,
`projection_retry_resumes_after_synced_partial_parent_chain`,
`projection_exact_updates_warm_cache_once_and_suppresses_watcher_echo`, and the
handoff tests at `model.rs:42880`. Reuse storage publication tests
`publish_exact_sequence`,
`exact_publish_retries_identically_without_temporary_residue`,
`staged_exact_commit_retries_streamingly_and_drop_cleans_unpublished_temp`, and
`concurrent_publishers_converge_and_preserve_one_divergent_winner` in
`tine-storage/src/filesystem.rs`.

### Exact new assertions

The fast-path test fixture needs monotonic counters/snapshots at call entry,
reply, and eventual quiescence. At the **save reply**, assert:

1. exactly one authenticated journal commit exists for a changed save (zero for
   unchanged), with the expected batch digest and all affected path targets;
2. every guarded graph target is exact, file-synced, parent-chain durable, and
   linked to that journal commit; a precommit base race produces zero journal
   commits and zero graph changes;
3. deltas are exactly zero for archive object writes, archive manifest
   publication, engine accepted events, tail reservations/enqueues/drains,
   SQLite mutations/frontier writes, projection intent/attempt/completion
   receipts and work-index marks, authorship receipts, provider pending/head
   markers, and editor/application reload counters;
4. the returned `PageDto`, editor DTO, IDs, path/kind, and revision equal the
   result of a later fully drained cold reload;
5. a second save using the returned revision succeeds **before** any archive,
   tail, SQLite, receipt, or reload work is allowed to run;
6. a multi-path rename has one journal commit and all source/destination/referrer
   projections durable atomically at the acknowledgement boundary, or remains
   routed to the slow path while engine-native planning is unavailable;
7. a matching watcher echo cannot become a second operation, while a racing
   divergent external edit is reconciled or reported as a conflict;
8. each injected crash boundary in section 4 reopens to exactly one semantic
   operation, one final graph target per path, and eventually one archive
   manifest/accepted history entry—never a redraft or duplicate publication;
9. clean shutdown refuses committed-pending-projection and uncovered journal
   states, and Unsafe reopen completes them before external-feed import.

Add a source-level structural test around the new foreground coordinator/runtime
method. Its production body (including directly called foreground helpers) must
not reference:

```text
OperationalCoordinator::execute_local
ObjectStore / publish_prepared
stage_archive_batch / AcceptedBatchEvent
TailOverlay / reserve_bound_mutation / enqueue_reserved / drain_ready
SqliteFrontier / database() / frontier_reference_query
ProjectionReceiptStore / publish_intent / reserve_attempt / publish_completion
record_local_authorship_receipt / record_provider_publication
load_current_editor_page / reload_application_page
settle_application_publication
```

Conversely, structural tests must assert that remote-frame ingestion and exact
external reconciliation still reference the existing coordinator and its
archive, engine, tail, SQLite, and receipt stages. Add compile-time/trait
assertions that `VerifiedLocalPaths`, `JournalCommittedPaths`, and
`DurableLocalCommit` are not forgeable through public constructors; the path
guards are non-`Clone` and non-serializable; and only a consumed durable journal
proof can reach the graph mutation method.

These structural assertions are necessary because eventual-state tests alone
cannot detect foreground regressions: the old synchronous pipeline and the new
asynchronous pipeline intentionally converge to the same final archive, SQLite,
receipt, provider, and graph state.
