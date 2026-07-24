# 0049. Oplog-first sparse storage uses catalog and stable home-page shards

- **Status:** Accepted — architecture/format selection; implementation and LocalActive remain gated
- **Date:** 2026-07-22
- **Supersedes:** [ADR 0048](0048-compatible-managed-sync.md)

## Context

ADR 0048 correctly made durable operations the sync truth, but its experimental
`.tine-sync/v1` prototype used one graph-wide Loro document and activation wrote a
Logseq `id::` to every block. At graph scale that makes one-page work retain the
whole graph and creates a compatibility-visible migration diff merely to preserve
Tine identity.

The isolated benchmark in `docs/BENCH.md` measured the graph-wide layout and
catalog/page candidates. The stable-home-shard candidate completed 1,000,000 blocks,
including deterministic current-page membership dispersed across immutable homes. The
graph-wide control reached 10,400,100 KiB RSS while constructing 1,000,000 blocks on
a no-swap host and was stopped; 750,000 blocks was its largest completed run. A
monolithic catalog ownership map completed but made a 1,000-page working set load
the million-entry map. Performance alone is insufficient: the layout also has to
preserve edits and select one deterministic owner when moves, deletes, and edits are
concurrent. The corrected 1,000-page fanout materialization passes the P0A CRDT
recovery/fallback gate. The separate 2-second normal startup/hot-state gate depends
on the later SQLite-backed P2.2 engine and is not measured by this harness. This ADR
therefore accepts the architecture and logical format contract without authorizing
implementation completion, persistent-byte rollout, or `LocalActive` activation.

## Decision

### Protocol fence and authority

Sparse storage is a new `.tine-sync/v2/` protocol/layout namespace with a new
enrollment descriptor. No v2 writer interprets or mutates `.tine-sync/v1` bytes.
Startup classifies a store as `Absent`, `LegacyV1`, `Supported`,
`UpgradeRequired`, `FutureUnsupported`, or `Corrupt`; only `Supported` can write.
Legacy v1 requires a separately approved, verified export/import or recoverable
migration and cannot become `SharedActive` with a v2 writer.

The immutable v2 operation log is the only authority for the versioned managed
entity set: page and journal text; their page/block identity, names and paths; and
references/properties in those documents. Markdown and SQLite are frontier-stamped
derived materializations. Assets, PDF sidecars, `config.edn`, and device/app
settings keep their existing separate authorities and are never projected,
regenerated, deleted, or claimed as convergent by v2. Protocol, operation,
object/envelope, receipt, projection-policy, SQLite, and entity-set versions remain
separate compatibility fields. An incompatible authoritative version blocks oplog
and projection writes; a SQLite-only mismatch rebuilds SQLite.

Production uses `sparse_logseq_ids`. `dense_logseq_ids` is test/developer-fixture
instrumentation only and is neither a migration phase nor a persistent production
format. Both policies pass through the same operation, receipt, and importer path
and must produce the same canonical semantics after policy-created anchors are
ignored. Tine adds a valid Logseq `id::` only when the same batch creates an
OG-visible block reference, embed, or exported/copied deep link. It never removes
an ID as cleanup.

### Identity, catalog, and page shards

`WorkspaceId`, `PageId`, `BlockId`, `BatchId`, `DeviceId`, and `SessionId` are opaque
application identities, independent of Loro IDs and file paths. A `PageId` survives
rename/path changes; a `BlockId` survives edits, moves, page moves, compaction,
CRDT replacement, and SQLite rebuild. Optional `LogseqUuid` is separate external
identity evidence. Valid imported IDs populate it; invalid or duplicate raw `id::`
values remain byte-preserved but authorize no overwrite. External removal or
replacement is imported exactly and clears/changes `LogseqUuid`; it is not repaired.

`ManagedPath` retains the exact valid UTF-8 spelling used by operation, object, and
projection bytes. Portable ownership uses a separate `PortablePathKey` version 1:
for every slash-separated component it computes
`Q(X) = NFC(default_case_fold(NFD(X)))`, then joins the results with a literal `/`.
This is full non-Turkic Unicode default case folding, not lowercase/simple fold,
and deliberately excludes NFKC. Version 1 pins `unicode-normalization` 0.1.25
(normalization tables 17.0.0) and `caseless` 0.2.2 (case-fold tables 16.0.0);
either table snapshot changing requires a new key version. Authenticated indexes
use the domain-separated SHA-256 digest of the version and key bytes. Managed
components also reject trailing ASCII space/dot, Win32 forbidden characters and
controls, and case-insensitive device stems before the first dot, including
`COM1` through `COM9`, `LPT1` through `LPT9`, and superscript `COM¹/²/³` and
`LPT¹/²/³` aliases. Accepted spelling is never rewritten.

