# Managed storage and sync contract

This document is the implementation contract for Tine's opt-in managed-storage
runtime. Direct Files is the default product path and selects a mutually
exclusive `Legacy(Graph)` runtime before graph open. When Direct Files is
selected, no code below may inspect or modify `.tine-sync`, open an oplog,
create managed scratch state, or start managed recovery.

The authoritative layout names live in
`crates/tine-core/src/oplog/sync_layout.rs`. Code must import names from that
module rather than introducing another literal. Format/schema constants remain
beside their codecs; scratch and SQLite format versions come from the pinned
`tine-storage::formats` module.

## 1. On-disk layout

### 1.1 Shared graph-local provider

The complete graph-local managed namespace is `.tine-sync/v2/shared/`. It is
provider transport, not the application's local database. Each device writes
to `outbox`; a file-sync provider delivers those immutable files into another
device's `inbox`. Tine tolerates temporary and reordered delivery and does not
interpret the mere presence of the directory as an opt-in marker.

| Relative path under `shared/` | Writer | Reader | Format | Lifecycle |
| --- | --- | --- | --- | --- |
| `inbox/`, `outbox/` | transport scaffold | `SharedProviderTransport` | directories | created on explicit activation/join; retained |
| `{inbox,outbox}/enrollment/shared-enrollment-v1.json` | initiator | cold discovery and joiner | canonical JSON descriptor v1 | immutable identity for the shared graph |
| `{inbox,outbox}/objects/<digest>.object` | publishing device | peer ingress/replay | immutable oplog object envelope | append-only; digest-addressed |
| `{inbox,outbox}/manifests/<batch>.manifest` | publishing device | peer ingress/replay | canonical batch manifest | append-only commit object |
| `{inbox,outbox}/frontier-heads-v1/<device>-<digest>.head` | each device | peer discovery | canonical JSON frontier head v1 | immutable heads; newer generations supersede discovery relevance |
| `{inbox,outbox}/publication-intents-v1/<digest>.intent` | publishing device | interrupted-publication recovery | canonical JSON intent v1 | immutable; retired only after covered publication is proven |
| `{inbox,outbox}/manifest-recovery-links-v1/<batch>.link` | publishing device | peer recovery | canonical JSON recovery link v1 | immutable |
| `{inbox,outbox}/manifest-recovery-blobs-v1/<digest>.manifest` | publishing device | peer recovery | exact manifest bytes | immutable; digest-addressed |
| `{inbox,outbox}/.part/` | provider transport | provider transport | temporary publication bytes | disposable after recovery |
| `{inbox,outbox}/removed/` | provider transport | provider cleanup/audit | retired provider items | bounded cleanup evidence |
| `{inbox,outbox}/rename-evidence/` | provider transport | provider recovery | interrupted-rename evidence | disposable after recovery |

The device-private provider journal also has `pending-publication-v1/` and
`provider-transaction.authority`; these never sync and cannot grant shared
graph authority.

Shared-provider paths and files may be owned by a different operating-system
user than the Tine process. This is normal for Android shared storage, NFS,
containers, and shared-group deployments. Unix UID equality is therefore not
an admission rule. Tine instead requires capability-relative no-follow opens,
the expected directory/regular-file kind, bounded names and sizes, immutable
content validation, and the protocol's exact descriptor/frontier relationships.

### 1.2 Device-private app data

Local managed state is deliberately outside the graph. The Tauri shell derives
a private root for the exact graph and stores the following components there.
Only the small binding is the opt-in marker; all caches may be reconstructed
from the immutable archive.