The production logical documents are:

1. A small catalog shard containing the sole live page state and `PageId -> path`
   register, plus versioned catalog/reference metadata that truly has graph scope.
2. One stable home page shard minted with each page. Blocks created during import or
   normal editing are assigned a home shard once. That shard co-locates the block's
   mergeable content/attributes and its sole live owner register
   `BlockId -> (PageId | Tombstone)`. Home-shard identity does not change on a move.
3. Each page shard also carries ordered membership claims for its current outline.
   A claim is materialized only when the block's home-shard owner register names
   that page. Losing concurrent claims and old shard remnants remain immutable
   evidence but are never projected.

The membership claim carries the home-shard document ID, so opening a page reads its
page shard and the exact home shards referenced by its members; it never scans all
shards. SQLite is the derived lookup for arbitrary `BlockId` queries. The engine
must not create a second live ownership index in the catalog or SQLite.

Loro's deterministic map-register winner selects concurrent owner changes. Because
content stays in the stable home shard, concurrent move/edit preserves the merged
edit regardless of delivery order. Concurrent move/move leaves claims in both
destinations but materializes only the register winner. Move/delete deterministically
selects either a page or `Tombstone`; delete/edit retains the merged text as
recoverable evidence but a tombstoned winner is not materialized. Page rename and
delete use the analogous catalog page-state register. Every cross-document action is
one `OperationBatch`; no receiver exposes only part of it.

### Frontier, batch, and object set

The logical causal frontier is `FrontierV2`: a canonical list sorted by
`DocumentId`. Empty document entries are omitted, and a globally empty frontier is
valid only for a true empty baseline. Every non-empty entry contains both (a) that
document's CRDT version vector as canonical sorted unique `(CrdtPeerId, max_counter)`
entries and (b) only the sorted unique exact direct `BatchId` heads relevant to that
document. A relevant head is an immutable batch that carries an update for the
document and has no relevant descendant in the declared atomic-batch DAG, including
when the descendant path crosses batches that update other documents. An unrelated
maximal batch, a redundant ancestor, and an omitted relevant head are all rejected.
The document causal digest binds the document ID, counters, and exact direct heads.
Cold validation uses authenticated exact document-state and causal-clock point
indexes. It does not walk immutable manifest ancestry; direct heads are still not
permission to omit or approximate the exact frontier.

The earlier full-transitive-closure representation made each sequential page edit
carry and validate all prior batch IDs, so manifest size and hot preparation work
were O(page history). The correction10 same-page probe also exposed accumulated
Loro-history snapshot export: its early/late clone-operation maxima grew from
`[4, 68]` to `[2, 834]` despite a constant-size edit. Compact exact heads plus
reusable current-state author buffers and semantic snapshots remove those two
history-proportional paths; componentwise early/late cost assertions now gate the
construction. `CrdtPeerId` remains an opaque engine-neutral interchange identity,
populated from the current Loro peer identity without making Loro's type part of
this API. Application IDs remain separate from CRDT peer/operation IDs.

This is an incompatible disconnected candidate-format correction.
`manifest_encoding_version` is 4 and `operation_schema_version` is 5.
`receipt_schema_version` is 5 and the device-local projection receipt schema is 4.
The internal CRDT update payload is 6, compact batch status is 3, block-claim records
are 2, Logseq claim records are 1, portable-path key/index records are 1, and engine
scratch is 3. Canonical manifested projection intents are schema 2 and annotated
bases remain schema 1; the authenticated device-local projection-work index/work
row is schema 2. Endpoint-scoped durable engine-history records are schema 4 and
their accepted head/root envelope is schema 2. The causal, document-state, and scratch-page
formats retain their explicitly versioned experimental schemas. None of these bytes
are activated for graph startup, v1 reinterpretation, or production writes.

Each semantic `OperationBatch` has exactly one manifest. The manifest contains all
compatibility versions, workspace and lineage/genesis hash, batch/author/device/
session IDs, an explicit origin (`LocalMutation`, deterministic
`ExternalReconciliation { ImportId }`, or `BootstrapImport`), `FrontierV2`, the
canonical semantic-effect digest, and a canonical sorted list of every required
object's document ID, type, SHA-256 content digest, and byte length. Its closed
object set is:

- exactly one typed semantic-effect object;
- exactly one immutable CRDT update object for each changed catalog/page shard;
- zero or more projection-intent objects for affected paths;
- exactly the immutable annotated base blob required by each non-`Absent` intent,
  deduplicated by digest.

Objects are written under the v2 content-addressed object namespace first and the
`batches/<BatchId>.manifest` commit marker last. Unknown, missing, duplicate, wrong-
type, wrong-length, or wrong-digest objects reject/stage the batch as appropriate;
the same `BatchId` with different manifest bytes blocks the workspace. A projection
completion is a later immutable receipt referencing its intent and is not part of
the already-committed mutation batch.

A receiver stages the complete closed object set. Typed effects are validation and
index metadata, never a second mutation stream. Validation reconstructs the exact
declared dependency state from immutable objects, excluding concurrent non-
dependencies, applies only the committed updates to that state, computes the
versioned canonical affected-entity delta, and compares its digest with the
manifest. Unavailable dependency state remains staged; a mismatch is rejected before
hot state, SQLite, or projection visibility. Full-page snapshots are only
bootstrap/import/recovery payloads, not ordinary edits or merge primitives.

Store-backed hot engines create exactly one `engine-scratch-v2/run-<UUID>`
capability containing a canonical workspace/run marker, an advisory lease, one page
file, and one blob file. Cleanup enumerates exact canonical run names, refuses
symlinks, reparse points, special files, malformed markers, and unknown entries,
skips live leases, and unlinks stale files only relative to that capability. Scratch
never performs a durability sync and has no capability that reaches authoritative
objects, manifests, or lineage bytes.

Authenticated point pages hold compact batch status, reverse dependency waits, a
deterministic disk heap of ready batch IDs, exact causal-dot/vector-clock records,
current and exact document checkpoints, block claims, and canonical conflict
evidence. Ready payload bytes are reloaded from their immutable locator only while
one batch is processed. Visible and terminal document handles share the catalog plus
64 non-catalog cache bound. Fatal hot state is a fixed root/count/digest handle;
complete conflict evidence is explicitly streamed from canonical pages. The
immutable home shard remains the sole owner authority.

Document checkpoints are editable Loro shallow snapshots split into fixed-size
content-addressed chunks, so state roots structurally share unchanged bytes and no
normal path calls `all_updates()`. Exact frontier validation loads the checkpoint
bound to the declared document causal digest; ordinary authorship and
materialization load the current checkpoint and revalidate its one latest immutable
manifest/object anchor. Missing, truncated, tampered, misbound, or noncanonical
scratch data fails closed without live ancestry reconstruction.

An authenticated content-addressed Patricia index records portable-path ownership
under `PortablePathKeyDigest`. Occupied records bind page, exact path and exact-path
digest, acquisition batch, and causal dot; released records retain the prior page
and acquisition plus release batch/dot as reuse fences. The durable engine root
atomically binds the portable key version, current path-index root, catalog
checkpoint binding, and terminal evidence summary. Missing, tampered, or misbound
nodes/roots fail closed.

Normal acceptance reads only the union of old and fully merged prospective keys for
`SemanticEffect::pages()`: it performs point lookups, applies net releases before
acquisitions (so swaps are atomic), checks occupied points against the winning
catalog register, and publishes the candidate root only with accepted document and
status roots. A losing concurrent tombstone does not release a winning live path.
A different page may reuse a released key only in the same release/acquire batch or
when its causal frontier contains the release. Release fences remain retained after
acquisition.

A duplicate already present at the declared dependency frontier is malformed and
rejects. A valid concurrent merge that creates portable aliases enters terminal
quarantine before the conflicting document or path-index roots advance. Canonical
evidence binds key version/digest and sorted `(PageId, exact ManagedPath,
introducing BatchId)` participants, independent of delivery order. The terminal
latch blocks document reads, materialization, SQLite/projection authorization, new
projection, and recovery.

### Projection, external import, and handoff