| Path below the graph's private root | Writer | Reader | Format | Lifecycle |
| --- | --- | --- | --- | --- |
| `sparse-v2/binding.json` | Tauri explicit activation/join | ordinary startup selector | canonical JSON app binding v2 | durable local opt-in; deleted on Return to Direct Files |
| `sparse-v2-recovery/` | Tauri recovery/escape flow | Tauri recovery | renamed private component trees | temporary crash recovery |
| `archive/lineage.claim` | object store initialization | every archive open | fixed binary claim | durable authority identity |
| `archive/archive-instance-v1.claim` | object store initialization | archive reopen | fixed binary claim v1 | durable local archive identity |
| `archive/objects/`, `archive/batches/` | oplog publisher/import | replay/runtime | immutable object and manifest bytes | authoritative append-only oplog |
| `archive/bootstrap-v1/{source-inventory-indexes,source-blob-indexes,source-chunks,parts,part-spans,part-object-packs,objects,evidence,aggregates,commits}/` | bootstrap import | archive open/rebuild | versioned immutable bootstrap records | authoritative bootstrap history |
| `archive/engine-history/{nodes,roots}/` | hot engine | hot-engine reopen | content-addressed nodes/roots | authoritative accepted history |
| `archive/engine-history/{engine-history.claim,engine-history.head,engine-history.transition.lock}` | hot engine | hot-engine reopen | claim/head/OS lock | current local writer/accepted-frontier control |
| `archive/engine-history/*.history-root` | hot engine | retained-history lookup | sealed root record | immutable history evidence |
| `archive/promoted-runtime.state` | promotion/recovery | runtime open | promoted state v2 | current promoted-runtime selector |
| `archive/{block-claim-index,logseq-uuid-claim-index-v1,portable-path-index-v1,page-name-ownership-index-v1,reference-catalog-v2,projection-work-index-v1}/` | hot engine | point lookup/materialization | content-addressed Patricia/work indexes | derived from authoritative history; rebuildable |
| `enrollment/sparse-storage/v2/local/enrollment/{authority-v1.claim,head,lease,records/*.enrollment}` | enrollment owner | startup/open | claim v2, record v6, checkpoint v3 | local lifecycle authority; lease is OS-owned |
| `enrollment/.../local-activation-v1.reservation` | activation | activation recovery | reservation v1 | temporary until activation resolves |
| `receipts/{projection-receipts.claim,projection-receipts.init,bases,intents,completions,attempts,forensics}/` | projector | recovery/readiness checks | projection store v5 and versioned rows | derived receipts and diagnostics |
| `receipts/.pending-cleanup/{round-0,round-1,round-robin.state}` and suffix authority files | receipt cleanup | receipt cleanup | bounded cleanup queue | disposable maintenance state |
| `archive/projection-work-index-v1/{projection-work.claim,projection-work.head,*.prepared,*.work-node,*.work-root}` | projector | projection drain/recovery | work-index v11 | derived, reconstructable |
| `archive/engine-history/resume-points/*.resume-point` | clean/unsafe handoff | promoted-runtime recovery | resume point v2 | bounded retained recovery hints |
| `reconciliation/{scan.sqlite,scan.sqlite-wal,scan.sqlite-shm,scan.sqlite-journal}` | reconciliation | reconciliation scheduler | SQLite baseline v3 | disposable |
| `reconciliation/<workspace>/<endpoint>/scan.sqlite.forensic-<uuid>/{database,wal,shm,journal,EVIDENCE_COMPLETE,REBUILD_COMPLETE}` | reconciliation recovery | cache-corruption diagnostics and crash-resumable rebuild | exact former baseline file set plus completion markers | diagnostic; never authority; created only when the disposable baseline fails semantic validation |
| `.tine-runtime/sqlite-workspaces/sqlite-applier.lock` | SQLite applier | SQLite applier | empty OS-lock file | disposable process coordination |
| `projection/materialization.sqlite{,-wal,-shm}` | SQLite applier | managed queries/navigation | `tine-storage` SQLite schema 15 | disposable; mismatch causes one rebuild |
| runtime scratch (`tine-storage::formats::SCRATCH_DIR` and its marker/lease/pages/blobs) | hot engine/import | hot engine/rebuild | scratch schema 13, page schema 1 | disposable; one run only |
| `managed-local-journal-v1/` | actor fast durability lane | drain/recovery | journal frames/segments | durable until incorporated into oplog, then reclaimable |
| `local-authorship-v1/` | actor publication | provider repair/recovery | receipt v1 | retained until corresponding publication is proven |
| `inactive-bootstrap-publication-v1/` and its sealed/aggregate/part spools | bootstrap authoring | bootstrap install/recovery | versioned bootstrap staging | disposable after installation |
| `inactive-shadow-projections-v1/{manifest.bin,proof.bin,committed.bin}` | shadow verifier | activation/promotion | shadow v2/proof v1 | retained activation proof; staging siblings are disposable |
| `migration-source-backups-v1/payload/` plus manifest/proof/commit markers | activation backup | restore/recovery | backup/proof/commit v1 | retained safety backup |
| `bootstrap-source-capture-v1/` plus manifest, sorted inventories and `source-chunks/` | source capture | bootstrap authoring | capture v1 | scratch evidence; disposable after successful activation |

Temporary prefixes (`.tmp-`, `.head-tmp-`, `.record-tmp-`,
`.authority-tmp-`) and `.staging` files have no authority until their named
atomic publication completes. Unknown canonical-looking files are errors;
recognized provider temporary files mean “delivery may still be settling.”

## 2. Enrollment and synchronization state machine

### 2.1 Actors and authority

| Actor | Owns | Never owns |
| --- | --- | --- |
| Tauri selector | private binding and explicit Direct/Managed choice | oplog truth, enrollment history |
| local enrollment owner | local lifecycle record and its OS writer lease | another device's state, graph Markdown truth |
| managed actor (`SyncRuntimeHandle`) | admitted mutations, local journal drain, archive publication, projection scheduling | authority before a validated active enrollment |
| initiator | creation/publication of the shared descriptor; initiator enrollment transition | joiner's private state |
| joiner | its own local archive/enrollment after validating the exact shared descriptor and provider cut | rewriting the descriptor or adopting incomplete provider bytes |
| provider transport | durable copy/rename/retirement of exact files | semantic acceptance; directory presence is not enrollment |
| immutable oplog/archive | managed page/journal semantic truth | assets, PDF sidecars, config/settings |
| SQLite, scratch, projection work/receipts | acceleration, reconstruction, diagnostics | semantic truth or permission to overwrite Markdown |

Authority is transferred only by a validated, durably published record while
the current owner retains the relevant lease/capability. A path name, a newer
mtime, a cache row, or provider arrival alone never transfers authority. Any
operation that observes a changed generation/descriptor/frontier must restart
from that observation instead of completing under stale authority.

### 2.2 Local lifecycle

1. **Direct / absent** — no private binding; startup opens Direct Files and
   does not inspect shared bytes.
2. **ShadowImport** — explicit activation captures source files and prepares an
   inactive immutable bootstrap. No managed graph writer exists.
3. **VerifiedLocal** — bootstrap, backup, shadow projection, and SQLite proof
   agree. Authority is still inactive.
4. **LocalActive** — promotion publishes the accepted runtime state; the actor
   acquires enrollment/archive leases and becomes the sole managed writer.
5. **Blocked / incompatible / corrupt / ambiguous** — typed terminal or
   retryable evidence; no fallback writer is silently admitted.
6. **StoppedSafe / StoppedCrashed / Terminal** — clean drain publishes a safe
   handoff; crash recovery may resume its own unsafe state, adopt a safe
   handoff, or take over a crashed unsafe state after validation.

Activation diagnostics run in this order: source capture; bootstrap import
preparation; immutable install; backup proof; SQLite open/build; shadow byte
verification; promotion/authority confirmation; reconciliation baseline and
actor open. Progress reporting is observational and never creates a timeout
fallback.

### 2.3 Sharing lifecycle

1. **LocalActive → SharePrepared (initiator).** The initiator seals and
   publishes the one shared descriptor, then records the matching local phase.
2. **Direct/explicit join → Joining (joiner).** The joiner reads that exact
   descriptor, observes two stable bounded provider cuts, validates every
   required manifest/object/recovery link, and constructs its private archive.
3. **SharePrepared/Joining → SharedActive.** Each device records its role
   (`Initiator` or `Joiner`) in its own enrollment. The descriptor remains the
   shared identity; local endpoint/device IDs remain local.
4. **SharedActive operation.** A local edit is durably journaled, authored into
   the oplog, accepted locally, projected, then published as objects → intent →
   manifest/recovery copy → covering frontier head. Peers admit only complete,
   validated batches and apply them in causal order.
5. **Interrupted transfer.** Missing/temporary/reordered bytes remain pending;
   exact immutable collisions or inconsistent stable cuts block. A retry
   resumes from durable observations rather than inventing state.

## 3. Invariants and versioning

1. The threat is crash, power loss, torn write, and interrupted/reordered file
   sync—not a malicious byte-forging actor. Content digests detect accidental
   damage and name immutable content; they are not a security authenticator.
   The sole `hmac::verify` call remains only for frozen legacy enrollment
   history compatibility.
2. The immutable oplog is the source of truth for managed page/journal content,
   IDs, names/paths, references, and properties. Markdown is a projection when
   managed mode is active. Assets, PDF sidecars, `config.edn`, and app settings
   retain their separate authorities.
3. SQLite, reconciliation databases, scratch, Patricia lookup indexes,
   projection-work indexes, and transient receipts are disposable. Deleting or
   version-mismatching one may cause exactly one bounded rebuild, never a second
   rebuild on the following open. A complete rebuild must be linear in graph
   size and finish within 10 seconds on the release corpus.