A device-local `ProjectionEndpointId` is enrolled later against exactly one
`WorkspaceId`, `DeviceId`, and canonical graph root. A source manifested projection
intent binds its workspace, source batch/author/session/endpoint, page/path, exact
post-state frontier, `Present`/`Absent` precondition and target, target bytes and
structural annotations, claim evidence, and exact immutable annotated-base object
references. Descriptor `DocumentId`s are deterministic and domain-separated. Raw
base bytes occur only in the annotated-base object, which additionally binds the
endpoint, source page/path, prior logical completion/frontier, exact digest/length,
complete identity annotations, and claim evidence.

The author endpoint's complete projection journal and annotated bases are therefore
immutable members of the `OperationBatch` closed set before manifest commit.
Receiver-local formatting is not universal: every replica validates and retains the
source intent, but only the enrolled matching endpoint may execute it. A non-source
receiver derives a separate device-local receipt intent from accepted semantic state
and its exact local base; this may preserve CRLF or other local formatting and
references, but never mutates or gains authority from the source intent.

Acceptance validates the complete manifested projection set before hot, SQLite, or
projection visibility and may durably prepare immutable matching-endpoint rows.
Preparation also adds a batch-keyed record to the current authenticated
pending-activation root, binding the workspace, endpoint, exact manifest
fingerprint, prepared digest, and work set without making any row Ready or
path-visible. The authenticated durable engine-status transition is committed
before one projection-work root transition removes that pending record and adds
the accepted witness plus Ready/path/row changes.

Endpoint-engine startup pages only the current authenticated pending-activation
root and performs exact batch point lookups in authenticated durable engine
history. An accepted record with the exact fingerprint activates the bound
prepared set; nonaccepted terminal, absent, and restart-discarded staged records
retire it without execution. Missing, tampered, or misbound pending, prepared,
work-set, history, witness, node, or root evidence fails closed. Recovery never
scans object-store history, manifest or prepared directories, or lifetime work
roots, and does not depend on duplicate delivery. Execution still requires both
the matching endpoint and the authenticated durable accepted record plus the
projection-work accepted witness at the exact manifest fingerprint. Work is
point-keyed by endpoint/batch/page/path, deterministically ordered, idempotent
under duplicate/reordered delivery, and recovered directly. Remote-source work
creates no executable row. Same-path work is superseded only by a causally
containing frontier; incomparable work is retained.

Operations, intent, base, and work are durable before the singular guarded
`Graph` page write/removal boundary runs. Stable completion references
the immutable work/intent objects and is published only after guarded success, exact
reread/absence verification, required durability sync, and watcher accounting.
Missing/corrupt base evidence blocks unless current bytes exactly equal a replayed
target, when completion can be reconstructed. Exact base bytes and annotations are
retained without garbage collection in the first rollout.

Manifested projection intents and durable work rows bind the portable key version,
the digest for their exact path, and the accepted authenticated path-index root.
The closed manifest object descriptor authenticates that intent binding, and the
accepted projection witness retains the complete bound work row. Execution and
recovery revalidate the accepted durable engine-history root and the current exact
portable ownership point under the endpoint/session boundary before reaching the
singular graph publication call.

External reconciliation is one revalidated deterministic inventory transaction over
`Present(bytes)` and `Absent` for all managed paths. It compares exact annotated
completion bases, current inventory, and the oplog-derived affected state. Identity
matching uses valid Logseq UUID anchors first and receipt-backed structural tree diff
second; content similarity never authorizes destructive identity reuse. Unchanged or
unambiguously moved nodes retain identity; unmatched nodes receive IDs derived with
domain separation from `ImportId = H(workspace, base-completion IDs, sorted
path/hash/absence inventory, diff-schema)`. The batch ID and unmatched entity IDs are
also deterministic derivations, so replicas importing the same inventory produce one
idempotent batch. Ambiguity may conservatively become delete+create but may not alter
the imported text. Genuine two-sided divergence preserves both external bytes and
oplog state and blocks visibly.

Before a writable session, Tine durably records `HandoffUnsafe { session }`. Only
after local batches, projections, completions, and watcher events drain and all
required bytes are durability-synced may it atomically record
`HandoffSafe { dependency frontiers }`. A crash or failed drain stays unsafe and
blocks automatic external import. This is the clean one-app-at-a-time handoff barrier;
concurrent OG Logseq editing is not promised.

### SQLite, enrollment, retention, and gates