4. Authoritative bytes are append-only or atomically replaced under an exact
   observed-generation/lease check. A cache cannot authorize oplog mutation or
   Markdown overwrite.
5. Shared publication is closed: a manifest names its complete object set, and
   a frontier head may advertise it only after all prerequisites and recovery
   evidence are durable.
6. A joiner must be able to reconstruct all device-private state from a
   complete shared archive. Local app data is not synchronized and must not be
   required from the initiator.
7. Simulator/test code may import production wire/storage code. Production may
   not import the `simulator` compatibility module.
8. Direct Files remains isolated: no passive `.tine-sync` discovery, managed
   recovery, oplog write, or managed cache work occurs without the validated
   private binding or an explicit activation/join command.

### 3.1 Refusal scenarios

Every public durable refusal must carry one of these stable scenario IDs. An
internal fail-closed validation is classified when it reaches the public
open/activation boundary; it does not need to duplicate the identifier at
every decoder call site. A transient condition that is safe to retry is not a
durable refusal; a disposable cache failure must rebuild instead of appearing
in this table.

| Scenario ID | In-scope failure | Required response |
| --- | --- | --- |
| `MS-REF-CRASH-TRUNCATED` | Crash, power loss, or interrupted provider delivery leaves a canonical record or immutable object truncated/incomplete | Preserve authoritative bytes; retry if delivery may still be settling, otherwise diagnose the exact corrupt component |
| `MS-REF-DISK-CORRUPT` | Disk/media error changes an immutable digest-addressed record or authoritative lifecycle record | Refuse the affected authority transition; retain recovery evidence and identify the component |
| `MS-REF-SYNC-CONFLICT` | A file-sync provider supplies conflicting bytes for the same immutable identity or a provider cut changes during admission | Do not choose bytes by mtime; retry a moving cut or block a stable immutable collision |
| `MS-REF-CONCURRENT-WRITER` | Another honest Tine process holds the exact enrollment/archive/SQLite OS lease | Refuse the second writer while the lock is held; reopening after release must work |
| `MS-REF-STALE-GENERATION` | An honest concurrent operation advances a binding, frontier, lease identity, or generation after validation began | Abort/retry the stale operation; never publish under the superseded observation |
| `MS-REF-UNSAFE-FS-KIND` | Sync delivery, filesystem damage, or an external tool replaces an expected directory/regular file with a symlink, special file, reparse point, or unexpected hard-link alias | Refuse access through the substituted entry without following it |
| `MS-REF-MALFORMED-IMPORT` | Imported/shared Markdown, Org, descriptor, manifest, or operation bytes cannot be decoded within declared bounds | Leave source/authoritative history unchanged and report the bounded invalid component |
| `MS-REF-BOUNDS` | Honest corruption or malformed imported/provider input exceeds explicit memory, depth, count, or byte bounds | Reject before unbounded allocation or traversal and report the bounded class |
| `MS-REF-PROTOCOL-INCOMPATIBLE` | An honest device or restored graph supplies a recognized managed-storage component whose schema/protocol is newer or otherwise incompatible with this build | Preserve the component unchanged, refuse interpretation, and identify the component so the user can upgrade or rebuild from Direct files |

Every public durable open/activation refusal carries its scenario ID separately
from its bounded reason/stage code. Retryable open failures do not invent a
scenario; if a lower storage boundary detects a durable refusal it emits the
literal table ID and the public boundary preserves it.

Unix UID equality and “only the current user may write this path” are
deliberately absent. The threat model does not defend against a malicious actor
who can already rewrite the user's private filesystem, and those checks reject
honest Android shared storage, NFS, restored backups, containers, and shared
groups. Capability-relative no-follow access, exact file identity, link count,
OS locks, digests, generations, and decoders carry the in-scope invariants.

Android bootstrap durability likewise does not require the application sandbox
to authorize a filesystem-wide `syncfs` operation. Source capture, prepared
bootstrap state, and migration-backup proof share one policy: Tine uses
`syncfs` as an optimization where the device permits it; a permission,
unsupported, or invalid-operation response falls back to synchronizing every
regular file and directory in that exact app-private tree. Other I/O failures
still abort activation, and the graph projection remains untouched until the
private state has been sealed.