SQLite is device-local derived state in WAL mode with one operation applier protected
by a workspace process lease. Applying a batch and advancing its exact frontier is
one SQL transaction. Exact reads and write authorization require a containing
frontier, otherwise they wait or use the bounded in-memory tail. SQLite never enters
the keystroke path. Schema mismatch, failed migration, corruption, or deletion
rebuilds from the oplog. Initial materialization covers workspace/frontier/applied
batches, pages, blocks, parent/order, references, properties, tags, tasks/deadlines,
projection state, and FTS5. The first production migration moves only the existing
reverse-reference index behind its interface.

The provisional tail safety cap applies visible mutation backpressure at either 16
MiB of retained staged-object bytes or 10,000 unapplied operation batches, whichever
comes first. It is conservative, not a measured pass. P1 must measure real envelopes,
retained-tail memory, applier behavior, and backpressure before format freeze or
activation. The tail never grows unbounded or authorizes from stale rows.

Enrollment is explicit. An ordinary graph moves through `Absent -> ShadowImport ->
VerifiedLocal -> LocalActive`, or `Blocked`, only after verified backup/restore proof,
exact-byte import, replay, staged projection, byte comparison, and atomic activation.
Sharing uses one lineage: the initiator alone moves `LocalActive -> SharePrepared ->
SharedActive`; peers move `LocalActive -> Joining -> SharedActive`. All apps stop and
the provider settles first. Multiple genesis/enrollment descriptors, incompatible
evidence, incomplete batches, foreign residue, dirty unique tails, or mismatched
content block activation; independently initialized histories never auto-merge.

No operation object, manifest, intent, completion, base blob, enrollment evidence,
or backup proof is garbage-collected in the first rollout.

The numeric format-freeze/regression gates and raw evidence are in `docs/BENCH.md`.
At 1,000,000 blocks the one-page cold path opens 100 referenced home shards in
64.306 ms at 14,240 KiB and passes its 1,000 ms/262,144 KiB gate. The adversarial
1,000-page/100,000-block path opens 4,663 distinct home shards in 3,825.617 ms at
25,260 KiB and passes the separate P0A CRDT recovery/fallback materialization gate
of 5,000 ms/524,288 KiB. This recovery path is bounded by the open content and its
referenced home shards, not total graph residency.

The normal user-facing 1,000-page/100,000-block startup/hot-state gate remains
2,000 ms and 524,288 KiB, but it is a SQLite-backed P2.2 gate and is **unproven**.
This standalone Loro harness does not implement or prove normal startup, bounded
tail replay, operation-batch replay, or SQLite rebuild. One-page edit, cross-page
move, and affected-only rename remain synthetic Loro-update lower bounds.
`LocalActive` stays disabled until P2.2 and the other implementation receipts pass;
acceptance of this ADR is not production activation.

P1A.2 separately proves authenticated offline replay of 25 batches containing
1,000,000 blocks and 10,000 page operations. The measured optimized runs took
37.860 and 39.833 seconds and peaked at about 224 MiB, so the regression ceiling is
45 seconds and 1 GiB. This rare full-recovery gate replaces the earlier provisional
15-second estimate; it does not relax the 2-second normal-startup gate. A subsequent
100-block page materialization took 39.180 ms and exactly one manifest plus one
object read. The complete receipt and distinction between recovery, SQLite rebuild,
and normal startup are recorded in `docs/BENCH.md`.

## Consequences

- The recovery/fallback materializer scales with open content and referenced home
  shards rather than total graph residency. The desired normal startup/hot state is
  a distinct SQLite-backed P2.2 path and remains unproven. The synthetic rename lower
  bound touches the catalog target and a preselected deterministic referrer set; it
  does not measure SQLite lookup.
- Moving a block does not move its mergeable content between CRDT documents. Some
  page shards therefore retain losing membership evidence; owner filtering is a
  mandatory semantic step, not cleanup.
- A highly moved page can reference several home shards, but work remains bounded by
  its members/affected shards. SQLite supplies lookup acceleration without becoming
  authority.
- v1 prototype stores are visibly fenced. There is no in-place reinterpretation,
  dense-ID production phase, implicit genesis merge, direct file-first fallback, or
  first-rollout garbage collection.
- The architecture and logical format are accepted, while concrete implementation,
  persistent-byte rollout, and `LocalActive` activation remain gated by semantic
  convergence, real replay, SQLite startup/rebuild, and retained-tail/backpressure
  receipts.
- The disconnected P1A.2 engine can reconstruct old causal state and identity
  evidence from immutable batches without retaining transitive frontier closures or
  one hot claim entry per block. This does not make its run-local authenticated
  indexes a second owner authority or a production startup database.