The pre-enrollment reservation and the first `ShadowImport` enrollment are
also reconstructible bootstrap state. On Android, Tine attempts each ordinary
file and directory synchronization, but a permission, unsupported, or
invalid-operation response from that synchronization primitive does not turn
the disposable pre-promotion tree into a durable refusal. Exact file bytes are
reread, bindings and digests are checked, and honest concurrent writers remain
serialized by the private lease; a crash may discard the incomplete tree and
rebuild it from unchanged Markdown/Org. Initial immutable names use ordinary
lease-serialized Android rename when `renameat2(RENAME_NOREPLACE)` is not an
available app-private primitive. Every non-capability I/O error still aborts,
and every enrollment transition after bootstrap promotion retains mandatory
file and directory durability.

Android app-private projection receipts retain create-new temporary-file
writes, exact-byte collision checks, file-content synchronization, and atomic
rename publication. Directory creation and immutable publication stay on
ordinary `mkdirat`/`openat`/`renameat` primitives throughout; opening the root
through Android's ordinary API and then re-entering cap-std preflights for its
children would reproduce the same false permission refusal one level down.
They do not require the hard-link-based no-replace primitive used by the generic
publisher: some Android app filesystems deny hard links even though ordinary
app-private create, write, sync, and rename operations are available. Receipt
directories are likewise opened through ordinary app-private directory handles
and classified from the retained handle, rather than requiring a preliminary
`fstatat` check or the Linux hostile-replacement `O_NOFOLLOW` primitive.
Receipt files follow the same rule: ordinary app-private open, then retained
handle type, length, and bounded-byte validation. This applies to the receipt
root as well: Android does not have to accept creation relative to a Linux
capability-style parent handle when its ordinary app-private file API is
available. Nor does Android have to permit canonical traversal through
system-owned ancestors such as `/data/user/0`: the platform-selected exact
app-private path is checked directly and retained as a directory handle. Honest
concurrent Tine writers remain excluded by the runtime lease;
a hostile process inside the same application sandbox is outside this threat
model.

Before an enrollment binding exists, projection receipts are reconstructible
bootstrap state rather than authority. If Android cannot reopen a receipt tree
left by an interrupted or older activation, retry renames it to a fresh sibling
`receipts.pre-promotion-failed.<unique-id>` diagnostic tree and initializes a
clean receipt store from the unchanged Markdown/Org source. It never traverses
or deletes an older diagnostic tree: that residue may be the very tree whose
Android access semantics caused the retry. Once enrollment has promoted the
receipt-store identity, this recovery is forbidden: normal exact identity and
receipt recovery rules apply.

The sibling `local-activation-v1.reservation` is also pre-enrollment,
reconstructible resume evidence rather than authority. On Android it is
published and reopened through ordinary absolute app-private file operations:
create-new temporary file, file sync, atomic rename, and exact-byte reread. A
permission/unsupported/invalid-operation response from the containing
directory sync does not refuse activation, because a crash may discard the
whole unpromoted subtree and rebuild it from the unchanged Markdown/Org graph.
The file write, rename, reread, canonical JSON, and binding comparison remain
strict.

The initial `ShadowImport` enrollment is likewise still pre-promotion. Its
authority claim, immutable first record, and head are each written to a synced
regular file and reopened through the existing canonical-byte, binding, lease,
and digest checks. If Android then refuses only the containing-directory fsync,
activation may continue: after a crash this unpromoted private state is either
resumed exactly or discarded and reconstructed from the unchanged Markdown/Org
graph. Every later enrollment transition, including `VerifiedLocal`, promotion,
and shared state, keeps the required directory-durability barrier. Ordinary file
I/O, locking, decoding, collision, and identity failures remain fatal in both
states.

The graph-local shared-provider tree is transport rather than local authority.
Tine still creates and opens it no-follow, requires ordinary directories and
regular files, flushes published file contents, and validates bounded bytes and
digests. On Android, inability to fsync a shared-storage directory is treated
as a platform durability limit rather than a durable refusal. App-private
post-promotion enrollment, archive, journal, and SQLite directory barriers
remain required.

Current disposable schema identities are scratch 13 / scratch page 1 / SQLite
15. Their authoritative values are `tine_storage::formats::{SCRATCH_SCHEMA_VERSION,
SCRATCH_PAGE_SCHEMA_VERSION, SQLITE_SCHEMA_VERSION}`. Bumping one invalidates
only that derived representation and costs one rebuild; it must not migrate or
reinterpret authoritative oplog bytes. Authoritative format changes require an
explicit versioned migration and cannot be treated as a cache rebuild.
