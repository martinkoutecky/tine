# Managed storage and sync contract

This document is the implementation contract for Tine's opt-in managed-storage
runtime. Direct Files is the default product path and selects a mutually
exclusive `Legacy(Graph)` runtime before graph open. When Direct Files is
selected, no code below may inspect or modify `.tine-sync`, open an oplog,
create managed private state, or start managed recovery.

One native `StorageModeSupervisor` owns storage-transition identity, priority,
serialization, and terminal outcomes. A transition has a monotonically
increasing operation ID, exact window and canonical graph root, typed kind and
phase, and exactly one native terminal outcome. Late work may publish only while
its operation remains current. Long work is never serialized app-wide:
different canonical roots have independent lanes, and graph-slot publication is
a short current-operation compare-and-publish. A stuck graph cannot block an
unrelated graph or later overwrite the newer selection in the same window. The
frontend renders events for the current operation ID; phase-name prefixes,
frontend attempt tokens, and inactivity timers are not storage authority.
Supersession is cooperative abandonment, not forcible thread cancellation: a
blocking OS worker may finish disposable computation, but it cannot publish or
change the selected mode after its operation ID becomes stale. Operation start
and final graph-slot publication share one short linearization lane; neither a
root lane nor the supervisor model mutex is held for graph-sized work. Managed
activation and join use a move-only publication guard: preparation can produce
exactly one published successor, and the post-publication guard has no API for
publishing again. Graceful Direct Files recovery uses a separately typed
multi-step guard which managed activation and join cannot acquire.

Installing a native graph slot is not sufficient evidence of stable readiness.
A managed candidate is reported successful only after its actor-backed
generation opens the exact accepted-frontier-stamped SQLite materialization,
answers its complete paged inventory, and opens deterministic representative
pages including the largest captured source page. The largest-page path is
retained during the activation capture; readiness never walks the Markdown tree
a second time. The resulting structured native receipt names inventory, sample,
and total timings. Only then may one native publication replace the exact
predecessor generation and persist the Managed selector. The frontend reacts to
that terminal native receipt by retiring renderer state; it does not re-prove
native readiness.
The SQLite candidate receives that stamp only after its authenticated page
catalog was covered exactly once and every page was materialized.
Readiness never compares against a cached Direct Files inventory captured before
the transition or the actor's live current-path catalog: filesystem delivery may
legitimately change either while startup catch-up is settling. The accepted
frontier's raw document count is not a page count because it also includes
non-page managed documents. An empty graph legitimately proves readiness with an
empty inventory. Its untouched lazy catalog has no causal dependencies, so the
immutable baseline and SQLite genesis bind zero documents; the catalog
contributes one document once pages exist.

Explicit activation is never an unexplained spinner. Before native activation,
the frontend names pending-save flush, confirmation, and progress-listener
setup; during activation it renders the active native operation and detailed
native construction progress. Fresh bootstrap reports its construction phases.
Reactivation of retained clean state separately reports marker/baseline/index
open, committed-tail replay, projection repair, and actor open. After native
success the frontend rebinds its renderer to the generation named by the native
result. These progress values are observational and cannot authorize
publication.
After managed slot publication, the watcher still performs one full handoff-gap
reconciliation. It begins immediately, but the expensive path-comparison phase
is a retained cursor with both a path-count and wall-time budget per actor turn.
That comparison reads exact accepted projection bytes; it does not replay the
parser and semantic mutation planner for unchanged pages. A differing path is
only a candidate: the ordinary external-reconciliation transaction must still
reconstruct and validate its complete semantic predecessor before authoring.
The cursor remains visibly pending and prevents `Safe` until its exact epoch is
settled; application, enrollment, and status requests can run between turns.
There is no timer-based priority claim and no O(graph) comparison turn on the
shared actor lane.

Return to Direct Files has two meanings. A graceful return drains a healthy
managed actor and confirms its committed projection before selecting Direct
Files. An emergency return is always available from managed startup/refusal.
The explicit recovery button invokes it immediately without a native
confirmation dialog that could be delayed by the failing managed open. It
atomically retires the private managed selector and opens the current
Markdown/Org tree without first opening, repairing, draining, archiving, or
recovering managed state. Managed evidence remains quarantined for inspection,
and the UI warns that it may contain operations newer than Markdown. Re-enabling
managed storage after emergency return starts from the then-live Markdown tree;
quarantined authority is never silently resurrected. Emergency return
supersedes in-flight managed open/activation/join at native safe checkpoints,
and an older operation cannot publish a managed slot afterwards. The ordinary
Settings action is always graceful: if drain or projection confirmation fails,
it leaves managed evidence selected and offers the separately named emergency
return rather than force-stopping managed authority implicitly.

The graceful return's graph-local set-aside is a durable cross-directory
rename, not merely a pathname move. It flushes `.tine-sync` before using the
`recovery` entry, including when that visible entry may be residue from a prior
refused attempt. Moving `v2` into recovery (or
rolling that move back) flushes the destination parent before the source
parent. A real directory-barrier failure is reported even though the rename may
already be visible; the retained name remains recovery evidence and retry must
reinspect current state. Filesystems which genuinely do not support directory
flush retain the shared unsupported-operation policy.

The authoritative layout names live in the pinned
`tine_storage::formats` manifest. Core code imports them through the
definition-free compatibility surface in
`crates/tine-core/src/oplog/sync_layout.rs`; it must not introduce another
literal. Format/schema constants remain beside their codecs and are likewise
certified through `tine_storage::formats`.

[ADR 0054](adr/0054-lazy-genesis-managed-activation.md) is the sole production
activation format. Existing pre-0.7 enrollment, sharing descriptors, and
multipart-bootstrap state have no production codec or lifecycle arm. They are
preserved as protocol-incompatible evidence and the product offers Return to
Direct Files before a fresh clean activation. Production contains one
baseline-plus-manifest actor constructor and one share/join state machine. The
cursor-based join state, legacy mutation slot, and legacy provider indexes are absent from production
and pinned absent by the retired-source guard. Older constructors and handoff
fixtures remain callable only under `cfg(test)` as bounded differential oracles;
they are not an alternate runtime authority. The final enumerated known-red
oracle corpus remains pending for MS-14b. A partially
implemented genesis artifact is never authoritative: only the final clean
activation marker selects the baseline-plus-manifest runtime. The exact
removal/replacement ledger is
[managed-activation-authority-census.md](managed-activation-authority-census.md).
Every constructed lazy-genesis candidate nevertheless carries the exact causal
`DocumentDependencies` for its catalog and page checkpoints. Installing that
candidate into the shadow engine constructs a sequence-zero accepted frontier
whose constant-size genesis binding commits the sealed manifest root. Its
accepted-document map is an initially empty overlay containing only causal rows
superseded by later operations; it does not copy the graph-sized genesis map.
Subsequent accepted operations must preserve the genesis binding. Test-only
legacy fixtures are not an alternate activation marker or permission to admit
a partially constructed candidate.

The clean engine's resident document and document-head maps are bounded
acceleration only. Evicting a document from either map does not turn an edited
page back into lazy genesis: a cold point load derives its current direct heads
from the complete accepted frontier and reconstructs the accepted suffix before
another save or projection check.

Production sharing likewise has one route: clean activation publishes a clean
baseline descriptor and clean joining installs that exact baseline plus its
manifest tail. The pre-0.7 share and join implementations compile only in
tests. A production decoder may still recognize an old descriptor so it can
give a bounded migration refusal, but it cannot use that descriptor to reopen
the retired runtime; the user must Return to Direct Files and share again.
The descriptor anchors the immutable enrollment baseline; it does not freeze
the state a later device must already contain. Before comparing or installing,
a clean join collects every valid descriptor-bound provider head and replays
the union of their reachable manifest tails. The joining Markdown/Org graph is
compared with that current reconstructed provider frontier, so an external edit
published after share setup cannot make an already-synchronized device look
divergent merely because it no longer matches the enrollment-time baseline.
Both successful enrollment cuts intentionally retire the actor that entered
them. Tauri must reopen the durable result, prove ordinary page inventory/load,
and atomically replace the exact predecessor graph slot before reporting share
or join success; querying or continuing to serve the retired actor is invalid.

That retirement is recorded on the handle by the cut itself, not left for the
caller to infer. `SyncRuntimeHandle::prepare_shared` on success, and
`join_shared` on completion, close the private sender and join the actor thread
before returning, so the handle's final published snapshot becomes its
authority. The two observational calls that survive a retired actor therefore
keep working: `status()` reports the actor's own last snapshot, and
`clean_shutdown()` reports `Safe` for a runtime that already reached
`StoppedSafe` — which the cut guarantees, because it commits the Safe
transaction before publishing anything. Every other request on a retired handle
is `ActorUnavailable`, which is the truth. A pre-Safe refusal from either cut
retires nothing: the actor stays reachable for an explicit retry or a
crash-style drop.

This is an availability rule, not a durability concession. `clean_shutdown`
still reports `Safe` only from a `StoppedSafe` lifecycle; it never converts an
unreachable actor, a terminal latch, or outstanding work into `Safe`. Reporting
a bare `ActorUnavailable` for a state the runtime durably reached serves no
in-scope threat, and off-host it costs a full CI round trip to localise
(Android CI run 32098261560: `clean shutdown failed: Err(ActorUnavailable)`
immediately after a successful share preparation). The Android instrumentation
receipt accordingly prints the runtime `status=` beside every save or shutdown
refusal, so an unreachable snapshot is distinguishable from a reported one.

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
| `{inbox,outbox}/enrollment/shared-enrollment-v1.json` | initiator | cold discovery and joiner | the one current clean magic-prefixed descriptor v1; any pre-0.7 shape is unrecognized protocol-incompatible evidence | immutable identity for the shared graph |
| `{inbox,outbox}/clean-baselines-v1/<root>.index` | initiator | clean joiner | canonical lazy-genesis provider index v1 | immutable; descriptor-bound; published after every baseline chunk |
| `{inbox,outbox}/clean-baselines-v1/<root>.<file>.<chunk>.chunk` | initiator | clean joiner | fixed-size exact chunk of a sealed lazy-genesis file | immutable; reassembled only through the descriptor-bound index |
| `{inbox,outbox}/objects/<digest>.object` | publishing device | peer ingress/replay | immutable oplog object envelope | append-only; digest-addressed |
| `{inbox,outbox}/manifests/<batch>.manifest` | publishing device | peer ingress/replay | canonical batch manifest | append-only commit object |
| `{inbox,outbox}/frontier-heads-v1/<device>-<digest>.head` | each device | peer discovery | canonical JSON frontier head v1 | immutable heads; newer generations supersede discovery relevance |
| `{inbox,outbox}/publication-intents-v1/<digest>.intent` | none — write side retired 2026-08-20 (`2a578d87`, D-1); read-only tolerance at `sync_runtime.rs:9950-10069` pending reader deletion | interrupted-publication recovery | canonical JSON intent v1 | no writer; nothing publishes or retires an intent |
| `{inbox,outbox}/manifest-recovery-links-v1/<batch>.link` | none — write side retired 2026-08-20 (`2a578d87`, D-1); read-only tolerance at `sync_runtime.rs:9950-10069` pending reader deletion | peer recovery | canonical JSON recovery link v1 | no writer; never created |
| `{inbox,outbox}/manifest-recovery-blobs-v1/<digest>.manifest` | none — write side retired 2026-08-20 (`2a578d87`, D-1); read-only tolerance at `sync_runtime.rs:9950-10069` pending reader deletion | peer recovery | exact manifest bytes | no writer; never created |
| `{inbox,outbox}/.part/` | provider transport | provider transport | temporary publication bytes | disposable after recovery |
| `{inbox,outbox}/removed/` | provider transport | provider cleanup/audit, and the evidence an exact repeat of a retired rename/remove settles from (§2.10c-i) | retired provider items | bounded cleanup evidence; retired against live journal state, not store lifetime (§2.10c-ii). `MAX_PROVIDER_RESIDUE_ENTRIES` remains the structural scan bound |
| `{inbox,outbox}/rename-evidence/` | provider transport | provider recovery | interrupted-rename evidence | disposable after recovery |

The device-private provider journal also has `pending-publication-v1/` and
`provider-transaction.authority`; these never sync and cannot grant shared
graph authority.

An INCOMPLETE provider tree is not an unsafe one. A file-sync tool creates the
directories above in whatever order it likes, may hold one back for minutes,
and may remove one again while it propagates another device's deletion. An
absent provider root, an absent tree, an absent namespace directory and an
absent descriptor therefore all read the same way — "no sync data here yet":
cold discovery answers `None`, the cold prefix classifier answers `Partial`,
the runtime's exact reads answer `None`, and a provider scan treats the absent
namespace as an empty one. `UnsafeProviderEntry` is reserved for an entry that
IS present and is not what the protocol requires: a symlink, a regular file
where a directory is required, a non-UTF-8 or traversing name, or an entry
that cannot be opened for any reason other than absence. Refusals name the path
on disk, not the bare component. Nothing about this relaxes what happens once
bytes ARE present: descriptor, manifest and object validation is unchanged.

The same rule governs the outbox's own children. Only a CANONICAL namespace
that is present as something other than a real no-follow directory is refused.
Every other entry there is skipped — a file-sync client writes its temporary
files and conflict copies into the directories it is delivering, and a future
Tine may add a namespace this build has never heard of. None of them is on a
path the scan reads, so none can grant authority, and refusing them stranded a
device over litter. The rule is about WHAT IS READ, so no sync tool is named in
it. (Conflict copies INSIDE a namespace remain classified by
`sync_conflict_base`, which recognizes the Syncthing, Seafile and Dropbox
formats from their upstream sources.)

`ProviderRuntime::open` creates the whole namespace inventory
(`tine_core::oplog::SHARED_PROVIDER_TREE_NAMESPACES`) in both trees before any
publication, and share preparation opens the transport before it writes a byte.
A preparation that fails at any later step therefore leaves a complete tree
with no descriptor in it, which discovers as "nothing to join yet" — never as a
half-built tree another device could act on. Any reader that claims to
recognize an untouched skeleton takes that inventory from the same constant
rather than re-listing it.

A first local activation writes NOTHING under the graph's `.tine-sync/`.
Managed storage is write-shy about the graph folder until the user asks to
share it, so the empty skeleton above appears only when the shared transport is
opened: share preparation, join, and every shared reopen. Anything reasoning
about "an untouched provider tree" is reasoning about that state, not about
activation.

The device that prepared a share owns the descriptor in its own outbox. If
something outside Tine removes it, the actor republishes it byte-for-byte, so a
graph whose sync tool propagated a peer's deletion becomes joinable again
without the user re-running setup. That republication is bounded per actor
session: one or two rounds cover an ordinary delivery window, and past that the
actor stops and reports one condition naming the file, rather than writing
against something that keeps deleting it.

Shared-provider paths and files may be owned by a different operating-system
user than the Tine process. This is normal for Android shared storage, NFS,
containers, and shared-group deployments. Unix UID equality is therefore not
an admission rule. Tine instead requires capability-relative no-follow opens,
the expected directory/regular-file kind, bounded names and sizes, immutable
content validation, and the protocol's exact descriptor/frontier relationships.

The clean shared descriptor names the immutable lazy-genesis baseline root,
its exact provider index, source capture and accepted manifest frontier directly. It contains no legacy
enrollment head, promotion proof, Patricia root, SQLite identity or persistent
projection-work state. The matching private `lazy-genesis.shared` record says
only which exact descriptor this device joined and whether it initiated or
joined the graph. Current semantic facts remain in disposable SQLite; durable
history remains the baseline plus manifest-committed operation tail.

### 1.2 Device-private app data

Local managed state is deliberately outside the graph. The Tauri shell derives
a private root for the exact graph and stores the following components there.
The Tauri binding selects the storage regime; inside the selected private
root, `lazy-genesis.marker` is the sole managed-authority commit marker. All
projection and query state may be reconstructed from the immutable baseline
and manifest tail.

**Superseded per-block stores.** Test builds that addressed one document per
block and per membership sealed their baseline with lazy-genesis manifest and
page-capsule schema 5, operation schema 9 and semantic-effect schema 7. This
build has no reader for any of them. A containing manifest that does not decode
as, or does not carry, the current schema 7 is classified
`superseded_containing_format` and refused as `MS-REF-PROTOCOL-INCOMPATIBLE`
before any receipt or checkpoint is restored. The graph-open boundary then takes
the blank-slate lifecycle of §3.1a: it archives the whole private root
byte-for-byte and automatically rebuilds the current store from the untouched
Markdown/Org tree. Journal and history bytes that never reached Markdown/Org stay
only in that backup; reconstruction does not replay them.

Live-save conflicts use a storage-mode-independent app-private protocol beside
those managed components: `<app-data>/conflict-capsules/<graph-key>.v1.json`,
where `graph-key` is the session-style sanitized graph basename plus FNV-1a of
the root path. `ConflictCapsuleEnvelope` contains the exact retained PageDto,
its load baseline, and page binding, but never Managed replacement authority.
The whole graph envelope is replaced through `atomic_write` (unique create-new
temporary file, file barrier, atomic rename, directory barrier); stale torn
temporaries are ignored and reclaimed on reopen. An envelope that does not
decode (torn or foreign bytes at this app-private boundary) is set aside as
`<graph-key>.v1.json.unreadable-<uuid>` with a directory barrier and the queue
reopens empty; it never blocks capture or resolution. Explicit resolution re-proves
the active backend's authority and durably rewrites the envelope, or removes
and directory-syncs the final file, before the frontend acknowledges success.
For a recovered Direct Files draft, the stored disk revision selects the durable
recovery path but does not authorize the next write. Review returns the revision
of the current disk snapshot it actually displays. Apply rechecks that revision
under the page lock; a subsequent external change returns `conflict.base_rev`
and preserves the newer bytes for another review. The native and browser review
adapters share this rule, covered by
`rehydrates a durable live conflict and applies the currently reviewed disk revision`
and `concord_live_save_conflict_capsule_survives_restart_and_rechecks_disk`.
This state is recovery material only: it grants neither graph authority nor a
Managed storage selection, and no byte is written into the user's graph.

| Path below the graph's private root | Writer | Reader | Format | Lifecycle |
| --- | --- | --- | --- | --- |
| `sparse-v2/binding.json` | Tauri explicit activation/join | ordinary startup selector | canonical JSON app binding v2 | durable local opt-in; its app-private name is retired with the whole private root on Return to Direct Files |
| private enrollment `lazy-genesis.marker` | clean activation/join installation | production managed open | canonical activation marker v1, including the active authority-directory `generation` | written last; sole local managed-authority selector and join commit point |
| private enrollment `lazy-genesis.shared` | clean share/join transition | clean runtime reopen | canonical clean descriptor digest plus local initiator/joiner role | device-local lifecycle fact; no semantic history or projection state |
| `sparse-v2-recovery/` | Tauri recovery/escape flow | Tauri recovery | renamed private component trees | temporary crash recovery |
| `archive/lazy-genesis.<generation>/{manifest.postcard,commit.postcard,catalog.snapshot,segment-*.pack}` | clean activation/join installation | clean open/join through the marker generation resolver | immutable baseline pack, manifest schema 7, page capsule v6, plus commit v1 | authoritative only when named by the marker; generation 0 is the fresh-store publication, and unreferenced generations are reclaimed on open. A sealed baseline whose manifest schema is not the current one is a recognized pre-0.7 containing format: open refuses with `MS-REF-PROTOCOL-INCOMPATIBLE`, which routes the store to preserve-and-rebuild (blank-slate), never to a retryable dead end; no earlier schema is decoded. A schema-5 baseline written by the retired per-block layout takes this same route: the whole private root is preserved as a backup and the current format is rebuilt automatically from Markdown/Org, without replaying backup-only history |
| `archive/operations.<generation>/{lineage.claim,archive-instance-v1.claim,objects/,batches/}` | clean local/external/provider commit and join installation | causal replay and publication through the marker generation resolver | content-addressed objects plus manifest-last batches | authoritative append-only tail paired with the same marker-named baseline generation; unreferenced generations are reconstructible join residue and are reclaimed on open |
| `archive/operations.<generation>/clean-open-checkpoint-v2/{current,payload-{a,b},generation-{a,b},capsule-v1-<digest>,sealed-v2-1-<digest>}` | clean engine actor plus one coalesced background writer | the single current-format clean managed opener and image-first cold page loads | canonical checkpoint v2; two bounded replaceable slots, one durable commit pointer, one sealed document-to-image roster, workspace/lineage/catalog binding, accepted-sequence/state-digest recovery fence, and per-document actual floor plus age/size-policy facts | disposable acceleration only; immutable images become durable before payload/generation/pointer, unchanged references are reused after manifest-binding validation without importing every document, objects outside both retained generation roots are reclaimed, and absent/torn/wrong-format/internally inconsistent state sequence-zero full-replays without migration or backup. A damaged predecessor never prevents a fresh base-zero publication, and checkpoint repair never deletes originals |
| `archive/operations.<generation>/sweeps/local-completion-index-v1/` | common own-endpoint manifested-projection executor | foreground/cold projection replay and the device-wide absence-decision map | immutable generation-named delta/compaction chain v1 | disposable local completion evidence; rebuilt from valid retained deltas when a summary is stale or invalid; removed with its enrollment era |
| `archive/operations.<generation>/sweeps/receiver-absence-summary-v1/` | foreign receiver completion/open machinery under the workspace lease | device-wide absence-decision roots | immutable generation-named summary chain v1 naming the absence-history root | disposable receiver roots acceleration; retained receipt records are truth and rebuild it |
| `archive/operations.<generation>/sweeps/receiver-absence-rows-v1/` | the same receiver open/completion machinery under the workspace lease | point-addressable absence history by exact `(PageId, ManagedPath)` | immutable content-addressed records and authenticated map nodes v1 | disposable derived index; retained receipt records are truth and rebuild it |
| `archive/operations.<generation>/sweeps/<uuid>.<20-digit-version>` | lease-owning absence-sweep coalescer and disposition actions | managed open, publication barrier, Re-apply, Keep-deletion, and Restore | append-only chain of canonical immutable full-state objects; highest valid linked version is current | authoritative disposition history; retain-all by default; a torn highest tail falls back to the preceding valid object |
| `receipts/{projection-receipts.claim,projection-receipts.init,bases,intents,completions,attempts,forensics}/` | foreign receiver projector | foreign recovery/readiness checks and the receiver half of the absence-decision map; own-endpoint open performs names-only residue reporting | projection store v6 and versioned rows | live foreign receipts and diagnostics; retired own-endpoint rows are inert, reported, and not deleted |
| `receipts/.pending-cleanup/{round-0,round-1,round-robin.state}` and suffix authority files | foreign receipt cleanup | foreign receipt cleanup | bounded cleanup queue | disposable foreign-recovery maintenance state; retired own-endpoint entries are inert and reported in place |
| configured projection SQLite file and sidecars | clean runtime | managed queries/navigation and identity preflight | current `tine-storage` SQLite schema plus disposable `projection_baselines.projection_baseline_digest` rows | disposable; writable WAL uses `synchronous=NORMAL` and fresh schema DDL is one atomic transaction; terminal publication leaves both FTS families unready, then bounded actor turns bulk-build from the stamped projection, drain the same-transaction live-edit outbox, and flip one readiness marker atomically; FTS consumers report building or use their exact non-FTS fallback until then; transaction commits are not authority or individual durability barriers; an explicit checkpoint plus atomic file-set publication establishes a reusable snapshot; missing/stale/corrupt state rebuilds from baseline plus manifests, and losing a baseline digest costs one render-and-bind, never a Markdown rewrite |
| application runtime `managed-local-journal/{clean-workspace-,projection-turns-}…` plus endpoint-scoped recovery-input storage | foreground authoring, projection-only producers, and below-floor provider custody | managed cold open, actor drain and full-history reconstruction | three independently sequenced `LocalJournalSegmentV2` domains | authoritative until each domain's independent checkpoint advances; recovery input additionally waits for successful actor reinstall |
| application runtime `move-episodes/` | correlated multi-page operation | idempotent retry/reopen and accepted-response acknowledgement | immutable episode sidecars | retained until the frontend installs the committed source/destination pair, then retired; interrupted pre-ack evidence remains replayable |
| device-private provider journal | clean shared publisher | interrupted provider publication | bounded publication/recovery records and lock; `completed/` bounded by live provider state, not store lifetime (§2.10c-i) | private transport recovery; never semantic authority |

Emergency return publishes the sibling app-private selector
`storage-mode-selections/<graph-digest>.direct-v1.json`. Ordinary startup checks
this before managed binding discovery, so retained managed bytes cannot
resurrect themselves. The receipt is retired only after an explicit fresh
managed activation has quarantined the former private root and published its
new binding. Both selectors are small app-private sole-writer authorities:
create, exact replacement, and active-name retirement go through
`DurableDirectoryPublication`. On Windows this means certified write-through
name operations; on Unix/Android the initial parent chain and final name are
flushed before success. Retirement first moves the exact selector bytes to a
fresh same-directory name outside the selector grammar, then removes that inert
residue, so a crash cannot turn a reported Managed activation back into Direct
Files merely by resurrecting the old active name.

**Refusal scenario:** `MS-REF-STORAGE-MODE-PUBLICATION-UNAVAILABLE`. The
in-scope failure is a crash or power loss after Tine reports an explicit storage
mode switch. If the platform cannot prove the required durable name operation,
or the exact app-private selector changed during publication, the transition
refuses before acknowledging the new mode. This protects authority selection,
not against an adversarial local writer.

The following path families are **retired pre-0.7 artifacts**, not an alternate
production layout: `archive/bootstrap-v1/`, `archive/engine-history/`,
`archive/promoted-runtime.state`, the block/name/path/UUID Patricia indexes,
`archive/projection-work-index-v1/`, `archive/reference-catalog-v2/`, the old
multi-record enrollment tree and reservation, `reconciliation/`, runtime
scratch, `local-authorship-v1/`,
`inactive-bootstrap-publication-v1/`, `inactive-shadow-projections-v1/`,
and `migration-source-backups-v1/`. Production
open never treats any of them as authority. Their production construction and
recovery routes are physically removed; negative contract tests and the frozen
pre-0.7 failure corpus may still name the formats. A real graph containing only
this state is refused and can Return to Direct Files for a fresh clean
activation.

MS-14b retired the only production engine constructor that opened the native
Patricia indexes, deleted their physical store/opening branches, and removed
the detached/inactive-bootstrap protocol closure and its representation-only
tests. The producer census pins the former entry roots, Patricia openers, and
`PatriciaIndexStore` absent from production. The live run-local identity maps,
clean source selection, clean lazy-genesis materialization, and sealed accepted
history/checkpoint bindings remain; they are semantic runtime state, not a
reader, writer, migration, or alternate bootstrap activation route.

The final MS-14b closure also removes the former scratch-backed document,
dependency, causal, evidence, and Loro stores and the separate physical
engine-history control. Store-backed engines retain only nonterminal staged
payloads in hot memory; terminal payloads are reloaded exactly from the
immutable operation archive when needed. Compact accepted statuses and event
evidence remain inline. After a marker-selected generation, complete
block/home, Logseq UUID, portable-path, and page-name acquisition/release
evidence is held in four canonical point maps over the shared sealed
authenticated-map store. Only current claims and the accepted tail are rebuilt
resident; release-only rows remain point-addressable on disk. Admission
composes the speculative local overlay, accepted tail, current claims, and a
sealed point answer without enumerating a complete map or reading an accepted
archive manifest/object. Exact historical frontier questions reconstruct from
retained semantic accepted evidence. The dependency is certified
`tine-storage v0.12.0`. Its supported-target guards include Linux, Windows,
macOS, iOS, and Android for both exact-file and whole-directory no-clobber
publication.

### 1.2a App-private immutable plugin packages

Installed plugin packages live below the app-data `plugins/` root as
`<plugin-id>/<version>/{manifest.json,plugin.wasm}`. The package is immutable:
same-version/same-bytes installation is idempotent, while the same version with
different bytes is refused. Manifest and capability policy stay in the Tauri
plugin layer; physical publication and removal use the certified
`tine-storage v0.12.0` package protocol.

Publication stages a complete directory at the store root under
`.install-<id>-<version>-<pid>-<sequence>`. Each file and then the staging
directory is synchronized before a native no-replace move installs the version;
Unix synchronizes both changed parents and Windows uses its certified
write-through name operation. Retirement durably moves the active directory to
`.retired-<id>-<version>-<pid>-<sequence>` at the store root before recursive
reclaim. Both transient grammars are disjoint from plugin ids because valid ids
cannot begin with a dot.

Every plugin-store open reclaims `.install-*` and `.retired-*` entries and any
active package directory lacking the exact two-file regular-file shape. This is
recovery, not interpretation: fully shaped packages still undergo the ordinary
bounded manifest, identity, symlink, and WebAssembly validation. Uninstall
first clears selection/settings through the audited settings replacement, then
retires package bytes. A crash at that seam therefore leaves cleared settings
and either a complete unselected package that can be retired on retry, or an
already retired/absent package; no state requires manual filesystem surgery.

Temporary prefixes (`.tmp-`, `.head-tmp-`, `.record-tmp-`,
`.authority-tmp-`) and `.staging` files have no authority until their named
atomic publication completes. Unknown canonical-looking files are errors;
recognized provider temporary files mean “delivery may still be settling.”

### 1.3 Direct Files disposable graph projection

Direct Files stores one app-private
`direct-files-projections/<canonical-graph-path-digest>.sqlite` database outside
the graph. It contains only the same parser-derived physical page, block, task,
property, tag, path-ref, property-atom, and search facts accepted by managed
storage's disposable projection; it contains no binding, oplog frontier, sync
role, or authority stamp. Markdown/Org remains the sole Direct Files authority.

**Path refs and property atoms are one producer, not two.** `block_path_refs`
holds each block's reference closure — its own normalized refs, every
ancestor's, and the page's own normalized name — and `property_atoms` holds each
property element already flattened, de-duplicated by `atom_key` (first spelling
wins) and renumbered `0..n`. Both backends and the in-memory tree walk obtain
these from the SAME closure and the SAME atomizer over the SAME per-block input
(`refs_norm`, the parser's own normalized refs). No backend may grow its own
copy that merely agrees by inspection: a graph has one behaviour, whichever
storage mode holds it.

**Planning is not a task.** `block_planning` carries a block's `[#A]`,
`SCHEDULED:` and `DEADLINE:` facets for every block that has any of them,
written from the projection's three fields alone and never conditioned on the
task marker. `tasks` cannot answer these questions: a row is written there only
when a marker exists, so a markerless `SCHEDULED:` block is absent from it while
the tree walk still evaluates its date. The `scheduled_day`/`deadline_day`
columns hold the `yyyymmdd` ordinal and are NULL when the timestamp text is not
a calendar day, so a malformed date keeps its presence and loses only its day.

**The query columns are the exact visible text.** `block_text.query_visible` is the
block's visible text byte for byte and `blocks.query_visible_folded` is that
text canonically folded. Content predicates read the folded query text. The
`searchable_text` and its FTS stay whitespace-collapsed for the existing search
consumers and are not a substitute: a phrase query has to be able to tell `a  b`
from `a b`. Both columns are populated at WRITE time by both producers, never by
parsing or hydrating rows during a query.

**Query rows stay narrow (schema 26).** `query_block_results`
stores public result identity, tree preorder, construction estimate and tag/
property counts without duplicating raw payload. `block_own_refs` stores own
normalized reference names; `query_page_order` stores Direct session positions.
All are rebuildable facts in the page transaction with explicit FK-off cleanup.
Direct full reconciliation reuses projected facts for unchanged source revisions
and reconciles only the inventory-order table. Identical order performs no order
writes. Live page deltas capture retained/append/remove positions before queue
coalescing. The shared runtime-ID helper reproduces fresh-session IDs from page
path and structural order; live identity mappings and metadata-backed result
consumption are subsequent packets. Managed supplies existing public identity
and uses path order. Authority capture input formats are unchanged. Removing
global parsing from warm startup, streaming cold initialization, and result
consumption remain subsequent work; DB reuse alone does not claim fast end-to-end
startup. Direct source-revision table shape includes the schema26 marker so
older projection readers reject the newer disposable cache and rebuild it.

`pages` holds identity and routing;
`page_text` owns preamble and search text. `blocks` holds structure, metadata
and folded query text; `block_text` owns source content, original query-visible
text and search text. Keyed payloads are written in the same transaction as
owners and removed on replacement, deletion and reset. Existing typed payload
reads retain their fields; missing payload is an error, not an omitted entity.
Projection-only filtering does not load these payload tables.

**The result read is a descriptor read plus payload batches, and a damaged one
fails.** One ready query's public result is constructed from ONE owned read
snapshot, in two stages. The DESCRIPTOR read wraps the compiler's selected-id
relation and adds only ordering and result metadata — `query_block_results`,
the page's display fields, `blocks.order_key`, and `query_page_order.position`
for Direct — with no raw text, tag or property payload; every join in it is
LEFT, so a missing `query_block_results`, `blocks` or `pages` row, a Direct
result page with no `query_page_order` position, a `query_block_results.page_id`
that does not own its block's page, or a `pages.text_kind` outside the two
written values FAILS the read rather than dropping a selected descriptor. Cross-
page order is `query_page_order.position` on Direct and `pages.path` under
SQLite's default BINARY collation on Managed, then `query_block_results.preorder`
within a page. The PAYLOAD read then runs for ADMITTED ids only, in batches of
128 bound ids and exactly three statements per batch — block/text/task/planning
facets, then `tags` by owner and ordinal, then `properties` by owner and ordinal
— never one statement per block and never a tags×properties join. Each batch is
validated for exact id coverage, page ownership, the stored tag/property counts
and, finally, the stored construction estimate against the estimate of the DTO
actually built; any violation abandons the WHOLE result with no partially
substituted rows. A read that fails this way is a rebuild request, never a
shorter answer (D-3).

**A tag key is a page key.** `tags.tag_key` is `refs::page_key(tag)` — the same
key page identity uses — because `#x` is OG's `[[x]]`; `tags_lookup_idx` leads
with it. `pages.journal_day` is the journal page's `yyyymmdd`, derived from the
page's own file stem under the graph's `:file/name-format` and journal formats,
and NULL for every other page.

**The projection has a statement seam; nothing else does.**
`PhysicalProjectionQueryReader` runs caller-supplied SQL against the graph
projection with bound parameters, plus `EXPLAIN QUERY PLAN` for the same
statement. Raw SQL crosses that boundary; **authority does not.** It is allowed
here, and only here, because the projection is a disposable cache derived from
the oplog: a malformed statement fails a read and can never corrupt truth. The
oplog, the frontier and the Markdown/Org tree keep their curated typed
boundaries and must never gain such a seam.

The restriction is the **engine's**, not a validator's. The reader owns a
connection opened `SQLITE_OPEN_READ_ONLY` and no constructor accepts an existing
writable handle, so it cannot be reached from one; SQLite itself refuses every
write through it, which
`the_query_seam_can_read_the_projection_and_cannot_write_it` proves by
attempting `DELETE`, `INSERT`, `UPDATE`, `DROP` and `CREATE` rather than
assuming. There is deliberately **no** SQL-text parser or "single SELECT only"
check: it would be a runtime refusal with no in-scope failure to name — the
shape this contract's refusal table exists to keep out — and it could reject a
legitimate statement. Values travel as bound parameters in the signature, so an
interpolated statement is not expressible, and `explain_query_plan` binds the
same parameters as the query it explains, because with `sqlite_stat4` present an
unbound explain can report a plan for a statement the caller never runs.

**This seam adds no SQL-text refusal.** A missing or stale projection is caught
by lifecycle admission before a statement; a corrupt or otherwise failed read
is reported to the public dispatcher, which owns the one-repair rule below.
SQLite lock contention uses `busy_timeout`, while exhausted query-job capacity
is the separate typed `NotReady(Busy)` outcome.

**A query job owns its snapshot; the projection owns the jobs (R3).** A
database-answered query runs on `PhysicalProjectionQuerySnapshot::open_direct`,
one read transaction pinned for the job's whole descriptor-and-payload read so
every row it returns describes one projection state. The projection admits at
most `DEFAULT_QUERY_JOB_CAPACITY` jobs at once and **capacity is acquired before
the snapshot**, so a waiting job pins no WAL pages; the snapshot is validated
against the exact graph cache generation before and after SQLite establishes
the transaction, and a job whose generation moved is `NotReady`, never a stale
answer. Every admitted job registers its interrupt handle with the owner, and
**the worker drains every job before a rebuild touches the file**: it cancels
each registered statement, cancels waiters and late registrations, and blocks
until no slot is held, so no owned snapshot can retain a handle to a file about
to be reset or replaced (in-scope: a torn projection rebuilt under a live
reader). Closing the projection refuses every later admission. Cancellation is
a typed dispatch answer, not a failed read: it schedules no recovery or retry.

**Result identity follows who lowered the row.** `query_block_results.result_id`
is the runtime id the lowering process assigned. The projection tracks
`session_pages` — exactly the pages whose stored ids are LIVE in this process:
a full snapshot's replacements and each live save's page add to the set; a
structural relowering (a warm-stream replacement parsed in isolation) and a
deletion remove the page; dropping the parsed cache clears the set, because
the ids it held are no longer reachable. The set is captured with each
snapshot. A row on a session page answers with its stored id; every other row
(a warm reopen lowers none of them) answers with the structural runtime id the
shared helper reproduces from page path and structural order, which is the id
a fresh parse assigns it.

**Warm validation from bytes, never from a parsed graph.** Opening a Direct
Files graph validates the projection against the walk inventory and each
page's exact content revision computed from file bytes, parsing nothing. An
unchanged graph is READY with no parsed cache and nothing retained. A changed,
missing, damaged or config-mismatched projection names its replacement pages
and the caller streams them through a bounded high-water queue, parsing each
page in isolation and retaining none. Live saves and deletions enqueue their
delta whether or not a parsed cache exists, but readiness is published only
after this session has validated the complete inventory once (a full
snapshot, a clean warm, or a closed stream) — a delta alone never publishes an
inventory this process has not compared to disk. The `query_page_order` table
is reconciled by the worker from the queue's own page order whenever a stream
closes or a delta arrives without a position. In-scope scenario: an external
edit between two sessions, followed by a save of a different page before the
warm completes.

**One parse config, or a rebuild.** Six graph-config facts decide those derived
rows — `:property/separated-by-commas`, `:ignored-page-references-keywords`,
`:block-hidden-properties`, `:journal/page-title-format`,
`:journal/file-name-format` and `:file/name-format`. Their digest is stamped
into the projection's
`materialization_stamp.parse_config_hash` when the database is created, and
every open route compares it. A mismatch is a benign, expected outcome of the
user editing `config.edn`, not corruption: the projection is discarded and
rebuilt from the untouched Markdown/Org tree, never migrated in place, and no
forensic evidence is preserved. Direct Files reaches the same end differently —
reconciliation compares only source revisions, so the same digest is folded into
each page's `projection_source_revision`, and a config edit therefore re-lowers
every page even though no file byte changed. That config travels **inside** each
queued Direct Files work item rather than beside the queue, so a page can only
ever be lowered and stamped under the config it was queued with: there is no
state in which queued work exists and the config describing it does not, and
therefore no way to stamp default-lowered rows as current.

Direct editor replacement briefly retains the old live inode as
`.<target>.<pid>.<sequence>.editor-recovery` and the proposed bytes as the
matching `editor-staged-recovery` name. Checked Direct Files open reconciles
only that complete producer shape through retained no-follow capabilities. If
the live target is absent and exactly one artifact claims it, that exact inode
is restored with no-replace. Multiple claims for an absent target remain in
place for explicit recovery; when a live target exists, every artifact is moved
unchanged to typed conflict trash. Every move rechecks the artifact's physical
identity and single-link status immediately before publication. A suffix
lookalike, symlink or reparse point, multiply linked file, ambiguous claimant,
or failed identity recheck is never deleted or selected as authority.

Every Direct Files create, live-name retirement, staged publication, recovery
restore, and recovery set-aside is one exact-byte name transition
(`move_graph_text_exact_no_replace`): the source must still hold the expected
bytes, the graph tree's own no-clobber rename publishes it
(`renameat2(RENAME_NOREPLACE)` through the raw syscall on Linux and Android,
`renameatx_np(RENAME_EXCL)` on Apple platforms, `FileRenameInformation` with
`ReplaceIfExists=false` on Windows), the parent directory barrier is required,
and the published name is re-read to prove what became visible. The staged
inode is flushed before it can become live. The typed
`DurableDirectoryPublication` boundary of `tine-storage` is NOT used on this
path: its Android arm is hard-link-then-unlink, which the FUSE-backed shared
storage a Direct Files graph lives in refuses (GH #466, v0.6.981: every
Android save failed with `Permission denied (os error 13)`). That boundary
remains the primitive for app-private sole-writer authorities such as the
storage-mode selectors, where hard links exist. Tine never acknowledges the
save merely because an ordinary rename became visible.
Successful replacement retires the displaced recovery name through a typed
`.editor-retired` name before deletion, so a crash cannot turn an unflushed
name transition into a reported durable save. That exact producer-shaped name
is cleanup-only: it never restores a document or becomes a conflict claimant.
The same bounded no-follow checked-open walk removes any copy left by a crash
or failed foreground unlink; a cleanup error fails that open without changing
the artifact and the next checked open retries it.

For an existing Direct editor save, the initial exact-file read supplies the
serialization baseline. The late external-writer proof is the atomic
retirement itself: after the expected physical owner is detached from the live
name, Tine reads that retained inode and compares it byte-for-byte with the
baseline before publishing. A mismatch restores the same inode when possible
and mints a conflict from the retained snapshot. There is no separate
pre-retirement full-file reread; creates, unpinned auxiliary writes, and managed
projections keep their independent recheck rules.

One background SQLite owner accepts either an already-resident
`PageEntry + Arc<Document>` snapshot or the bounded warm stream. The database
retains each page's exact caller-owned content revision together with the Direct
fact-extractor version as disposable adapter metadata. Bumping that extractor
version forces one background re-lowering when unchanged source bytes acquire
new physical facts. Warm validation compares the complete byte-derived source
inventory, parses only changed or missing pages in bounded batches and retains
no parsed graph; a clean reopen lowers none. One-page cache upserts and deletes
enqueue coalesced page deltas. The editor, watcher, and save paths never wait
for SQL. Indexed reads are admitted only when the worker has published the
exact current graph cache generation. One app-private sidecar lease permits
only one graph instance to publish into a projection database at a time, which
prevents an older instance from replacing facts behind another instance's
locally-ready generation watermark. A public query against a missing, stale,
corrupt, incompatible, leased or unwritable projection follows the typed
dispatch and bounded repair rules below; the remaining parser-owned navigation
and search consumers may use their existing fallback. Neither case blocks
graph open, save or external file observation.

The switched read families are literal fuzzy-search candidate
selection (including the `((` picker), and the original-case referenced-page
inventory used by autocomplete and navigation. They also include the shared
property-facet rows used by the query builder and editor autocomplete, and the
PageRef simple-query candidate plan in the independent test oracle. Both production
query backends now select results through SQL. The switched families further include page aliases and
real-page ownership, explicit backlink and safely tokenizable unlinked-reference
candidate selection, persisted/runtime block-identity lookup, block-referrer
candidates, and distinct-referrer counts. Once current, these families
obtain a generation-bound
candidate/name set before applying the existing parser-owned matching and
presentation semantics. They no longer use manual whole-graph candidate scans
or second in-memory alias, reference-candidate, block-identity, referenced-name,
or block-ref-count semantic caches as their ordinary route.

There is no separate sparse task-query read family. It was a marker-narrowed
candidate stream that SQLite enumerated and the parser evaluator then
re-evaluated block by block, admitted by its own `sparse_task_query_eligibility`
gate. The Managed runner was retired first; the RET2 correction deleted the
gate, its two source guards and the parser sparse runner once Direct Files
answered a task query through the ONE lowered statement below instead of
pre-filtering a walk. A task query is now simply one shape that statement
answers, and it is admitted by the lowering, not by a second opinion about
which shapes are enumerable.

TQL publication indexes its immutable captured documents with the existing
Direct projection producer only when an authorized page contains a TQL macro.
Setup failure aborts before publication replacement. The snapshot owns its
temporary root, and the writer retains that owner until its connection and
exclusive lease have closed. Expiration of the bounded close wait cannot remove
the root beneath an active writer. A real paused-writer test checks retention
through timeout, release at exit, and late registration after exit.
The root is created with mode0700 on Unix. Windows supplies a protected
inheritable owner-rights DACL at creation and reads it back before indexing;
unsupported or unexpected ACLs fail setup while the directory is still empty.
Existing directories are never reused or removed after a create collision.
The projection writer reports stale indexed reads without claiming that query
execution will fall back to traversal; public query adapters own typed recovery.
Bounded block-referrer results are semantically bounded, not merely count
bounded: managed candidate discovery covers the complete generation-bound
candidate set, groups it in the same relative-path/document order as Direct
Files, and only then applies the shared row and byte construction budget.
`groups`, `total`, and `exceeded` therefore describe the exact same prefix in
both modes; an internal-ID-ordered early subset is not an allowed optimization.
The bounded generation-keyed memo of already-shaped frontend result DTOs
remains Tine-native for the reference-result families that still hydrate parser
DTOs; it is separate from the Direct public-query route below. For those
not-yet-migrated navigation and search families, an unavailable candidate or
name read may still use the established parser-owned fallback. Referenced-name
fallback walks only an already-parsed page cache and deliberately retains no
separate semantic memo. Non-UUID `id::` values and names that cannot be safely
narrowed by SQLite tokenization also use that parser fallback. Friendly graph
search likewise still ranks and produces evidence from parser-projected blocks:
the Direct projection narrows the single literal-fuzzy block shape when ready,
while the other Friendly shapes still use the parsed page source. These routes
remain explicit migration work; they do not authorize a fallback from the
simple, advanced, page, registry, or Explain public-query dispatch below.

**The Direct public-query route has no production tree-walk fallback.** A
semantically refused source returns its existing empty or unsupported-report
answer before any job or snapshot. Otherwise simple `{{query ...}}`, advanced datalog,
`@block`, `@page` and Explain reads enter `dispatch_direct_query`. The public
registry returns its published snapshot only when the graph generation, parse
config and declaration state still match; a refresh enters the same dispatcher
and query-job owner. A ready block query lowers once, wraps the selected ids in
the descriptor read and loads only admitted payload; a ready page query uses
the corresponding ordered page statement. Explain runs all of its probes
through one owned query job. There is no cost test and no selectivity hatch in
front of this route. The lowered selection chooses the ANSWER rather than a
candidate superset. Selection may inspect substantial predicate data; output
payload construction is restricted to admitted results.

Every other dispatch outcome is typed. Capacity pressure is
`NotReady(Busy)`. A worker that is already reconciling, rebuilding or applying
a delta reports its current `NotReady` reason. A stopped or unattached
projection is `Unavailable`; cancellation is `Cancelled` and schedules no
repair. An idle stale projection or an attempted failed read gets at most one
bounded repair and one SQL retry. If that retry still cannot answer, the caller
receives the resulting typed readiness or unavailability error. None of these
branches evaluates the parsed graph or fabricates an empty success.

**There is no fourth shape for "the compiler would not lower this."** The
lowering is TOTAL by type: it returns a statement for every query the IR can
represent, so no evaluable shape is routed to the walk by the compiler's own
choice. The two families that used to be excepted here — a valid `content
regexp` pattern (both the `content regexp <p>` spelling and a whole-query
`/pattern/`), and a `refs` nested inside a `children` quantifier — now lower like
any other predicate. Regex matches through a bound compiled-pattern ID that the
statement installs on the read-only seam and the seam evaluates against the
block's exact visible text; the pattern itself is never part of the SQL and never
logged. A pattern that does not compile stays a FALSE leaf, negation included,
exactly as the walk answers it — that is an answer, not a decline. Regex remains
deliberately UNINDEXED (there is no content index to bind it to), so it never
bounds an anchor by itself; a nested `refs` reads the ANCHOR's ancestor context
and so cannot bound its anchor either. Both facts are recorded as plan classes
rather than as exemptions, because an unbounded plan that is measured is
information and an unbounded plan that is excused is not.

Context-dependent child relations use a statement-local materialized map of
child and parent IDs. SQLite builds a temporary parent lookup rather than
scanning every child for every anchor. This setup can read all structural IDs;
it does not duplicate text or construct output payload. The durable schema is
unchanged. Regex program IDs distinguish the effective patterns of both
syntaxes, and missing required visible text fails the read.

**Direct current-snapshot reads.** Live queries do not capture saved-edit
targets, wait for coverage or require the projection to match the latest source
generation. Query execution reads the current complete SQLite image.
It uses the existing bounded producer capture queue to open the current complete
image, with actual SQL revision, session identity, parse configuration and
registry inputs captured together. Incomplete initial/rebuild images are refused;
ordinary queued edits do not make the committed image unusable. Full-text
readiness is read from the same transaction. Closure and replacement continue
to use the existing query-job lifecycle and cancellation owner.

**Committed-image refresh.** A Direct serving-image publication increments an
opaque notification counter after SQL, session identity, registry and readiness
publication, then wakes the existing application watcher control channel. The
watcher observes it per window binding and emits `query-projection-changed`.
The frontend rejects retired bindings and bumps ordinary dataRev, without
reloading live editors or changing page inventory. No query waits on or consumes
this counter; it is not a saved-edit target or answer-cache key. This covers
an early coherent query followed by later queued writes and rebuilds whose
physical SQLite revisions repeat. Inotify mode needs no periodic poll.

**Query answer ownership.** Direct and Managed simple and advanced queries
construct an operation-scoped answer from their acquired SQLite snapshots. The producer retains
no query answers and manages no answer-cache invalidation. SQLite owns database
page caching. Sharing result groups within an operation does not retain answers
across requests. UI retention of displayed results during refresh is independent
of this backend read path. Non-query reference caches keep their existing policy.
Repeated valid requests execute SQL again; only the displayed UI result may
remain mounted while its replacement is loading.

**What one dispatched query reads.** Capacity is acquired before SQLite opens
the owned snapshot. The snapshot is validated against the complete producer
image before and after its read transaction starts, then its
interrupt handle and the snapshot-scoped `session_pages` identity set are
registered with the job owner. A property-bearing Direct query captures its
committed registry cache alongside the SQL snapshot, using the actual storage
query revision and parse config. A query without a property leaf carries no
registry capture and does not clone dirty keys or scan the published registry
base; asking that registry-free job for registry state is an invalid-snapshot
error. This owner is separate from the editor registry.
The first read builds the registry from SQL; successful page deltas invalidate
only changed property keys and declaration-page dependencies. The producer
compares touched-page metadata before writing with its physical materialization
inputs, releasing the maintenance snapshot before the write. Text-only edits
with unchanged metadata reuse the registry without a full or per-key scan.
Full initialization, recovery, and config replacement invalidate the cache.
All turn writes and identity publication precede cache revision publication;
failed turns discard the owner. Query workers build or patch on their owned
snapshot, validating its actual revision even on cache hits. Older coherent
reads cannot overwrite newer publications or clear newer dirty keys. A
query without a property leaf uses the empty registry because no predicate can
observe its types. The answer then uses one descriptor statement plus payload
batches for the ids the budget ADMITS — never a candidate superset, never a
page the answer does not contain, and never a page document. Ready query selection
and result construction load NO `Document`, read NO source text and consult no
parsed graph. Recovery source-inventory work is counted separately. The page's name, kind and journal day
come from the projection's own page row, and a projection that cannot account
for a selected descriptor fails the whole read rather than serving a shorter
answer.

**A block statement answers `(block_id, page_id, path)` and nothing else, and
its match set reads `blocks` alone.** The block identity is the answer, the page
id is the stable routing identity a managed overlay addresses a page by, and the
path is the Direct Files order key the descriptor read joins on; the page's
name and kind come from the descriptor read's page row for the ANSWER, never
from a column carried alongside every candidate row. The materialized production spelling joins `pages` for result routing
only on the ANSWER — after matching and after the
result-set rule has dropped a match whose immediate parent also matched — because
a page predicate carries its own `pages` subquery keyed by `page_id` and
`blocks.page_id` is a NOT NULL foreign key, so joining during candidate
selection could neither add nor drop a match. Both halves are load-bearing:
carrying two unread columns and probing the pages primary key once per candidate
is the unrequested work the projection exists to remove, and it measured 0.81 –
0.98× of the previous statement across broad and selective nonempty shapes on
the anonymized corpus, with controls recorded in `RECEIPT-db1.md` (under 1% for shapes above
25 microseconds). Page-anchored statements are unaffected and still answer
`(page_id, name, text_kind, journal_day, path)`. A row that does not have its
required shape is a failed read, never an empty answer. The lowered selection
statement itself has no ordering metadata. Its descriptor wrapper does: Direct
block answers carry `query_page_order.position` and
`query_block_results.preorder` and end with `ORDER BY` on those columns; the
page wrapper carries the same page position. Missing Direct order metadata
fails the read, because silently moving a page would change which rows survive
a bounded budget.

Page results carry physical graph-relative `path`, name, kind, optional journal
day and authored ordered properties. Their shared descriptor wrapper applies
the complete saved sort and `COUNT(*) OVER()` before its row limit. Unicode
text sorting reuses Rust lowercase through operation-owned rank callbacks;
numeric-looking property values remain lexical. Explicit sort ties use physical
path, while unsorted reads retain Direct inventory or Managed path order.
`query_page_results` supplies raw construction estimates and property counts;
only admitted owners receive property payload reads, in batches of 128, with
ownership, ordinal, count and estimate validation. Both row and byte limits
apply. `total` counts admitted pages before sampling; optional `matched_total`
reports the complete SQL match count, independent of limits and sampling.
Page navigation uses the physical path even when display names coincide.
External watcher reconciliation uses the ordinary page-delta producer even
when the full parsed cache is absent. Repeated delivery of an already admitted
revision/config uses the existing session identity record to avoid another
delta; no query answer or saved-edit target is retained for this purpose.
Recency selection may read file metadata and is measured separately from
output payload. Cancellation during selection or hydration returns no partial
page answer. This does not yet supply complete SQL grouped statistics.

The FTS-readiness signal the content predicates' bounds
depend on is probed through the same seam and remembered once per generation,
never once per query — and only a READY observation is remembered, because
readiness is monotonic within one projection file while a rebuild publishes a
new generation.

**A failed read repairs once, then reports the SQL outcome.** The in-scope
scenarios are §3.1's: a torn or truncated projection file after a crash or power
loss, a disk error, a resource limit, or a projection whose page set has drifted
from the current graph generation. Because the projection is disposable,
`dispatch_direct_query` requests a rebuild only after the failed job and its
owned snapshot have dropped, then retries the SQL route once. The worker drains
all old query jobs before it resets the file. If a complete parsed snapshot is
already resident, recovery may enqueue it; otherwise recovery validates a
complete source inventory from bytes and streams bounded per-page replacements,
without constructing or retaining a whole parsed graph. Clearing readiness
alone would strand the projection until another edit. Cancellation is excluded
from repair because the drain or close deliberately removed the snapshot's
subject.

Managed production queries no longer construct a candidate-page plan or apply
its selectivity cutoff. A simple query is parsed once; invalid input returns its
semantic empty refusal before any snapshot or registry acquisition, while valid
IR enters the captured SQL route. Candidate types and lowering remain test-only
for the independent oracle. Production metadata cursor reads and oracle lowering
share the unchanged `query_cursor::drain_after` advancement and batch-retry owner.

**Managed live queries read one current main snapshot.** For simple, IR block/page, Explain and supported advanced queries, one short actor turn captures immutable inputs: the actual database frontier root and canonical digest, parse config, query/view, execution day, lifecycle epoch, and an optional committed-registry capture. The current root is the complete SQLite image already available; it is not the latest required editor or synchronization target. The calling thread acquires job capacity before it opens one main SQLite snapshot. `open_managed` validates the captured acceptance sequence and frontier digest inside that read transaction, which serves all selection and payload statements. A concurrent commit before opening may require bounded recapture; a later commit does not invalidate the coherent read.

All registry construction and query execution run outside the actor and cache lock. Property-free queries skip registry acquisition. Property queries reuse the existing inferred/declared type logic and committed registry owner, building or patching affected keys against the same validated snapshot. The normal accepted-apply producer invalidates affected property/declaration keys; ordinary text-only changes do not require a whole-registry rebuild. A metadata read failure is an error, not an empty registry. Public `query_registry` uses this same capture and snapshot path. Registry wire generation describes changes to effective metadata; it is not an acknowledgment of a particular edit.

The reader loads no page document and parses no source text. Selection may inspect substantial database data, including all visible text for an unselective regex. Ordered narrow descriptors charge the existing construction budget; raw output payload is fetched in batches only for admitted rows. Managed page order remains `pages.path` under SQLite BINARY collation, with stored within-page preorder and exposed identity. Existing journal-name recency and admitted result-page filesystem metadata lookup remain separate from source-text reads. Required-row decoding or coverage damage is a failed read. A predicate whose match itself depends on an absent joined row can still omit that match without the result constructor observing the missing row; this existing limitation is not repaired by removing pending composition.

The authoritative pending editor and navigation state remains in its ordinary journal and editor machinery. It is not a live-query source and has no query-only database, mask, merge, repair worker or patched pending registry. Before ordinary drain, a query may show the older current main image. Drain or external acceptance advances that image, and database-change notifications schedule another UI refresh. Query result blocks still edit immediately through the normal editor path; membership grace is UI scheduling, not a storage freshness guarantee.

Lifecycle cancellation reaches both queued captures and open read handles. Capacity is acquired before transactions; the transaction ends before the job slot is released. Closure and projection replacement cancel and drain the existing job owner before closing the projection. Ordinary accepted writes coexist with readers under WAL and do not cancel them. A stale acquisition recaptures at most twice, then reports typed readiness; busy admission reports busy readiness; cancellation remains cancellation; failed reads report unavailability. No outcome switches to traversal. The independent traversal oracle remains test-only for these migrated surfaces. Advanced source parsing, binding, limits and refused-source reports keep their existing shared owner and semantics.

Copy/export query subtrees use the same current-main snapshot boundary and one shared SQL export executor on both backends. The Managed adapter releases the actor before SQL selection and subtree hydration; it neither includes pending editor pages nor advances their save continuation. Root/node/byte limits remain cumulative over the command. The previous export walkers and actor merged-registry cache exist only as test oracles. Friendly public search and print-query rendering remain separate campaign consumers until their SQL migration is accepted.

Static whole-site publication uses one operation-owned main snapshot and the shared query resolver/compiler/readers for every authored query occurrence; it has no answer memo or captured-document query database. Direct compares the complete captured path/content-revision/config fingerprints to `direct_source_revisions` inside that read transaction, refusing a mismatch before staging output. These fingerprints are disposable derived data, not source authority. Its fresh capture uses structural result IDs without changing the live editor identity owner. Managed reuses the existing bounded normal foreground drain for this explicit publication command, then captures main SQL and the authoritative page documents with stored result IDs. This command-specific capture rule does not apply to ordinary live queries. The existing Managed whole-site renderer still occupies its actor during publication; this migration does not claim nonblocking publishing.

The whole-site renderer already owns captured documents for page output, embeds and namespaces; admitted query roots hydrate from that same capture. Visibility filtering happens after complete matching, and ambiguous canonical page identities remain refused. Page-result links require matching physical path, title and kind in the captured public capability. Cancellation is checked before the atomic output-stage commit. Publication tests exercise real main SQL, private omission counts, advanced/TQL/BEGIN_QUERY/query sheets, ordinary external source progression, capture identity isolation and output-stage recovery.

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
| SQLite and projection receipts | acceleration, reconstruction, diagnostics | semantic truth or permission to overwrite Markdown |

The native watcher may observe metadata changes below the approved assets
capability solely to invalidate WebView render caches. That observation grants
no managed actor admission, publishes no operation or provider object, and does
not make Tine responsible for transferring or resolving asset bytes; the user's
whole-directory synchronizer remains their transport. Notifications through a graph's
`assets` symlink are mapped to its currently approved canonical asset root
before per-window routing, including when several graph windows share that
root. This mapping uses all live approved bindings and works after deletion;
it neither follows an event path to a new target nor grants approval.

Authority is transferred only by a validated, durably published record while
the current owner retains the relevant lease/capability. A path name, a newer
mtime, a cache row, or provider arrival alone never transfers authority. Any
operation that observes a changed generation/descriptor/frontier must restart
from that observation instead of completing under stale authority.

### 2.2 Local lifecycle

1. **Direct / absent** — no private binding; startup opens Direct Files and
   does not inspect shared bytes.
2. **ShadowImport** — explicit activation first remains Direct/absent while it
   reads the live source once into a sealed private capture and prepares an
   inactive immutable bootstrap from that capture only. One fresh complete
   live-source scan must then match the sealed capture. Only after that proof
   does Tine publish `ShadowImport`; a mismatch leaves durable enrollment
   absent, refuses without changing Direct Files, and permits a clean retry
   from the current Markdown/Org bytes. If the pre-enrollment reservation's
   source digest differs on retry, Tine preserves and detaches that attempt's
   archive, receipts, SQLite state, runtime, backup, preparation, enrollment,
   and reservation as one reconstructible diagnostic episode before rebuilding.
   The new sealed capture and the live graph are not moved. Archive detachment
   happens first and enrollment last, so an interrupted reset retains the old
   reservation and repeats safely. Active/shared enrollment is never retired.
   Bootstrap semantic lowering is size-adaptive. Canonical encoded operations
   remain in memory through partitioning and detached authoring while their
   measured retained bytes stay at or below 128 MiB; this ordinary route writes
   no operation spool and performs no operation external-merge sort. Crossing
   that byte budget deterministically spills the same canonical records into
   the bounded external-sort path. Both routes must produce byte-identical
   aggregate and commit records. The process-only terminal SQLite optimization
   retains only authenticated accepted events after authoring, never a second
   operation-spool artifact.
3. **VerifiedLocal** — bootstrap, backup, shadow projection, and SQLite proof
   agree. Authority is still inactive.
4. **LocalActive** — promotion publishes the accepted runtime state; the actor
   acquires enrollment/archive leases and becomes the sole managed writer.
   Once the activation marker and immutable baseline have been retained, a
   later runtime-open failure must keep that authoritative set whole. The next
   explicit open resumes from it; it must not reclaim only the archive and
   strand a half-live marker.
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

1. **LocalActive → SharePrepared (initiator).** The initiator publishes every
   exact chunk of the sealed lazy-genesis baseline, publishes the small index
   that binds those chunks, publishes the accepted operation tail, and only
   then publishes the one shared descriptor and records the matching local
   phase.
2. **Direct/explicit join → Joining (joiner).** The joiner reads that exact
   descriptor. Before bootstrapping the descriptor's authority, it renames any
   app-private managed root that is not selected by the Direct Files slot into
   `sparse-v2-recovery`; this includes an interrupted activation candidate and
   a complete predecessor retained after an explicit Direct Files selection.
   The move preserves the predecessor whole and prevents its clean activation
   marker from being reopened under the descriptor's different identities.
   The joiner then reconstructs the descriptor-bound baseline and causal
   manifest/object closure in a private staging area, and replays it to the
   advertised frontier. Before replacing local managed authority it compares
   the complete disk-expressible page/outline semantics with the currently
   synchronized Markdown/Org graph. A mismatch leaves both authorities
   unchanged; equality installs the provider history without rewriting graph
   bytes. The refusal crosses the native boundary as the typed
   `shared-frontier-mismatch` payload (`docs/contracts/typed-errors.md`): its
   detail reports complete page and mismatch category counts and names at
   most 32 differing relative paths with whether each is local-only,
   shared-only, or differs in kind, preamble, outline, or externally supplied
   block IDs, plus how many further mismatches were omitted. The join panel
   shows those paths only to the user who attempted the join; the general
   shareable diagnostic line stays path-free, and nothing ever prints note
   content or UUID values. Local-only endpoint and device identities remain
   local.
3. **SharePrepared/Joining → SharedActive.** Each device records its role
   (`Initiator` or `Joiner`) in its own enrollment. The descriptor remains the
   shared identity; local endpoint/device IDs remain local.
4. **SharedActive operation.** A local edit is durably journaled, authored into
   the oplog, accepted locally, projected, then published as objects → intent →
   manifest/recovery copy → covering frontier head. Peers admit only complete,
   validated batches and apply them in causal order. A peer renders accepted
   semantics against its own exact current bytes, using the source operation's
   authenticated render-base block identities. It therefore retains harmless
   receiver-local representation such as CRLF while preserving parser-owned
   structural layout, including non-bulleted Markdown headings whose content
   changed. The source endpoint's target bytes do not become receiver write
   authority.
5. **Interrupted transfer.** Missing/temporary/reordered bytes remain pending;
   exact immutable collisions or inconsistent stable cuts block. A retry
   resumes from durable observations rather than inventing state.

On every cold installation of a `SharedActive` actor, the first production
watcher turn performs one imprecise provider scan before relying on exact
filesystem callbacks. Provider bytes may have arrived while Tine was stopped,
before an inotify watch existed; graph-local text scanning alone cannot prove
that shared transport is current. Local-only managed storage never performs
this provider adoption merely because another device's namespace is present.

A completed local journal drain emits `LocalMutation(Durable)` after the
main SQLite projection and local checkpoint are complete. The existing watcher
change event wakes live query consumers even when an earlier post-edit refresh
ran before projection completion. Pending local drain turns remain `Recovering`
and do not announce a content change. This notification does not make queries
wait for any particular saved edit.

Provider traversal and incomplete projection recovery may span several actor
turns. `Recovering` reports bounded progress only; it is not itself a content
notification. When an inbound provider batch becomes visible in SQLite and the
receiver's Markdown/Org projection, the actor emits one `ProviderMutation`
tick naming that batch. If an interleaved serialized application request
finishes a retained provider projection between watcher ticks, the actor keeps
that batch identity until the next tick emits the same notification; a read
must not consume the live-view wake-up. The production watcher treats that tick
as an observable graph change and schedules a continuation for any remaining
provider work. A terminal quiet watcher admission remains `AdmittedNoop` and
must not manufacture a frontend refresh or conflict.

**Known provider work is itself a runnable work source.** Delivered provider
evidence is work this device never performed; it arrives as bytes another device
wrote, and once the delivery is over no further filesystem event announces it.
One actor turn settles one lane, so a turn that consumed a watcher epoch reports
`Admitted*` while delivered provider evidence is still retained — that report is
not evidence of a quiet graph. The actor therefore publishes
`SyncRuntimeStatusSnapshot::provider_runnable`, the exact predicate `tick`
consults before routing into the provider lane, and the production watcher
schedules its next turn while `has_runnable_work()` (a pending watcher epoch OR
runnable provider work) is true. `provider_pending` is NOT that predicate: it is
a broad protocol inventory which also counts durable publication intents that
legitimately remain after publication, so a scheduler driven by it would never
sleep. Conversely, when `has_runnable_work()` is false the scheduler arms no
timer at all and blocks on the kernel; and a turn that reports `Idle` while
still naming runnable provider work — the ready queue blocked on causal
dependencies whose bytes have not been delivered — is paced by the ordinary
retry backoff rather than the progress cadence, so a blocked dependency cannot
become a poll loop.

**A causal dependency is never queued behind its dependent.** The direct
provider manifest lane advances only its front entry. A batch whose dependency
sits behind it in that same queue therefore deadlocks the pair: the front batch
re-inspects the dependency every turn while the dependency never reaches the
front, and — by the rule above — no later filesystem event breaks the tie, so a
peer's edit is stranded while the actor honestly reports runnable work forever.
Membership in the lane's dedupe set does not by itself mean a dependency will be
admitted in time; position is part of the contract. Whenever admission blocks on
a dependency, that dependency is moved to the front of the lane (restoring the
deque/set invariant if they disagree). Queue order otherwise follows provider
scan order, which is why this state reproduces only intermittently in journeys —
`a_dependency_queued_behind_its_dependent_is_promoted_ahead_of_it` pins the rule
deterministically.

**Outbound causal heads retain replay evidence.** Before publishing a queued
batch's objects, manifest, or frontier head, the runtime requires every immediate
causal parent to be complete in the logical hot/cold archive and to name the same
workspace and lineage. Accepted effects alone do not prove retained replay bytes.
Missing manifests or objects block publication and Safe handoff; restoration
unblocks the queued batch without reopening. This check reads the immediate heads,
not the whole accepted history. The two
`outbound_child_blocks_when_ordinary_parent_*_lost` tests cover missing manifests
and objects, refusal, and recovery.

### 2.3a Adoption: a device that already has a managed graph of its own

Both a phone and a desktop can enable Tine-managed storage on the same synced
folder without either knowing about the other. Each activation mints its own
`WorkspaceId`, `LineageDigest` and catalog `DocumentId`, so §2.3's join refuses
immediately with `clean shared descriptor names another managed graph`. That
refusal is correct: the join in §2.3 step 2 REPLACES this device's baseline and
operation archive and deletes the replaced pair, and it may only do so when the
two sides' user-visible semantics already agree. Widening the identity check
would be silent data loss.

**Adoption** is the named operation for that state, and it is a composition of
the two transitions that already exist, not a new storage operation:

1. **Set aside.** The graceful Direct Files return (§2.2) drains and stops this
   device's managed runtime and renames its whole app-private managed root to
   `<app-data>/sparse-v2-recovery/<graph-key>-<uuid>`, then publishes Direct
   Files from the unchanged Markdown/Org tree. Adoption runs exactly that,
   with one difference: it does **not** archive `<graph>/.tine-sync/v2`. That
   subtree is the OTHER device's shared evidence. Archiving it would remove the
   descriptor the second half is about to read and, under a folder-syncing
   tool, propagate that removal back to the sharing device.
2. **Join.** The Direct Files join branch bootstraps a binding out of the
   descriptor's three identities — keeping this device's own `DeviceId` and
   minting fresh endpoint/preparation/session identities — and performs §2.3
   step 2 unchanged.

Each half is a complete supervisor transition with its own stable end mode
(`ReturnGracefully` → Direct, then `JoinManaged` → Managed). A crash between
them therefore lands on Direct Files with the predecessor archived and the
shared graph still joinable. That state reports as ordinary Direct Files
(`sparse_v2_status_for_slot` deliberately does not inspect a shared descriptor
for a Direct Files slot), whose panel already carries "Join a synced graph from
another device", so the second half is retryable on its own. Within each
half the pre-existing rollback applies: a failed drain restores the managed
slot, a failed archive leaves both roots byte-identical, and a failed Direct
publication leaves the archive and the shared evidence in place with the
Markdown/Org tree serving. No seam can produce a hybrid.

**What adoption carries across, precisely.** Nothing of this device's own
managed history: not its operation lineage, not its baseline, not its block
identities. That history is archived whole and stays readable — the same
activation request rebased onto the archive still opens it. What survives on
this device is its Markdown/Org tree, which adoption never writes; and because
the second half is §2.3 step 2 unchanged, that tree must ALREADY be
semantically equal to the shared graph's. When it is not, the join refuses by
name (`… not in the shared provider frontier`), the archive remains readable,
and nothing is merged. Adoption is therefore "keep the shared graph's history,
set mine aside", never a merge of two divergent histories.

**Refusals, each with its remedy.** Adoption decides all of these BEFORE the
first durable step:

| State | Refusal |
| --- | --- |
| Incomplete provider tree | The §3.1 partial-provider refusal: sync data is still arriving; let the file-sync tool finish and retry. |
| No descriptor at all | "This graph does not yet contain sync data from another device", naming the path looked for. |
| Every identity matches | Nothing to set aside; use the ordinary join. |
| Some identities match and some do not | Tine cannot tell which history is which; nothing is changed. |
| This device is itself sharing, joining, or holding an unfinished cut | Adopting would abandon devices joined to this one; finish or return to Direct Files first. |
| This device is already in Direct Files | There is no managed history to set aside; use the ordinary join. |

Every user-facing first line carries a diagnostic-class word, because the panel
keeps only that first line and drops lines with no recognised class. A refusal
may append the bounded local-only diagnostic continuation defined in §2.3; it
is written to the detailed local trace and is not copied into the panel or the
privacy-safe flight recorder.

### 2.4 Lazy activation and clean runtime boundary

The accepted next activation generation has one authority-changing record:
**one final lazy-genesis authority marker**. The Tauri binding records opt-in
intent and permits setup/resume UI, but it is not semantic authority. Until the
final marker exists, Direct Files remains the sole authority and every baseline,
SQLite, receipt, and episode artifact is disposable.

The marker binds exactly the workspace, lineage, authority-directory generation,
immutable baseline root, sealed source-capture description, accepted-frontier
digest, and watcher fence.
SQLite identity is deliberately absent: SQLite is a frontier-stamped disposable
projection and can be rebuilt without changing the marker or semantic truth.
The marker is published only after the baseline is durable and one final
byte/inventory comparison under the watcher fence matches the sealed source.

**Harvest W3-R1 generation commit.** Fresh activation publishes
`lazy-genesis.0` and `operations.0`. A shared join writes its verified baseline
and operation archive under the next generation, synchronizes the archive
directory, and then replaces `lazy-genesis.marker`; that one marker rename is
the commit point. It never renames or hides the active generation first. Cold
open resolves both directories from the marker through one resolver and
reclaims `.clean-join-*` staging directories plus every generation the marker
does not name. Therefore a crash after publishing either directory, after the
directory barrier, or after marker replacement opens the complete old or new
pair—never a half-swapped pair.

Each page capsule carries the exact original Markdown/Org bytes once, one
deterministic CRDT checkpoint constructed directly from its terminal page
state, plus its compact causal dependencies. New page-capsule v5 records may
also carry a versioned, bounded SQLite semantic receipt binding the exact-source
digest to a canonical materialized-page digest and payload. Receipt bytes are
limited to 64 MiB and one million block rows; a larger page omits the receipt
instead of refusing activation. Existing capsule-v4 records remain readable
with no receipt. A missing, malformed, digest-mismatched, capsule-inconsistent,
or parser-version-stale receipt is recovery evidence, not a refusal: disposable
SQLite reconstruction reparses the exact source and performs the original
capsule comparison. Only divergence established by that retained parse refuses
reconstruction. A payload and digest consistently authored together are trusted
at baseline construction under the crash/corruption threat model in §3; they
are not a defense against a malicious byte-forging actor.
Ordinary foreground saves separately retain at most three exact whole-document
outline-parser results per worker thread (accepted base, requested target, and
clean render base), only for sources up to 2 MiB and never for parser failures.
Parser-owned unbulleted-heading event facts travel with that result rather than
causing one more outline parse per block. This cache is not durable authority;
an exact cache miss takes the complete path.
One canonical activation-record pass fans each parsed page into both the
baseline pack and bounded SQLite materialization chunks. Neither candidate is
published by that construction pass, and SQLite does not re-read, re-parse, or
replay the graph to derive the same terminal state a second time.
Managed inventory derives Page-versus-Journal identity from the decoded file
name and configured journal title format, matching Logseq's
`get-page-name` and `convert-page-if-journal` at OG revision
`6e7afa8eb040686ff057156ee877193b581dd369`, respectively in
`deps/graph-parser/src/logseq/graph_parser/extract.cljc` and
`deps/graph-parser/src/logseq/graph_parser/block.cljs`; the containing
configured pages/journals directory
is path ownership, not semantic-kind evidence. Both current-hot and
caller-borrowed materialization use one membership/block collector. The hot
arm clones at most one non-page home at a time (no all-home arena), preserves
the existing distinct-home statistics, and alone applies the current accepted
exact-title selection; historical and prospective state materialization does
not consult that later root.
The single catalog checkpoint is constructed by the same direct terminal-state
builder, and the sealed manifest binds its non-derivable catalog document ID.

Interactive state uses one catalog document plus one shard document per page.
The catalog holds every page's live-or-tombstone state, including its immutable
home shard. A page shard binds its page identity in `shard_meta` and holds that
page's `owners`, `members`, `content`, `logseq_uuids` and preamble maps; each
block's content is one ordinary upstream `LoroText` container inserted under
`content`. A block's
immutable home is the shard of the page it was created on, and its
`BlockBirth { page_id, page_document_id }` names exactly that page and shard.
The birth is authenticated in semantic-effect schema 9 against the shard's
immutable page identity at the batch's declared causal base, or against a page
born in the same batch. Delete removes the live owner, content, membership and
paired sparse UUID/origin keys for payload live at that causal base. An atomic
create/delete has no accepted deletion before-image: it retains the existing
birth-retirement tombstone provenance and is the sole physical-removal
exception. Restore is explicitly marked as reconstruction,
selects an accepted delete before-image (or the recorded predecessor frontier
for an activation-era sweep without a deletion id), retains the BlockId,
immutable home and Logseq identity, and inserts a new text container; moves and
replay never rewrite the immutable home. No document exists per
block or per membership. Accepted frontier roots, the run-local accepted
document map, SQLite frontier rows and point reads key a live document by its
16 UUID bytes; a key of any other length is refused, never truncated. Journal
and manifest commit points and checkpoint recovery are unchanged. This is one
current format: operation schema 11, semantic-effect schema 9 and lazy genesis
schema 7. Earlier managed stores are preserved as backups and take the existing
Direct-files rebuild path; there is no old-format decoder or migration.

A cross-page `MoveSubtree` rewrites the moved blocks' owners in their home
shards and their membership in the source and destination shards; content stays
in each block's home shard. A subtree born entirely on its source page therefore
changes exactly two CRDT documents, while a mixed-home subtree also touches the
other home shards. A same-page `MoveSubtree` is only the existing root
`ReorderBlock` mutation after the exact root-home check: it changes that root's
membership and never rewrites descendant memberships or owners, so an
independent child reorder remains independent. A move may target a page deleted
since the caller observed it; the moved owners then name the tombstoned page,
and exact revival removes them again.

Whole-file external deletion is represented by `DeletePage` alone. It
tombstones the page's catalog identity, removes each still-owned block's live
owner/content and paired sparse identity keys, removes the page's memberships,
and clears its live preamble. `DeleteSubtree` performs the same physical removal
for the selected owned subtree. The accepted semantic effect records complete
block, membership and preamble before-images with absent post-state, so Restore
does not depend on retained CRDT payload. A block already moved to another live
or newly created page is not removed when the source page is deleted and keeps
its immutable identity and home. An atomic same-batch birth/retirement retains
its tombstone payload and birth authority because no earlier accepted state can
supply a before-image; a later Restore validates that authority and authors a
new container while taking its placement from the Restore operation. Deferred pages keep
their existing behavior. A path rename preserves its matched `PageId` and is
therefore not a whole-file page deletion.

Projection-manifest completeness is computed per affected page from the exact
native before/after states. A page absent at both states needs no file intent;
every page live before or after still requires its complete manifested
projection directions. This is page-local even in a mixed batch: an unchanged
tombstoned page may be projectionless while another affected live page carries
its ordinary replacement. The author carries the catalog, whose state proves an unchanged tombstoned
page, through the existing read-only dependency frontier; missing exact proof
never excuses a missing intent, and receiver-current SQLite/page state is
not authority for the decision.

`AcceptedBatchEvent` resolves affected pages against its authenticated accepted
post-root. `EffectValidationContext.merged_deletions` carries that existing
accepted-rendering absence proof for both contested and uncontested pages;
contested live pages retain their complete recomputed rendering in
`merged_replacements`. SQLite therefore permits an absent page's replacement to
be omitted only with accepted-root deletion proof, while every visible page is
still checked byte-exactly against the authored effect (or recomputed-and-
compared when merge superseded it). The supplied materialization must still
cover every affected page exactly once as replacement or derived deletion.
Cold replay preserves retained blocks' original creation-page homes and content
text identities and leaves an absent page hidden until an explicit
`RevivePage` restores its predecessor state.

Every `PreparedBatch` constructor proves that the existing canonical manifest
encoder accepts the manifest before returning the publishable value. Oversize
uses the existing typed `ManifestTooLarge` error and unchanged 1 MiB limit; it
does not cache another encoding or define a second serializer. Whole-batch
pending/error handling remains the existing route. This contract does not add
chunking or promise support for an arbitrary oversized atomic batch.

These checkpoints are baseline semantic/causal state, not fabricated interactive
history: their construction authors no `SemanticOperation`, batch, ordinary
mutation receipt, partition, or detached bootstrap part. Untouched page
checkpoints remain unopened in the lazy pack until a page read or first ordinary
operation needs one.

The sealed manifest's page-home index resolves a page shard's baseline
checkpoint without scanning unrelated page descriptors. Accepted dependency
heads and the current overlay remain authoritative, so an edited cold document
never falls back to its initial baseline checkpoint, and a moved block's content
remains in its creation page's shard rather than following current ownership.

Reading one baseline page costs that page, not the pack. A sealed segment pack
is written once and never rewritten, so its whole-pack digest is proved against
the sealed manifest at most once per opened baseline — re-proving it per page
would make every consumer that walks the graph, including the clean watcher's
full scan, cost `O(pages x segment bytes)`. The proof is discarded and repeated
if the pack is relocated. Each page still verifies its own capsule bytes against
the sealed descriptor digest on every read, so damage to the bytes a caller
actually receives is rejected regardless of that retained whole-pack proof
(`lazy_genesis::tests::lazy_genesis_proves_each_sealed_segment_at_most_once`
and its damage siblings).

The corresponding crash states are exhaustive:

| Durable state at restart | Authority | Required behavior |
| --- | --- | --- |
| No final marker; no episode | Direct Files | Start activation from the current graph. |
| No final marker; partial baseline or SQLite | Direct Files | Ignore/quarantine the episode and rebuild from the current graph. |
| Final comparison differs; no marker | Direct Files | Preserve bounded diagnostics and restart from a fresh source observation. |
| Marker publication began but no complete canonical marker exists | Direct Files | Treat every candidate artifact as uncommitted. |
| Valid marker and complete baseline | Managed baseline plus later accepted operations | Open the lazy engine; open matching SQLite or rebuild it. |
| Marker exists but baseline validation fails | No silent writer | Refuse managed admission and offer recovery or Return to Direct Files. |
| First materialization has no durable ordinary operation | Baseline page capsule | Discard the partial materialization and retry deterministically. |
| First ordinary operation is durable | Baseline plus that operation | The ordinary document state supersedes the page capsule. |

Managed mutation ordering is likewise fixed before native identity-index
removal: validate SQLite at accepted frontier `F`, prepare the semantic
operation and exact row delta, durably append the operation as `F+1`, then
commit SQLite at `F+1`. A crash after the durable append leaves a stale
projection which is replayed/rebuilt. A SQLite failure after the append does
not turn the accepted edit into a retryable save or permit a duplicate write.
SQLite must never publish `F+1` before semantic history does.

Application protocols which need retry-stable identity, currently cross-page
subtree moves, supply one deterministic `BatchId` to the same clean commit
pipeline. Their immutable episode record and manifest fingerprint are
published before the operation manifest. A failed episode publication cannot
reach the manifest; after the manifest exists, cold replay plus that record
turns a repeated request into one recovered result rather than a second edit.

Cold tail replay is a causal fixed point, never manifest-directory or random
`BatchId` order. A batch becomes runnable only after this run has reproduced
every prerequisite named by the union of its compact causal heads, operation
dependency frontier, and each manifested projection post-frontier (excluding
the batch's own post-state head). This union is load-bearing: an operation can
touch only one semantic region while its projection post-state includes an
otherwise unrelated page creation, and projection validation reconstructs that
larger frontier. A merely durable pre-shutdown status or an effect-equivalent
accepted prefix cannot make an unreplayed manifest ready.

Clean open first attempts the disposable `clean-open-checkpoint-v2` state. Its
single `current` pointer is the commit point; payload and generation bytes land
completely in the inactive one of two bounded slots before that pointer changes.
An interrupted write therefore leaves the prior pointed generation complete.
Every create and replacement uses the audited durable directory-publication
boundary. There is exactly one current format, no migration reader, and no
preserved backup for rejected checkpoint bytes because the operation archive is
the sole semantic authority. A checkpoint whose roster names an accepted
manifest or required object that the archive no longer holds is not a
checkpoint defect: the checkpoint is discarded and the missing archive
evidence surfaces as `MS-REF-DISK-CORRUPT` (scenario: disk error or torn
sync-service delivery of the archive), the same scenario the full-replay path
would report.

The payload binds workspace, lineage and catalog document; its recovery fence
binds accepted sequence, semantic-state digest, live eligibility `E`, and the
age/size policy revision, 30-day lower bound, minimum-tail bytes and live-size
multiplier. Each document-to-image record binds exact accepted dependencies,
its image descriptor, `E`, the same policy config, requested `K`, actual removed
sequence, actual native floor, and the measured image/latest/removable/budget,
post-cut, hysteresis, overage and limiting-cause facts. Live eligibility comes
from canonical device-local acceptance observations persisted in the same
checkpoint. `T` is the latest first-local-acceptance observation at `S`; the
cutoff is `T - 30 days`; `E` is the greatest whole accepted prefix whose
observations are no newer than that cutoff. Duplicate delivery, replay,
checkpoint export and checkpoint installation do not advance `T`. A recovered
unknown suffix receives a conservative current upper bound without becoming an
acceptance event. Clock rollback/discontinuity freezes advancement; a later
stable wall/monotonic pair starts a fresh conservative epoch and records its
reset time. Elapsed wall time without a new acceptance advances nothing.

Before publishing `current`, the worker rereads the complete inactive payload
and generation, checks their exact digests and slot/sequence bindings, validates
the workspace/lineage/catalog and recovery fence against the decoded semantic
state, and validates roster completeness plus every descriptor/dependency/floor
binding. A reused immutable image is already pinned and unchanged, so this
qualification validates its manifest binding without importing that document.
The actor re-proves its continuously held lease and capture binding at
publication and again before installation. Exact name replacement supplies the
crash commit point; it is **not** cross-process exclusion.

An absent, torn, wrong-format or internally inconsistent pointer, generation,
payload, floor record, roster record or document image always discards the
derivation and takes sequence-zero replay from immutable genesis plus all exact
accepted originals and selected journal prefixes. The replay seeds a fresh
base-zero checkpoint instead of trying to extend the unreadable predecessor.
No repair path deletes accepted objects, manifests, managed-local records,
projection-turn records or recovery-input originals. If replay itself proves an
authoritative archive object missing or damaged, it reports
`MS-REF-DISK-CORRUPT`; a disposable checkpoint cannot mask that damage.

The restart contract at every publication edge is:

| Crash/publication edge | Required restart behavior |
| --- | --- |
| Before recovery-input append is durable | The provider item remains unconsumed and receives no custody acknowledgement. |
| During append, or after durability before acknowledgement | The WAL distinguishes an uncommitted suffix from damaged committed data and replays one complete frame once. |
| During reconstruction or accepted-object publication, before manifest commit | The old checkpoint remains selected, journals retain originals, and journal-covered object repair applies. |
| After accepted manifest commit, before checkpoint capture | Full replay discovers the accepted original; a retained duplicate is harmless. |
| During inactive payload write, sync or replacement | The prior pointed generation remains usable; partial inactive state has no authority. |
| After payload, during generation publication | The prior pointed generation remains selected; no candidate is selected before `current`. |
| During pointer replacement or its durability barrier | Reopen selects a complete old or new generation; absent, torn or invalid `current` takes genesis plus originals and journals. |
| After durable pointer, before actor swap or SQLite rebuild | Reopen selects the new checkpoint, replays retained tails and rebuilds disposable projections. |
| After actor swap, before journal/input cleanup | Reopen replays and deduplicates retained work; it never authors the operation twice. |
| During cleanup | Durable selector/drain-anchor ordering preserves either the retained frame or proven exact accepted coverage. |

The checkpoint names every clean-runtime authority needed to reproduce later
admission, conflict, or query decisions: accepted/frontier semantic roots,
resident-document identities, the bounded current-action hot-retention closure,
and the accepted sequence. Complete identity evidence for page names, portable
paths, block homes and Logseq UUIDs lives in sealed point roots; companion
current roots carry only current claims, current-path rows and their covered
projection-head facts. Reopen enumerates those current roots once and then
composes them with the accepted recent overlay. Released evidence remains
point-addressable without becoming resident actor state.
The accepted roster is not a parallel list: it is
the canonical `tine-storage` sealed accepted index with exact accepted evidence,
causal records, status map and sequence root. The checkpoint also records each
document's latest-change point root and a point-addressable covered-object root;
it does not embed an H-sized manifest roster or required-object union in the
payload. Run-local capabilities, cursor nonces, timing counters, LRU-only
caches, attached graph/receipt handles, and an unaccepted foreground journal
overlay are excluded; they are newly minted, rebuilt, or replayed by their own
authority before use.

On open, the marker-selected generation and its terminal accepted proof are
qualified before archive content validation. Namespace validation parses every
hot name but does not read, digest or decode a name authenticated as covered;
it fully validates only live tail and explicitly pinned hot manifests. Covered
history is resolved through its sealed point indexes and exact cold originals.
An absent or invalid generation takes the unchanged sequence-zero full audit.
An interrupted post-marker retirement is discovered from covered hot names and
resumes idempotently: cold manifest and object bytes are point-verified before
any repeated unlink. On the healthy generation path the open-work counter
requires zero covered-sequence enumeration. Covered namespace manifest and
object decodes are held at zero by a stronger channel than a counter: the
`is_covered` early `continue` in the namespace walk, which makes a covered
object decode unwritable, plus assertions on the real
`namespace_{manifest,object}_decodes` instrumentation, which does grow when a
covered name is read. There is deliberately no covered-decode or
covered-roster-rows counter: both were previously reported as zero by code that
never incremented them, which reads as proof and is not.

After checkpoint restore, only archive manifest names outside its roster enter
the same dependency-staged fixed point described above. A failure to admit that
tail discards the restored state and retries from sequence zero. The SQLite
genesis choice reads the engine's accepted-frontier predicate directly; an
empty tail is not evidence of genesis. Open counters distinguish checkpoint
and full-replay paths and report roster/name work, checkpoint capture work and
payload bytes, the actual tail replayed, and durable lag.

Snapshot capture is coherent on the owning actor and is attempted after every
accepted managed save. The actor captures canonical semantic metadata plus only
the accepted rows and required-object additions after the publisher's durable
frontier C. The worker path-copies those additions into the predecessor's
sealed accepted, covered-object and latest-document-change roots; a later
generation visits exactly C+1..=new C, never 1..=E. For a document whose accepted dependency binding differs from the
last published checkpoint, a resident document crosses the thread boundary as
an upstream Snapshot clone; an evicted document carries only its exact
dependencies and is reconstructed by the worker from its prior image plus the
accepted suffix. Bootstrap falls back to immutable genesis plus accepted
archive ancestry when no image exists. Unchanged documents carry no image
bytes.

The worker imports or reconstructs only changed documents, runs the shallow
export and fresh-document verification policy, and publishes the resulting
content-addressed image and descriptor before the payload, generation, and
`current` pointer. The complete sealed roster reuses the exact prior descriptor
for every unchanged document. Reopen imports only the documents that were
resident at capture; a later cache miss or same-session eviction loads the
current published image and applies only accepted document updates after that
image's S. It falls back to genesis/original replay only when the roster has no
image for that document. For every changed document the worker measures the
current image and a latest-state-only image at the same `S`. Removable bytes
`R = max(0, I-L)` trigger a cut only when they exceed
`B = max(256 KiB, 4*L)`. The worker obtains at most one floor candidate per
changed document: the latest qualifying change in the current delta, otherwise
one predecessor query against the sealed latest-document-change root, otherwise
lazy genesis. It verifies the native candidate image without assuming the
requested floor is installed exactly. If that candidate does not reach the
hysteresis target, the shortfall/overage cause is diagnostic. Age wins over
size: recent or uncertain history is never removed to meet a budget, and a
within-budget document never cuts even after months of device time.

After cold publication has preserved exact bytes, `current` is replaced and
only then may covered hot names be retired. The retained hot closure is bounded
by current document/projection heads, causal peers, and current-action roots
(unfinished receipt work, sweep predecessors and pending Restore). Newly
completed action pins are retired by a later generation. Crash prefixes leave
the old generation, the new generation with both copies, or the new generation
with cold-only covered history; reopen accepts all three and repeats only safe
post-marker retirement.

Provider admission nevertheless implements the below-floor boundary before
that policy becomes active. Preflight checks every declared affected/support
document dependency and the CRDT update's own causal requirements against the
document's actual native shallow floor. A missing causal parent remains
ordinary pending delivery. An input proven older than a floor returns typed
`NeedsFullHistory { batch_id, document_id, dependency, floor }`; validation
uses disposable Snapshot clones, so neither side of a partially validated
cross-page operation changes live state.

The exact original then enters the current recovery-input envelope schema 1 in
a graph/receiving-endpoint-scoped `LocalJournalSegmentV2` under private
app-data. The envelope binds workspace, lineage, receiving endpoint/device,
source endpoint and BatchId to the canonical manifest bytes and every required
canonical object byte in manifest order. It records transport custody, not
acceptance, and is never published into the accepted archive namespace.
Durable append completes before provider work is dequeued or custody is
acknowledged. An uncertain append reopens the selected WAL and resolves the
exact BatchId and bytes before any retry; identical redelivery is idempotent,
while conflicting bytes under that identity follow input-validation collision
handling. I/O or resource failure leaves the provider item queued and all
authoritative inputs retryable. The recovery-input segment itself is the restart trigger;
there is no separate durable recovering flag.

Before reconstructing, the actor computes a fixed point over the current
accepted frontier **union all retained recovery-input envelopes** using the same
admission-order routine reconstruction uses. If a genuine prerequisite is
absent, idle ticks keep the input pending without rebuilding genesis or walking
the lifetime manifest set. Provider delivery is still polled; arrival of the
missing parent immediately admits it and any dependent retained-envelope
cascade, then triggers reconstruction. Thus stuck work is bounded by delivery,
not by the 50 ms actor tick, while every retained dependency chain keeps an
automatic exit.

Automatic full-history reconstruction follows eight ordered boundaries. The
serialized actor first finishes or classifies any already-started append,
revokes ordinary application admission, and takes the accepted-archive
boundary plus each selected local-journal generation and durable prefix:
managed-local, projection-turn, and recovery-input. Journal cleanup remains
frozen until successful reinstall. It retains the existing workspace and
writer-lane leases, joins the old checkpoint publisher, and only then creates
a replacement publisher. A fresh engine starts from immutable
genesis, deliberately bypasses `current`, and replays the deduplicated union of
hot committed manifest names and authenticated cold manifest-map membership in
causal dependency order. Every byte is obtained through the single logical hot-then-indexed-cold resolver.

The fenced managed-local records are reapplied through the existing
`replay_managed_local_record` transition. Recovery-input originals are admitted
without regenerating an equivalent edit: their original BatchId, canonical
manifest and object bytes, causal dot, author device/session, and writer
identity remain unchanged. Existing semantic conflict rules decide visibility;
that does not permit discarding the operation. Missing prerequisites or I/O
leave the named inputs pending and the actor retries automatically; neither
Markdown nor a disposable cache may replace missing authoritative history.

After commit-last archive publication, SQLite and Markdown settlement, the
runtime publishes a from-genesis checkpoint at the recovered frontier, joins
its publisher, opens that same checkpoint through logical hot/cold
qualification, and rebinds the disposable projection under the continuously
held workspace lease. Only successful reinstall selects an empty
recovery-input successor and permits best-effort cleanup of the old segment.
Ordinary writable admission is then restored automatically. No shallow
checkpoint advances a journal drain checkpoint or retires recovery input by
itself.

The background publisher folds accepted-row deltas into the same
`clean-open-checkpoint-v2` format; no second reader or authority is introduced.
Changed/exported/reused, handoff-import/reconstruction, and worker export/import
counts are persisted as diagnostics. Cleanup retains both replaceable payload
roots (so pointer rollback stays complete), pins the exact object set of every
in-process reader until its last handle drops, and reclaims only unreferenced
content-addressed image objects. Each pass examines at most twice the retained
graph object count plus 1,024 directory entries, so repeated crash orphans drain
without turning one publication into a lifetime-sized walk. A cleanup
interruption can leave an orphan but cannot remove an image named by either
retained generation or an active reader. At the accepted
2026-09-02 gate, capture work at N=800 divided
by N=50 must be at most `A5_ACCEPTED_CAPTURE_RATIO = 1.25`. Each
capture hands immutable canonical bytes to at most one background writer; one
newest snapshot replaces any queued snapshot. This coalescing bounds memory,
not freshness. Publication failure is logged and the next trigger retries
without affecting correctness. Durable lag has no hard bound and never applies
foreground backpressure: a crash replays exactly the unpublished tail. Lag
above 64 marks the next coalesced rewrite elevated and
immediate, still off the waited path. Archive rebaselining, co-designed with
0.7 sync, is the committed terminal bound that will reduce the roster,
namespace scans, and checkpoint state to graph-proportional plus recent tail;
this checkpoint does not compact or delete authoritative history.

An unrelated accepted batch may advance a page's causal frontier without
changing its rendered bytes. A concurrent merge can also change those bytes
without carrying a new projection row for the page. A clean projection head is
therefore superseded when either the head batch was admitted against a
concurrent prefix or a later accepted batch performed a concurrent merge. In
that case the immutable row remains a locator and historical rendering proof,
while the projection planner recomputes current bytes from current accepted
semantics. With a wholly linear head and tail, any byte/frontier/layout mismatch
remains a refusal.

If conflict-resolution authoring finds the exact graph file still equal to the
superseded head's immutable target, the actor may perform one guarded point
projection from those exact bytes to the recomputed current rendering and then
redraft the resolution. Bytes that do not exactly equal that authenticated old
target are never repaired by this route; they remain external reconciliation or
refusal. Once any manifest head exists for a path it also supersedes lazy
genesis as predecessor authority, so an exact-byte mismatch cannot fall through
to a baseline capsule (and a post-activation page can never be looked up there).
The clean runtime has no persistent projection-head or completed-*path* index.
It does retain the intent-keyed own-endpoint completion index in §3.2b; a
receiver-local completion that belongs to a superseded source batch remains
durable historical receipt evidence and does not update that local half merely
to replay as a later merged point authority.

File synchronizers deliver the visible Markdown/Org projection and the hidden
provider history independently. Before classifying concurrent semantic edits,
the runtime drains every provider operation already visible in the current
provider observation; an intermediate operation must not be resolved while its
causal descendant is waiting in the same delivered cut. If a projection-first
external admission and provider history reach the same authored block text,
they are one semantic edit and collapse to one block. The same applies when a
later operation on one branch reaches the other branch's exact authored text.
Only genuinely different final authored texts use keep-both siblings. Such a
sibling has deterministic conflict-pair identity so later convergence can
retire it, but retirement is permitted only while its text is unchanged and it
has no children; any user-touched sibling remains user data.

Conflict evaluation uses one disposable, run-local index stamped by accepted
sequence. Each accepted semantic delta adds its touched blocks and concurrent
same-block pairs. Pure projection creates also meet through the exact page,
path, and target-byte digest because their sparse block identities may differ;
the existing forest-shape proof remains authoritative. A later touch that
causally descends from both members removes the settled pair. Evaluation visits
only the currently unresolved pairs and their causal branch members, never the
full retained history. When a descendant settles a pair, the index carries
only that pair's transitive members as sparse branch ancestry; a later
concurrent descendant can therefore retire the deterministic sibling authored
for the earlier pair without rediscovering unrelated history. A cold or
checkpoint-backed open rebuilds the index from immutable accepted batches
before conflict work is reseeded, and any stamp mismatch does the same before
the next acceptance or evaluation. The index is neither resolution evidence
nor persisted authority: dropping it cannot change an answer, and no history
cap or fixed lookback participates in classification.

Concurrent new blocks may legitimately choose the same sibling-order key.
Projection orders that temporary merged state by `(order key, block identity)`;
equal order keys are not corruption and must not block provider recovery. A
new-block projection echo collapses only when one concurrent batch is an
external reconciliation, the other is an ordinary local mutation, both carry
the same complete projected bytes for the same page, and their newly created
unstamped forests have the same structure and content. The unchanged external
forest is then retired by an ordinary durable semantic operation. Different
projected bytes, explicit Logseq identities, or a changed external subtree are
preserved for ordinary conflict handling.

**An applied provider batch always owes a Markdown projection.** The same rule
holds on the receiving side, and there it is a durability rule rather than a
planning optimization. A receiver decides whether an inbound foreign projection
intent is still the live authority for its page; equality of the whole merged
frontier is NOT that test. An ordinary local external admission landing in the
same window commits its own batch, which advances the shared page-catalog
document, so a delivered intent's post-frontier stops matching a page it never
touched. Treating that as supersession dropped the intent while the batch stayed
applied in SQLite: the Markdown file was never written, `clean_shutdown` still
reported Safe, and a reopen with a full re-drive never repaired it — durable
data-visibility loss, because Markdown is the interchange truth for Direct Files
parity and for every external tool. A receiver that cannot prove the delivered
intent is still current therefore projects the page's CURRENT accepted state
instead of skipping it. That is idempotent: a page genuinely superseded by newer
accepted work renders to the bytes already on disk. A receiver that can
authorize neither the delivered intent nor the current accepted state retains
the obligation as a published continuation rather than reporting completion, so
Safe is never published over a missing projection.

Completion publication carries the `ProjectionPlan` that authorized the graph
mutation through the completion boundary. Ordinary execution compares that
plan's workspace, page, path, frontier, and claim evidence with the current
accepted page before exposing point-addressable authority. Recovery first
reconstructs one plan from the durable intent and exact base, requires the full
reconstructed intent to equal the durable intent, uses that same plan for the
recovery mutation, and then performs the same current-authority comparison.
Completion recording never invokes the planner again. This preserves the
stale-frontier and reused-path refusal while making the plan that actually
authorized the completed bytes—not a later re-derivation—the compared value.

A receiver-local projection can legitimately differ byte-for-byte from the
source target while expressing the same accepted semantic page. On a later
local edit, the current accepted manifest head remains the semantic authority,
but not an assertion that the receiver copied its target bytes. Capture must
reprove the receiver's live Markdown/Org as an exact source for that accepted
page and bind the resulting annotations and bytes to the current manifest
head. A semantic mismatch enters external reconciliation; it may not be hidden
by canonical rendering or bypassed by trusting live bytes alone. This proof is
reconstructed from the manifest plus live file on reopen and therefore does
not require another persistent page/layout index.

The same endpoint-local rule applies before a page has its first manifest head.
The immutable activation capsule remains semantic authority, but it is not an
assertion that an external editor has preserved the capsule's byte spelling.
Local-save capture may use live bytes as the lazy-genesis predecessor only
after the exact-source parser proves that they express the capsule's accepted
page state. The resulting annotations and bytes are scoped to that capture;
they do not author a formatting batch, mutate shared history, or require a
persistent formatting overlay. A semantic mismatch instead enters external
reconciliation (or refuses stale local authoring), and publication still guards
the exact live predecessor, so a second external write cannot be overwritten.

For a valid clean marker, the immutable baseline plus committed ordinary
manifests is the complete semantic authority. The runtime reconstructs any
current projection-head map in process memory from those manifests; it neither
opens nor updates a persistent projection-work index. SQLite owns current exact
path identity and current canonical page-name identity. External reconciliation
reads both affected path owners and name-acquisition candidates from the
frontier-matched SQLite projection, reproves the corresponding baseline or
latest-manifest bytes against the engine, and then uses the same structural
page/block matcher as the established importer. It must not ask a native
Patricia path or page-name index to duplicate SQLite ownership. A content or
path-only edit of an existing physical same-name page does not reacquire its
logical name; only a creation or exact-title change enters name-acquisition
preflight.

An incoming projection intent's portable-path root describes the author's whole
index at authoring time. It is retained as part of the original intent and its
derived work identity; it is not proof of the receiver's whole index. Receiver
admission validates the exact path/key binding, agreement among the batch's
intents, semantic projection transition, and occupied/released per-key causal
records. It does not compare global roots or use whole-device causal clocks as
a proxy for that comparison: unrelated accepted or journaled work may differ.
The receiver derives its own path index through the existing per-key transition.
Own journal replay still requires each record's root to match its local prefix
transition, and reuse of an own projection candidate keeps its same-context check.
No incoming root changes path ownership, release ancestry, or conflict ranking.

One canonical page name has one owner, and a graph may legitimately hold more
than one physical file for it. Activation already resolves that: it selects one
authoritative source per canonical page name and per portable path in exact-path
order and retains every other file untouched, with no page of its own
(`bootstrap_authoritative_source_paths`). External reconciliation makes the SAME
selection. A source that carries no accepted page identity and whose decoded name
is already owned — by an established page, or by an earlier exact path in the
same transaction — acquires no identity: no page is created for it, no operation
touches it, and its exact bytes are still observed by the transaction. A clean
requested set likewise selects the first exact path per portable identity rather
than refusing the set. Refusing instead turned an ordinary duplicate into a
permanent graph-wide denial: planning failed for every affected path, on every
tick, with no user action that could clear it. An accepted page is never
withdrawn this way — it keeps the identity it already has, and a real title
change into a name another page owns remains the visible ambiguity preflight
refuses.

An exact watcher callback queues only the named managed paths. An imprecise
callback, and every cold open of a clean marker, queues one full comparison of
current Markdown/Org paths, SQLite paths, and released paths named by accepted
manifests. Equal bytes acknowledge the watcher epoch without an operation;
changed, created, deleted, and jointly observed renamed paths become one
external-reconciliation operation. A manifest-committed operation whose SQLite
or Markdown derivative is interrupted retains one affine continuation. The
watcher epoch remains unacknowledged until that continuation and any
observations queued behind it are reconciled, and clean shutdown drains this
work before reporting `StoppedSafe`.

Application-page hydration has selector parity while bounded external
reconciliation is pending. Exact-path and logical-name application reads may
combine the exact current Markdown/Org source with the projected page only when
path, outline topology, and block identities still match. This source-rebased
value is a disposable read view, never accepted history. This packet applies it
only to logical page reads; page-id-resolved mutation baselines such as page
deletion continue to read accepted authority. The watcher remains the sole
author of the external operation. A structural or identity-changing outside
edit therefore waits for ordinary reconciliation rather than being guessed,
while a same-topology content edit cannot make one selector fresh and another
refuse at `hot_source_join`.

SQLite schema 21 provides the physical replacement for all four native
identity-index families. Page-name and portable-path rows contain one complete,
application-owned causal point record; exact names and paths are inline and do
not depend on a content-addressed side blob. External Logseq UUID introductions
and block-home claims are append-only bounded histories which preserve every
claimant. Every causal origin is explicitly either `Baseline` or an accepted
`(batch, dot)`; activation never fabricates a bootstrap batch merely to seed an
index. Schema 21 additionally makes the disposable FTS readiness protocol
explicit; it does not change the identity-record semantics. The old Patricia
values remain only as a differential oracle until the
single production cutover, and are then deleted rather than retained as a
second ready route.

A receiver-local projection intent authored by another endpoint whose target is
`Absent` releases one exact path on this device. On the clean runtime its
authority is: the batch carrying the intent is archive-ready and is exactly the
batch this runtime accepted (accepted-batch evidence, manifest fingerprint
matched against the archived bytes), the batch carries that intent exactly once,
any declared render base is the authenticated annotated base bound to this
workspace/page/path, every declared frontier head is accepted and durable here,
and — the release itself — no live page owns the exact path in the
frontier-matched SQLite projection. Path ownership, not the page's catalog
lifecycle, is what authorizes a removal; a rename releases its old path while
its page stays live. This is deliberately the same question the own-endpoint
clean deletion asks, and it replaces the pre-0.7 proof built from the durable
endpoint-history record and the portable-path release record, neither of which
the clean runtime persists.

That authorization is total: it either authorizes the removal, proves the
release superseded because a live page now owns the path (complete without
touching that file — the owner projects it), or defers with a named reason and
retains the published continuation. Only malformed delivered content is an
error. A deferred receiver-local deletion keeps its batch `DurablePending`, and
clean shutdown refuses `Safe` naming the batch, the phase, the operation and the
path.

The clean engine does not hydrate those baseline UUID introductions into a
resident identity map. During ordinary operation the exact-frontier SQLite
projection supplies bounded baseline candidates for planning, authoring,
commit validation, and every manifested projection drain; the engine unions
them with post-baseline introductions from committed manifests, and current
CRDT block state decides whether a candidate is live and unique. This includes
replaying a retained projection after an interrupted manifest-committed
UUID-bearing edit or move: derivative Markdown authorization asks the current
SQLite projection for the baseline claimant rather than treating the
index-free hot suffix as the whole claim history. If disposable SQLite is
missing or corrupt, terminal reconstruction derives one rebuild-scoped
candidate snapshot from the immutable lazy-genesis capsules, including every
ambiguous claimant, and drops it when SQLite publication finishes. That
snapshot is a construction input, not a runtime index or semantic authority;
ambiguous baseline claims remain unresolved after reconstruction.

## 3. Invariants and versioning

1. The threat is crash, power loss, torn write, and interrupted/reordered file
   sync—not a malicious byte-forging actor. Content digests detect accidental
   damage and name immutable content; they are not a security authenticator.
   The sole `hmac::verify` call remains only for frozen legacy enrollment
   history compatibility.
2. The immutable oplog is the source of truth for managed page/journal content,
   IDs, names/paths, references, and properties. Markdown is a projection when
   managed mode is active. Assets, PDF sidecars, `config.edn`, and app settings
   retain their separate authorities. Merely opening a PDF reads its asset-side
   state and does not create an empty semantic `hls__` page; the first
   annotation write creates or updates that page through the paired sidecar and
   managed-page publication path.
3. SQLite and transient projection receipts are disposable.
   Deleting or version-mismatching one may cause exactly one bounded rebuild,
   never a second rebuild on the following open. A complete rebuild must be
   linear in graph size and finish within 10 seconds on the release corpus.
   Reconciliation databases, Patricia lookup indexes, and persistent
   projection-work indexes are retired formats: production neither opens nor
   rebuilds them.
   An unsafe retained-runtime reopen after 800 accepted page edits on the
   release corpus must finish within the manual gate's 5-second ceiling.
   Recovery retains one coarse user-visible operation and its 10-second waiting
   heartbeat, while native diagnostics emit ordered, content-free completion
   boundaries for baseline authentication, receipt precheck, graph and endpoint
   open, object-store validation, committed-tail replay, projection open,
   indexes/sweeps, journal open, own-endpoint retirement scan,
   absence-decision-map open, journal drain, terminal projection repair, and
   completion flush, followed by one content-free work-counter record
   (`SyncRuntimeCleanOpenCounters`) attributing that open's counted work —
   batches replayed, receipt evidence names and content reads, full-catalog
   passes, summary and local-completion chain reads, and archive
   inspections. Every counter is produced after the work it describes; no open
   decision reads one. These observations confer no authority.
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
   private binding or an explicit activation/join command. Its separate
   app-private graph-fact projection contains no managed state and grants no
   authority.
9. Graph-text writes take two locks, and always in this order: the
   **graph-text identity-mutation gate** (`ManagedTextWriteGate::lock_identity_mutation`,
   graph-global, exclusive across threads, re-entrant per thread) first, then the
   **per-page lock** (`Graph::page_lock`, per path). A writer that holds a page
   lock and then reaches the gate deadlocks against every writer that takes them
   the other way round — and because the gate is graph-global and its holder is
   blocked, the whole process stops publishing graph text, not just that page.
   The order is a static property of the code and is proved statically, by
   `graph_text_writers_take_the_identity_gate_before_any_page_lock`, which walks
   `model.rs`'s call graph and fails on any function holding a page lock that can
   transitively acquire the gate. `debug_assert` cannot enforce it: the shipped
   release profile compiles those out, so before 2026-09-01 the release binary
   reached the deadlock where debug builds reached an assertion.
   Reading the resource epoch (`identity_mutation_epoch_under_authority`) requires
   the calling thread to hold the gate; it now refuses with
   `graph_text_admission_unavailable` rather than reading a value another thread
   is free to advance. That is an internal precondition, not a threat-model
   refusal, and no in-scope scenario reaches it once the static order holds.
10. The hot engine's four run-local identity indexes (page names, portable
   paths, block claims, Logseq claims) have NO fixed capacity and never refuse
   for occupancy. They grow with lifetime-DISTINCT identities (a rename
   retains the released old key; deletion frees nothing) and are rebuilt from
   accepted history at every open; the stated bound on that growth is archive
   rebaselining (SPEC-A A5 decision record). The removed 4,096-entry caps
   named no in-scope scenario and were a permanent wedge across reopen — the
   block-claim member refused only at acceptance, after the drain had
   published the manifest, turning a reported save into a permanently
   unopenable store. Guarded by
   `a4_run_local_identity_indexes_have_no_fixed_capacity` and the `a4_*`
   past-capacity tests (`hot_engine_integration_tests.rs`).
11. A `None -> Some` block transition is exactly one of two authenticated
    semantic cases: a fresh `BlockBirth`, or a marked reconstruction naming an
    accepted deletion source. Reconstruction retains BlockId, immutable home,
    UUID and UUID-origin provenance. The author creates and carries the new
    Loro text container in the ordinary document update; receivers validate
    the source, update and final semantic effect together before publication.
    If editor undo text differs from the selected delete before-image, the
    ordered transaction reconstructs first and performs an ordinary edit on
    the new container second. Full accepted replay and shallow reopen must
    produce the same semantic state and Logseq-readable projection.

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
| `APP-REF-PLUGIN-IMMUTABLE-COLLISION` | Two honest concurrent installs, or a crash-recovered retry racing a completed install, present different bytes for the same plugin id and version | Keep the no-clobber winner byte-exact and refuse the other install as `immutable plugin version ... different bytes`; never overwrite or merge the package |

Reconstruction applies those scenarios at these concrete call sites:

| Condition | Scenario ID | In-scope failure and required response |
| --- | --- | --- |
| Selected batch/object is still arriving | `MS-REF-CRASH-TRUNCATED` | Interrupted sync delivery has not supplied the manifest-bound semantic effect. Keep Restore retryable, preserve the selector and source bytes, and never synthesize empty text. Surfaces as the retryable application conflict `RestoreSourceStillArriving`, never as unknown-block. |
| Accepted source is missing or damaged in both archive tiers | `MS-REF-DISK-CORRUPT` | Disk/media failure removed or changed the selected immutable object. Use existing archive repair where possible; otherwise fail the affected action naming the exact batch/object. Surfaces as the bounded editor refusal code `restore_source.disk_corrupt` with the batch/object in debug detail. |
| One immutable source identity has conflicting bytes | `MS-REF-SYNC-CONFLICT` | Provider delivery produced a stable immutable collision. Preserve both pieces of evidence and never select by arrival order or mtime. |
| Reconstruction names the wrong block/home, is malformed, or exceeds an existing bound | `MS-REF-MALFORMED-IMPORT` / `MS-REF-BOUNDS` | Malformed imported/shared operation input attempts identity substitution or excess allocation. Reject before publication or allocation beyond the bound. |
| Imported reconstruction update, source and declared effect disagree | `MS-REF-MALFORMED-IMPORT` | Malformed peer/import input substitutes content, identity, placement, or container operations. Reject the complete batch atomically; install no document or replacement. |
| Restore planning state advances | `MS-REF-STALE-GENERATION` | An honest concurrent operation changed the page/frontier after validation. Re-diff through the existing bounded retry/action cursor; do not publish the stale plan. |
| Checkpoint binding, recovery fence, floor policy, roster or document image is absent, torn, wrong-format or inconsistent | no durable refusal; disposable recovery | Crash/power loss tore a derivative publication. Discard it, sequence-zero replay all retained originals and journals, and publish a fresh base-zero checkpoint. Never delete originals to repair the derivation. |
| Recovery-input parent or object has not arrived | no durable refusal; retryable pending | Honest delivery reordering or interrupted sync delivery. Retain custody evidence, poll delivery cheaply without repeated full reconstruction, and retry the complete retained-envelope cascade when its prerequisite arrives. |
| Original or journal data missing inside a proven durable prefix | `MS-REF-DISK-CORRUPT` | Crash/power loss, torn storage, or media damage removed bytes whose selected journal fence or accepted archive prefix proves they were durable. Do not skip the missing input or advance cleanup authority; preserve every remaining journal/original, identify the affected domain, and retry repair or refuse the affected authority transition. |
| Age alone never selects the below-floor recovery row | no durable refusal; policy constraint | An honestly long-absent device delivers a causally valid old change. Acceptance age may make a floor candidate eligible, but only byte pressure plus the qualified native floor policy can select a cut; age by itself retains the existing floor and therefore cannot manufacture recovery work. |
| Outbound frontier-head publication while a recovery input is pending | no durable refusal; suspended until reconstruction completes | The device holds custody of an original that is not yet in its accepted frontier, so a published head would claim a frontier that omits work it already holds. The suspension latches the pending repair request rather than dropping it, so it runs on the tick after recovery finishes. Enrollment-descriptor repair is explicitly NOT suspended (it republishes the join file verbatim and asserts no head), and inbound delivery is never suspended, because delivery is what ends the wait. |
| Recovery publication fails, or lease/binding changes before installation | `MS-REF-STALE-GENERATION` only for a proved honest authority advance; otherwise retryable pending | I/O/crash failure or an honest concurrent instance invalidates the candidate. Keep the actor paused, retain every original, abandon the stale candidate, and retry under current authority. |

A page shard may transiently hold a **sparse block**: an owner register naming a
block whose text container is absent. Physical deletion removes the owner,
content, Logseq UUID and identity-origin keys together, but Loro resolves each
map key independently, so a concurrent move -- which rewrites only the owner key
-- leaves the owner behind while the content key stays deleted. This state is
honest, not damage, and it lasts until the deterministic conflict actor authors
the settlement.

Two rules follow, and neither is optional:

- **Validation tolerates it; it does not refuse.** Shard validation and working-
  document snapshotting are document-scoped, so refusing a sparse block refuses
  every edit to that page -- including the settlement that clears it. That is not
  hardening, it is an availability bug with no in-scope scenario behind it: it
  wedged the conflict actor into unbounded retry. The malformed-versus-honest
  decision is made where the conflict index is available, in page
  materialization, which reports `MalformedDocument` for a sparse block with no
  unresolved accepted pair (`MS-REF-MALFORMED-IMPORT` / `MS-REF-DISK-CORRUPT`).
- **A reconstruction may land on a live block only when settling a conflict.** A
  page revival that reconstructs a block while another device edits it attaches a
  fresh text container, which wins the content-map key and orphans the container
  carrying that edit; the block is therefore live holding the deletion
  before-image, and the settlement restoring the edit is a `Page -> Page`
  transition. Ordinary reconstruction stays restricted to absence or the
  `Tombstone -> Page` shape. Author-side marking and receiver-side validation
  share one implementation of this predicate, because when they drifted the
  author could build an effect the receiver would have accepted.

One bound, not a refusal, applies to the same path. Editor undo resolves a
deleted identity by walking the page shard's accepted ancestry newest-first,
because the undo stack is not one batch deep: typing in another block and
undoing back past a deletion puts ordinary batches between the two. The walk
returns as soon as it finds the deletion and inspects at most
`EDITOR_RECONSTRUCTION_ANCESTRY_BUDGET` (4096) accepted batches, so the bound
is only ever reached by a lookup that was going to fail. Beyond it, and for an
ancestor whose semantic effect no longer resolves, undo reports the identity as
unknown rather than waiting. Reconstruction depends on the deletion's semantic
effect still being retained; a deletion below the retention floor is not
undoable, by design.

Three retryable refusals are intentionally recorded outside the durable-scenario
table:

| Operation | In-scope scenario | Required response |
| --- | --- | --- |
| local activation fails after retaining its activation marker and immutable baseline but before actor open | Crash or runtime-open failure lands between authoritative retain and the first active actor | Preserve the marker, baseline, archive, and enrollment as one authoritative set; the next explicit open resumes and completes activation |
| shared-join generation publication is interrupted before or after marker replacement | Crash lands between baseline generation publication, operation generation publication, the archive-directory barrier, and the marker commit point | Resolve the baseline and operation archive named by the durable marker, reclaim every unreferenced generation/candidate, and resume from that complete pair; no durable refusal is emitted |
| prepare-share while an absence publication barrier is active | A half-synced folder or dying mount delivers mass absence; publishing the first shared baseline would propagate history-bearing deletions before disposition | Refuse with `external deletions awaiting disposition`; retain all local durability and retry after sweep close/grace expiry or explicit disposition |
| exact provider removal whose caller requires the source present, on a path that is already absent | Sync-service delivery, an honest concurrent instance, or this device's own earlier completed removal has already taken the path; the completed journal record for that removal has since been compacted against provider state (§2.10c-i) | Report `UnknownProviderPath` for the exact path. This is the same answer the `RequirePresent` policy gives for any absent source; the caller re-observes provider state (the clean provider path walk reads the path before it asks for the removal). A caller whose policy is `SettleIfAbsent` settles instead. |

The three internal generation refusals below are pinned to the durable scenario
vocabulary above. They are not new public scenario IDs.

| Refusal stem | Scenario ID | In-scope scenario and required response |
| --- | --- | --- |
| `clean authority orphan is not a private directory` | `MS-REF-UNSAFE-FS-KIND` | Sync delivery, filesystem damage, or an external tool substituted a symlink, special file, or regular file where cleanup owns only private directories. Refuse without following or removing the substituted entry. |
| `clean shared join generation destination already exists` | `MS-REF-STALE-GENERATION` | An honest concurrent instance advanced the generation after this join validated its prior marker. Abort the stale join; reopen follows the marker-named complete pair and reclaims unreferenced generation state. |
| `clean shutdown could not drain the checkpoint publisher` | `MS-REF-DISK-CORRUPT` | A disk/media error or torn write fails the in-flight cold publication that the checkpoint worker performs. The generation worker became an archive writer with the hot-retirement landing, so a settled actor must join it before `Safe`: `Safe` asserts that no writer is left inside the archive, and returning it over a failed publication would be a false claim callers act on. Refuse `Safe` and report the drain error. Recovery is preserved, not removed — marker-last ordering leaves every crash prefix readable, and both callers keep a path forward (`stop_without_clean_drain` for the forced return, the emergency return for the graceful one). |

#### Checks with no in-scope scenario, and what happened to them

The rule that every refusal, fail-closed path, or re-verification of already
established state must name a concrete in-scope failure applies to *silent*
defensive work too. A barrier or re-proof that cannot name a scenario is not
hardening; it is unpaid latency, and later a source of availability bugs.

| Removed check | Where it was | Scenario it claimed | Why it has none | Replaced by |
| --- | --- | --- | --- | --- |
| `fsync` before reading a projection evidence file | `model::sync_and_read_projection_regular` | — | A read through the same process's page cache returns the bytes the writer wrote whether or not they are on the platter. Flushing cannot change the result and cannot detect corruption. | Plain bounded read (`read_projection_regular`) |
| `fsync` before opening-and-reading a projection file | `model::sync_open_and_read_projection_regular` | — | As above. On Windows it additionally forced a write-capable open for a read. | `open_and_read_projection_regular` |
| `fsync` before re-reading a retained quarantine handle | `model::sync_and_reread_retained_projection_file` | — | As above; the handle is the one this process just wrote through. | `reread_retained_projection_file` |
| Per-intent namespace **reservation** artifact (`<intent>.namespace-reservation`) and its refusals | `projection_store::open_intent_namespace` | — | Published before `mkdir` of `attempts/<intent>` and `forensics/<intent>` and re-read on every open. It detects only a directory renamed/replaced *inside Tine's app-private receipt store*, which needs an actor with write access as the user — out of scope. No crash, torn write, disk error, sync delivery, external-editor race, honest concurrent instance, or honest multi-device divergence can rename a directory. A torn or lost 1 KB binding, by contrast, wedged the page's projection permanently. | Absence is recovery: `ensure_directory_nofollow` recreates the namespace and the drain republishes its byte-identical contents |
| Per-intent namespace **authority** artifact (`<intent>.namespace-authority`) and its refusals | `projection_store::open_intent_namespace`, `projection_store::validate_live_intent_namespace` | — | As above. Its device/inode binding re-proved, from a file, a fact the live directory handle already answers for free. | The live `canonical_directory_identity` comparison against the identity the in-flight `DurableProjectionMutationAuthority` already records; no artifact, no barrier |
| `fsync` of every **ancestor** of a projection target's parent chain | `model::sync_projection_chain_with_class` (leaf-to-root loop), reached from ~30 write/rename/preflight call sites | — | The operation changes entry lists in the chain leaf only. An ancestor Tine created in this operation is already flushed by `create_projection_chain_component` at creation; an ancestor it did not create already has a durable entry in its own parent, and no in-scope scenario (crash/power loss, torn write, disk error, sync delivery, external-editor race, honest concurrent instance, honest multi-device divergence, malformed import) can un-durable an entry already on stable storage. See §2.10a-i for the one out-of-ownership case it did cover. | One barrier on the chain leaf, plus the existing per-creation barrier |
| Catch-all refusal of every unenumerated runtime tick | `sync_runtime::clean_shutdown` drain loop | — | The `other =>` arm refused `Safe` for every `SyncRuntimeTick` variant nobody had enumerated, so it named no failure at all. It had silently collected four that are ordinary progress — `ProviderMutation`, `CheckpointCaptureSkipped`, `RetryFull`, and the two non-terminal `LocalMutation` outcomes — and `ProviderMutation` was live: a peer's batch landing while the user quit returned an error instead of `Safe`, which is what kept `oversized_provider_callback_retains_scan_and_safe_shutdown_drains_it` red. | An exhaustive match with no catch-all, so a new tick variant fails to compile until it is classified deliberately; guarded by `the_clean_shutdown_drain_classifies_every_tick` |

The three `fsync`-on-read helpers fired three times per managed save and eight
times per cross-page move. The two per-intent namespace binding artifacts fired
**four times per projected page** (two namespaces x reservation + authority),
costing eight barriers per page: eight of an ordinary save's 45 and sixteen of a
cross-page move's 109. Removing them is the four-artifact cut Martin signed on
2026-08-26 after the refusal census
(`specs/notes/2026-08-26-p-census-receipt.md`) confirmed that no in-scope
scenario relies on them.

The refusals they carried are replaced by recovery, not by nothing: a per-intent
recovery namespace that is absent on reopen is recreated
(`projection_store::intent_namespace`), and everything inside it is
content- or intent-addressed, so the drain — which still holds the undrained
journal frame for the accepted edit — republishes byte-identical artifacts.
`projection_integration_tests::a_missing_per_intent_recovery_namespace_is_recreated_instead_of_wedging_projection`
is that recovery's test; it replaces
`established_per_intent_namespaces_cannot_be_deleted_replaced_or_recreated_after_reopen`,
which asserted the deleted refusal. Integrity of projection evidence is still checked, by the means
that actually detects corruption: `projection_recovery_matches_record` compares
the exact `BlobDescription` and the canonical file resource id. No read path may
reintroduce a durability barrier.

Every public durable open/activation refusal carries its scenario ID separately
from its bounded reason/stage code. Retryable open failures do not invent a
scenario; if a lower storage boundary detects a durable refusal it emits the
literal table ID and the public boundary preserves it.

A managed application/editor refusal must also be *attributable*. An
internal-invariant refusal — one that is not a caller error and has no user
remedy — still names the stage it came from, and an error crossing between the
editor and application surfaces preserves that stage instead of collapsing it.
The reason is a boundary, not a preference: on a platform Tine cannot debug
interactively (Android instrumentation, a user's device, a bug report) the
returned value is the only evidence that exists, and an unattributed
`ActorRefused` makes the failure permanently undiagnosable. An editor rejection
of a request the *application layer itself* constructed is such a refusal: it
is never reported as an invalid caller request, and it is never anonymous.

Attributability is now total rather than aspirational: no call site on the
managed application or editor surface may construct the payload-less
`SyncApplicationPageRequestError::ActorRefused` /
`SyncEditorRequestError::ActorRefused`. Those variants survive only as the
declaration, their two `Display` arms, and the total mappers that re-shape an
already-decided refusal when it crosses between the two surfaces; every origin
uses `ActorRefusedAt`, `ActorRefusedAtWithCode`, or
`ActorRefusedAtWithDebugDetail`. The rule is mechanical, not editorial:
`sync_runtime::tests::managed_save_refusals_cannot_be_constructed_without_a_site_name`
reads the production source and fails on any new bare construction, and on a
collapse of the named-stage inventory. This closes the gap that left the
Android post-activation save reporting `debug_detail="none"` with no stage — a
refusal that could have come from any of 131 unnamed sites.

#### Acceptance-only refusals for drain-published local batches

The managed-local drain publishes a batch's immutable manifest at stage
`ArchivePublication` and only then offers it to the hot engine at stage
`EngineAcceptance` (`oplog/local_journal_drain.rs`). Replay treats a
manifest-committed clean operation that does not validate as accepted as
archive corruption, not as an alternative status
(`hot_engine::replay_clean_committed_batch_ids`: "manifest-committed clean
operation ... did not validate as accepted"). A refusal an honestly drafted
LOCAL batch can meet **only** at acceptance therefore converts a Save the app
reported successful into a store that refuses to open on every later open. That
is the shape A4 removed for the four run-local capacity caps; this census
enumerates the rest of the class.

Scope: every refusal reachable from
`ShardedHotEngine::accept_clean_prepared_below_managed_local_overlay` for a
batch whose origin is `BatchOrigin::LocalMutation` — its own preconditions,
`stage_ready_internal`/`drain_staged`, everything `validate_and_apply` calls,
the quarantine dispositions, and the `accepted_exactly_once` fall-through.

Classes:

- **U** — unreachable for an honestly drafted, drain-published local batch,
  because a draft-time check that is at least as strict runs BEFORE the journal
  append, or because the value is fixed by construction. The draft path is
  `trusted_local_commit::commit_compound` →
  `ShardedHotEngine::prepare_managed_local_record` →
  `validate_managed_local_candidate`, preceded by
  `prepare_transaction_core`, which runs the same page-name and portable-path
  producers the acceptance path runs.
- **S** — reachable only through an in-scope scenario from the table above, and
  the current outcome is already correct: the drain maps it to `recovery`, the
  store stays openable, and the record is retried.
- **R** — reachable for an honest local batch. Fixed; class size is zero.

No row needs a new `MS-REF-*` identifier. The R row no longer reaches any
public boundary (it is now refused at Save, before the journal append). Every S
row reaches the public open/activation boundary only as an already-classified
durable refusal — `MS-REF-DISK-CORRUPT` for a damaged immutable object or
run-local index, `MS-REF-CRASH-TRUNCATED` for a torn record, and
`MS-REF-STALE-GENERATION` for an honest concurrent advance — so §3.1 above is
already complete for this class.

| Refusal stem | Class | Draft-time check (U), in-scope scenario (S), or fix (R) |
| --- | --- | --- |
| `clean foreground acceptance requires an index-free baseline runtime` | U | The drain calls this entry point only on the managed-local clean runtime, and `validate_managed_local_overlay_candidate` refuses a non-`LocalMutation` origin before the journal append ("only trusted local-mutation batches enter the managed-local prefix"). |
| `clean runtime has no operation archive` | U | The drain resolved and duplicated the archive capability at its `Authenticate` stage, before publishing anything. |
| `is not manifest-committed` | U | The drain published the manifest and re-proved it with `exact_archive_batch` in the same call, immediately before this stage. |
| `differs from its retained manifest` | U | Byte equality against the manifest the drain itself just wrote. |
| `did not validate as one accepted operation` | S | Fall-through for `Quarantined`/`IncompleteStaged`; the underlying dispositions are the quarantine rows below. |
| `EngineError::WorkspaceMismatch` | U | Same comparison in `validate_managed_local_candidate` before the append. |
| `EngineError::LineageMismatch` | U | Same comparison in `validate_managed_local_candidate` before the append. |
| `EngineError::BatchCollision` | U | The batch id is minted per draft; a collision needs a second batch with the same id and a different fingerprint. |
| `EngineError::SelfDependency` | U | The frontier is the pre-batch frontier the draft built, and `validate_managed_local_overlay_candidate` re-proves `actual_pre == manifest.dependency_frontier()` before the append. |
| `EngineError::RejectedDependency` | U | Every dependency of a local batch is an accepted batch or an earlier record of the same journal-durable prefix. |
| `EngineError::DuplicateDocumentUpdate` | U | `updates` is a map built by the foreground; the draft validates the same object set. |
| `EngineError::MissingDocument` | U | A non-empty page effect puts the catalog in `updates` by construction; `affected_projection_pages` equality is proven at draft. |
| `EngineError::CrdtUpdateBaseMismatch` | U | `validate_update_base` runs at draft and maps this exact error to `ManagedLocalRecordError::StaleBase` before the append. It checks both the exact starting frontier and each carried peer range: its start counter equals the retained before-vector counter, its end is strictly greater, and every start has an end. Loro metadata supplies the ranges; replayed overlapping operations cannot establish a new writer binding. |
| `EngineError::BlockAlreadyExists` | U | The draft checks accepted claims **and** `local_overlay.block_claims`, so it already sees claims introduced by journal-durable records. |
| `EngineError::MalformedDocument` | U | Document-shape checks over documents the draft built and already validated with `validate_shard` / `validate_immutable_shard_identity`. |
| `EngineError::ProjectionManifest` | U | `validate_managed_local_projection_candidate` re-renders every intent deterministically and compares target bytes and annotations before the append. |
| `EngineError::ProjectionClaimEvidenceMismatch` | U | The claim evidence is recomputed from the same accepted catalog the intent was drafted against; at `EngineAcceptance` for record N the SQLite claim source reflects exactly records 1..N-1. |
| `EngineError::InvalidCrdt` | S | `MS-REF-CRASH-TRUNCATED` / `MS-REF-DISK-CORRUPT`: a torn or damaged update payload. Drain maps it to `recovery`. |
| `EngineError::Archive` | S | `MS-REF-DISK-CORRUPT`: archive read/encode failure. Drain maps it to `recovery`; the store stays openable. |
| `EngineError::ProjectionWork` | S | `MS-REF-DISK-CORRUPT`: raised by `refresh_clean_projection_head_for_batch` AFTER the batch is accepted, so the drain maps it to `recovery` and retries; acceptance itself already stands. |
| `history_failure` | S | `MS-REF-DISK-CORRUPT`: an earlier publication failure in the same run already made the workspace terminal. |
| `canonical page-name key is occupied at the declared dependency frontier` | **R → fixed** | Was the one acceptance-only refusal reachable from ordinary editing: the run-local page-name index was blind to journal-durable-but-unaccepted records, so two pages whose exact names differ but whose canonical keys are equal ("Alpha" and "/Alpha") both passed the draft and the second was refused at acceptance after its manifest was published. Fixed by layering `CommittedLocalOverlay::page_names` under the accepted index in the single producer (`prepare_page_name_updates`), exactly as `portable_path_records_many` already layers the overlay for portable paths. The refusal is now raised at draft time, before the journal append. Guarded by `w5_census_a_canonical_page_name_collision_is_settled_before_the_journal_append`. |
| `two PageIds acquire one canonical page-name key in the same batch` | U | Same producer, same effect, at draft. |
| `page-name transition observations are incomplete or non-unique` | U | Same producer, same effect, at draft. |
| `page-name transition disagrees with the authenticated dependency catalog` | U | `exact_before` is derived from the effect itself in both calls. |
| `conflicts at the declared dependency frontier` | U | Portable paths: `portable_path_records_many` layers `local_overlay.portable_paths` over `ephemeral_portable_paths`, so the draft already sees paths acquired by journal-durable records. This is the shape the page-name fix above copied. |
| `PageNamePointBatchTooLarge` | U | `MAX_PAGE_NAME_POINT_BATCH` is charged against the same delta count at draft and at acceptance, so it can only ever refuse at Save. |
| `MalformedPageNameIndex` | S | `MS-REF-DISK-CORRUPT`: the run-local index or its checkpoint is damaged. It is disposable derived state and is rebuilt from the accepted tail on the next open. **This row also had a second, non-corruption cause, and for that cause neither the scenario nor the recovery held.** `prepare_page_name_transition_core` resolves its records through a bounded point view the CALLER pre-fetched, and `prepare_page_name_updates` derived that key set independently of the core's. Where the two disagreed, a record that exists read back as a proven absence and the transition reported a malformed index over undamaged state — and because nothing was damaged, reopening rebuilt the same disagreement rather than repairing it. Both sites now derive their keys from `page_name_transition_keys`, guarded by `page_name_transition_keys_has_exactly_two_production_callers` and `transition_keys_cover_prospective_and_tombstoned_observations`. A future caller that derives its own key set reintroduces the shape. |
| `MissingExactLogicalPageNameBlob` | S | `MS-REF-DISK-CORRUPT`: as above. |
| `portable_path_blocked` | S | `MS-REF-SYNC-CONFLICT`: a non-ancestor concurrent acquisition of the same portable path. Quarantine retains the batch as validated-unpublished evidence rather than losing it. |
| `page_name_blocked` | S | `MS-REF-SYNC-CONFLICT`: a non-ancestor concurrent acquisition of the same canonical page-name key. Same quarantine treatment. |
| `identity.blocked` | S | `MS-REF-SYNC-CONFLICT`: an immutable block-home claim conflict delivered by a peer. |
| `is_blocked()` | S | The workspace already carries terminal conflict evidence from one of the rows above; batches offered afterwards are deliberately retained, not discarded. |
| `allow_publication` | U | `drain_staged` always passes `true`; only `drain_blocked_evidence` passes `false`, and it runs only when the workspace is already blocked. |

The census is pinned by
`hot_engine::validation_tests::w5_census_pins_every_acceptance_refusal_to_a_class`:
a new refusal in this path with no row here, or a row whose stem no longer
exists in production, fails that test.

### 3.1a The private receipt-store claim, and when it is checked

`receipts/projection-receipts.claim` identifies the one implemented private
receipt-store format by magic. The current claim is **`TINEPR7\0`, `STORE_CLAIM_VERSION` = 7**.
Earlier development magics — `TINEPR6\0`, `TINEPR5\0`, `TINEPR4\0`,
`TINEPR3\0` — are
recognized only so the low-level opener can refuse them without mutation. They
have no reader, compatibility implementation, or migration path.

**Why the version moved to 7.** Local forensic evidence previously accepted a
schema-1 record beside the schema-2 current record. Removing that private
dual-decoder requires invalidating its containing store too: a TINEPR6 store is
now rejected at the claim precheck rather than failing later on an unreadable
record. Managed storage has not shipped, so the 0.7 blank-slate policy applies:
the Tauri graph-open boundary preserves the entire unrecognized private root as
a backup, opens the untouched Markdown/Org tree as the reconstruction source,
and automatically activates a fresh store in the one current format. The user
does not migrate or manually re-activate anything.

**Why packet 2c does not move it again.** Packet 2c retires only the
own-endpoint facet of the receipt protocol. The foreign receiver namespaces,
record formats, and recovery protocol remain live and unchanged, so the
wholesale-retirement premise for a claim bump was false by itself. A store written by
a pre-2c `(c)` build differs only by possibly retaining own-endpoint receipt
artifacts. Current code neither authors nor consults those artifacts as
authority: it reports their validated names, leaves their bytes untouched, and
recovers own work exclusively from the durable turn/journal plus the local
completion index. The independent forensic-decoder retirement above is the
containing-format reason the claim now moves; packet 2c still contributes no
additional format change. The real-store recovery-equivalence oracle covers
every specified crash cut for the 2c transition.

| Claim observed | Response |
| --- | --- |
| current magic, current version, exactly `STORE_CLAIM_LEN` bytes, regular file | proceed to the full in-place validation |
| current magic, any other length, or a non-regular file | `MalformedStoreClaim`, refuse, zero mutation |
| a prior magic, or a version below the current one | `UpgradeRequired`, refuse, zero mutation; the graph-open boundary archives the private root and automatically rebuilds current state from the intact Markdown/Org tree |
| a version above the current one | `UnknownStoreVersion`, refuse, zero mutation |
| absent, on a populated store root | `ClaimlessNonemptyStore`, refuse, zero mutation |
| absent, on an absent or empty store root | initialization owns it; this is a fresh store |

**Refusal scenario** (§3.1 rule): `MS-REF-PROTOCOL-INCOMPATIBLE`. The in-scope
failure is an honest pre-(c) private store meeting a (c) build — reachable with
no attacker and no corruption. The low-level refusal is the typed signal for
the outer blank-slate lifecycle; it is not a user-facing migration request.

**Where the check runs, and why there.** The full validation has always been the
first thing the receipt store's `open` does, and it stays there as defense in
depth. But on the clean cold-open path that `open` happens *after*
`Graph::open_checked`, and `Graph::open_checked` is **not read-only**: its
publication recovery renames graph files and moves artifacts to `.trash/`. A
store this build cannot serve must not get that far. So a read-only **claim
precheck** — `ProjectionReceiptStore::precheck_authoritative_claim` — runs at the
HEAD of the clean cold open, immediately after clean-authority discovery returns
and before any other step. It applies only to an authoritative store, where the
claim provably predates the authority marker; a fresh store has no claim and
initializes exactly as before.

The precheck holds a current-magic claim to the **exact** version-specific
envelope length. A magic-only check would pass a truncated claim, and graph
publication recovery would then run before the in-place length check ever fired.

A refusal propagates through the managed-open failure channel as a named notice
(`OpenRefused`, carrying the scenario marker). During startup the Tauri
graph-open boundary treats that exact protocol-incompatible marker, and an
unrecognized outer binding, identically: archive the whole private root,
publish Direct Files as the intact source, and automatically construct and
publish the current managed store. The Direct Files selector durably records
that this is a pending blank-slate rebuild before private state is moved, so a
crash at any later cut or a failed first reconstruction retries automatically
on the next open. If reconstruction itself fails, Direct Files remains serving
and the failure is retained in diagnostics. The original unrecognized private
root is archived once behind a durable completion marker. Later failed
reconstruction candidates are reconstructible and rotate through one bounded
recovery slot rather than minting an unbounded archive on every launch. An
explicit or emergency Direct
Files selection does not request this retry. This is one current format, not a
compatibility reader or migration.

Both private-root moves (`archive_private_root` and the bounded failed-candidate
replacement) synchronize the recovery destination parent first and the source
parent second after rename. A power loss therefore cannot acknowledge removal
of the private-root name without also making its retained recovery name durable.

#### Explicit target kind on intent and completion records

An absent target flattens to the empty blob description, so byte length cannot
tell "this page renders to nothing" apart from "this page must not exist". Both
`ProjectionIntent` and `ProjectionCompletion` therefore carry an explicit
`target_kind` (`present` | `absent`) in their canonical encoding, from store
creation. There are no legacy records to classify.

Two constraints hold today, and both are tested:

* a record declaring an absent target may not declare target bytes;
* `ProjectionIntent::id()` is **unchanged** — the kind is a stored field, not an
  identity input — while `matches_replay_except_frontier` and a completion's
  binding to its intent both compare it.

The absence-decision map reads this field directly from both completion halves.
Nothing may infer absence from byte length.

### 3.2 Clean-runtime save settlement

An eligible ordinary application-page save has a foreground acceptance lane.
After the semantic transaction and exact projection have been prepared, Tine
commits the exact graph bytes together with one append to the device-private
foreground journal, installs that journal record in the hot semantic overlay,
and may then report the new page and revision. Immutable archive publication,
SQLite materialization, provider publication, and journal checkpoint/compaction
are derivative work advanced after the foreground response. Reads combine the
accepted SQLite baseline with the exact pending journal suffix; they must never
answer from either one alone when the other may change the result.

The append result is a commit boundary. A definitely-not-appended failure may
refuse the save normally. If the filesystem reports an uncertain outcome after
the append may already have become durable, that actor becomes terminal and
accepts no further edits. Restart replays the authenticated journal and either
recovers the one accepted operation or refuses recovery; retrying the edit in
the same process could otherwise duplicate it. Replayed task-query overlays
begin as incomplete and force the complete evaluator until their bounded sparse
facts have been reconstructed, so stale SQLite can never hide a journaled edit.

The recovery-input journal has the same WAL boundary but a distinct settlement
meaning. Before a durable append the provider item stays queued and unacknowledged.
After a complete durable frame, custody may be acknowledged even though semantic
acceptance awaits full-history reconstruction. An uncertain result reopens and
resolves the exact BatchId and bytes; it never appends an equivalent edit. Its
selected generation cannot advance or be cleaned until successful actor
reinstallation proves exact accepted coverage.

Foreground-journal compaction publishes a complete successor generation before
retiring its predecessor. Failure to retire the predecessor is retryable
cleanup, not permission to forget it: reopen selects the greatest authenticated
generation and retries removal of every older tuple before advancing derivative
work.

Cross-page subtree movement uses the same foreground boundary as a page save.
The source and destination CRDT updates and exact projections are one compound
journal record; once that record is durable, both pages enter the hot overlay
atomically and the application may return them without waiting for archive,
SQLite, receipt, or provider derivatives. A subsequent move composes with the
latest pending `(page, path)` projection through an exact in-memory index; it
must not scan the pending journal prefix. Pending records are decoded once on
recovery or append; derivative turns point-query `(path, page, sequence)` target
and digest postings, and uncertain move retries point-query the pending batch
identity. The derivative may read only the affected page identities and
materialize those pages from the retained accepted catalog proof. It must not
decode or validate the graph-sized catalog merely to apply a bounded move.
Rapid application commands are serialized before their source/destination
intent is resolved, so each command observes the accepted result of its
predecessor rather than replaying a stale page pair. Once the frontend installs
the actor-returned source/destination DTOs, it acknowledges that exact episode
and batch. The actor revalidates the canonical episode, its completion proof,
workspace/lineage binding, and accepted-or-visible semantic batch before
retiring the two response-replay sidecars. The oplog batch remains authority;
acknowledgement can only bound private response evidence and cannot undo or
re-run the move.

Page rename discovery follows the same bounded-work rule. An ordinary rename
may point-read the exact normalized source and target names and range-read the
source namespace descendants; it must not enumerate the graph page inventory.
Collision-rename/merge uses the identical name and namespace indexes before its
reference rewrite. Work may scale with the renamed namespace and actual
referrers, never with unrelated pages.

Likewise, admission tracks its exact live staged set. Final status history may
remain available for point answers, but an ordinary drain turn must never scan
that lifetime map merely to rediscover the handful of currently staged batches.

Advancing the clean runtime's authenticated accepted-frontier roots follows the
same rule. The document overlay and accepted-batch maps are persistent
path-copying authenticated trees: one accepted operation updates only its
changed document keys and its one new batch key. It must not clone, sort, or
rehash every document touched earlier in the run or every earlier accepted
batch. The incrementally maintained root is required to be byte-identical to a
canonical complete rebuild; the complete rebuild remains only a differential
oracle and an explicit rebuild operation.

Checkpoint-generation support obeys the pre-0.7 blank-slate rule: production
implements one current format, not old/new readers or a migration bridge. The
canonical authenticated-map priority/node algorithm has one owner,
`tine-storage::sealed_accepted_index`, shared by the clean runtime and
SQLite. The same module owns the one current sealed batch/status/sequence/causal
encoding and its bounded cross-checking reader; its caller-provided Tine
evidence decoder validates the one current accepted-evidence encoding without
reversing the crate dependency. The **R1b accepted-cutoff builder** streams engine
accepted rows through the same row writer as A5, starting at a sequence-zero
frontier or continuing a previously built cutoff. It checks contiguous evidence,
exact predecessor/target frontiers, unique batch membership, the resulting
causal batch-map root and the new status/sequence/causal point proof. Continuation
reads persistent index paths and the new row, not the covered status/sequence
inventory. Failed appends leave predecessor roots unchanged; unreachable
construction nodes do not confer authority. The cutoff also binds exact highest
accepted `(peer, counter, batch)` tips through the shared causal-tip value digest
and authenticated map. Delta builds update only affected peer paths; older
accepted counters cannot lower a tip. Empty-tail continuation retains exact tips.
Damaged tip nodes or conflicting batches at one tip fail before candidate roots
advance. The O(peers) in-process records are not yet live authoring input.
The accepted-cutoff builder remains an in-process construction surface with no
serialized cutoff token. The live v2 checkpoint uses the engine's existing
accepted-row seam and accepted document dependencies; it does not adopt the
cutoff builder as a second acceptance authority. A separate construction per-document builder
uses the existing accepted-root loader and document validators, exports a shallow
snapshot at the exact accepted frontier, then imports it into a fresh document.
Qualification requires exact version vectors, oplog frontiers, root and nested
container IDs, map values and text deltas. Run-local Loro allocation slots are
excluded from identity. It never recreates documents from visible page text. This
synchronous staging proof has no installation path and makes no COW actor-budget
claim; old concurrent branches still require full-ancestry recovery.
The engine-level recovery fixture now reconstructs original accepted ancestry,
includes acknowledged post-cutoff work, admits two independently returning peers
through normal replay, and recompacts all affected documents. It covers a block
moved to another page while retaining its original home shard. Its negative control
omits the post-cutoff tail and detects the resulting lost edit. This proves the
isolated engine recipe, not durable installation, missing-history recovery, actor
concurrency, or shared retirement.
That construction helper is not itself a durable generation. The v2 disposable
checkpoint separately binds its live image roster, accepted cutoff, semantic
state, and publication order. The physical layer has
one current SQLite schema for both the live disposable projection and a
separately built checkpoint candidate, plus a read-only injected sealed-history
reader. It has no prior-schema enum, reader, compatibility fixture, or
migration path. An unrecognized pre-0.7 private store is preserved as a backup
and rebuilt from the untouched Markdown/Org tree by Tine.

The **sealed generation staging directory** now persists the existing canonical
node/record bytes as `sealed-v2-<kind>-<digest>` files under a caller-owned private
directory capability. The numeric kinds are the same five codes used by A5's
adapter; there is no second node codec. Its point reader does not enumerate the
directory. Reads reject non-regular/symlink entries through the shared reader and
retain the current checkpoint's 512-MiB per-record read ceiling. The same layout
is now the immutable-image portion of the disposable v2 checkpoint; it remains
non-authoritative and cannot replace accepted originals.

Construction reuses `ExactImmutablePublicationBatch` on Linux. Windows, macOS,
iOS, Android and other targets use retained `DurableDirectoryPublication` with
`publish_new_exact_single_writer`; this preserves Windows write-through and
Android private-directory rename fallback. Pending bytes use the existing memory
adapter. It flushes at 8 MiB or 64 objects, bounding both payload
memory and retained directory capabilities. A larger legal single record flushes
alone; these batch budgets do not cap graph size or total history. Failed writes
poison the construction handle; dropping it never installs a generation marker.
Successful finish completes the shared durability protocol. Fresh canonical
membership qualification still follows: a durability receipt is not an integrity
proof, and merely opening a directory is not generation adoption. Collision,
missing/corrupt record, no-follow, publication-fault and retry tests preserve
predecessor roots. The v2 payload binds the finished image root only after
staging finish; the existing generation and `current` pointer remain the sole
selection boundary.

The retained **document capsule roster** codec uses `SealedDocumentMap`
over the shared full-key authenticated map. `DocumentKey::Entity` encodes as the `0x01` tag plus
its UUID (17 bytes), while `DocumentKey::Membership` encodes as the `0x02` tag
plus the complete `(block_document_id, page_document_id)` pair (33 bytes). That
lossless encoding is `AuthenticatedMapKey`; entity and membership entries share
one root, preserve meaningful-byte lexicographic ordering, and never hash or
truncate an address. The live checkpoint producer supplies entity rows only:
the live write path addresses catalog and page-shard documents by their 16 UUID bytes, and a
page-document adapter wraps a UUID as `DocumentKey::Entity` only at this codec
boundary. Logical retirement uses the certified shared `remove_map` operation, which removes only
the addressed search path while preserving older immutable roots; it never
physically deletes objects. Construction reads pending staged nodes before
on-disk nodes, and a fresh complete-key census independently rebuilds the shared
root and verifies its count. A map value addresses one canonical postcard record
`{schema=1, dependencies: DocumentDependencies, checkpoint: BlobDescription}`.
Descriptor and actual CRDT checkpoint blobs share the `capsule-v1-<digest>`
content-addressed staging namespace and the same bounded publication machinery.
The descriptor preserves exact accepted heads and counters; it contains no
run-local cutoff digest. A changed document replaces only its roster search path;
unchanged values remain shared. Catalog identity and roster completeness must still
be bound and qualified by the enclosing complete generation.

Point reopening checks canonical current descriptor encoding, digest, document
identity, exact checkpoint length/digest, CRDT import completeness, schema/shape
and version vector through the existing catalog/shard validators. It never
reconstructs a CRDT from projected text. A forged in-memory cutoff cannot be
serialized through this surface; producer input remains an engine-qualified compact
document bound to the current cutoff. The active v2 payload stores this roster
root; engine restore receives it only after the payload and archive bindings
qualify.

The explicit full-roster builder enumerates the engine's complete accepted
document set, including the catalog and retained home shards that need not appear
as live pages. It checks the current accepted cutoff and exact document count. An
empty accepted set keeps the existing implicit empty catalog; a nonempty set must
contain the catalog.
An inherited descriptor with identical canonical dependencies avoids CRDT loading
and republishing; changed documents update the existing roster paths. The older
explicit full-roster builder remains a repair/oracle seam. Ordinary v2 capture
uses the published dependency map to hand off only changed documents, while its
complete manifest metadata still enumerates the current live document set.

The full bootstrap/repair oracle independently derives the canonical roster root
from the exact accepted document keys, so extra entries cannot hide behind a forged
count. Each persisted document must then match accepted dependencies and the
source CRDT's stable container identities, frontiers, map values and text deltas.
Both the per-document compact producer and full-roster oracle reuse one equivalence
checker. The full-roster oracle remains an explicit construction/repair
operation; the live runtime uses point descriptor qualification and imports only
resident or requested images.

### Additive cold whole-object history and one read resolution

R2's cold tier is a **physical** layer below the existing object model. It changes
where a logical object's bytes live and nothing else: canonical `OperationObject`
and `OperationBatch` bytes, their content digests, their `ObjectDescriptor`s and
their `BatchId`s are preserved verbatim, and every read re-proves them. There is
one current representation and no migration path; an unrecognized private store is
still backed up and rebuilt.

Layout, under the retained archive capability beside `clean-open-checkpoint-v2`:

```
<archive>/cold-history-v1/
  pack-v1-<uuid>            immutable pack file
  sealed-v2-<kind>-<digest> shared authenticated-map nodes
  current                   canonical root marker, installed last
```

A pack is `record* footer footer_len:u64be "TINECLD1"`. A record is
`sha256(payload):32 payload_len:u64be payload`, so one ranged read self-verifies
its payload. The footer makes each pack self-describing, which is what keeps the
locator index disposable derived state: `repack_cold_history` rebuilds every root
from pack footers alone. Packs are built to a 4 MiB construction target; that is a
target, never an occupancy cap, and one larger legal record is packed alone.

`ColdLocatorV1` is exactly `pack_uuid:16 offset:u64be length:u64be` = 32 bytes, so
it occupies the shared authenticated map's fixed value slot directly. No side blob
and no filesystem object exists per logical record: many small records share one
pack and one point read.

Two key domains compose the same shared canonical UUID map; there is no second
tree, no tuple hashing and no SHA-256 truncated into a UUID. This cold object
index remains its own two-level 256-bit content-digest composition; unlike it,
the dormant `SealedDocumentMap` codec stores each lossless tagged entity or full
membership address directly in one shared authenticated map. The **object domain** carries
the full 256-bit digest through an outer map keyed by
`sha256[0..16]`; its value locates a canonical postcard **inner-root descriptor**
`{schema=1, root: {count, root_key, root_digest}}` packed with the same physical
pack machinery, and that descriptor names an inner authenticated map keyed by
`sha256[16..32]` whose values are the record locators. An outer entry therefore
holds a whole map, not a list: the number of objects sharing one 128-bit prefix is
unbounded, there is no bucket byte cap and no fixed-occupancy refusal, and
lookup cost under a prefix is a map path rather than a scan. The descriptor's
fixed 256-byte read bound is the codec size of that constant structure and is
independent of prefix occupancy, history size or graph size. The **manifest
domain** keys the map by the `BatchId` UUID and locates the record directly. One
object lookup costs `O(log n)` outer map-node reads, one bounded descriptor read,
`O(log m)` inner map-node reads and one bounded payload read — two pack reads
however large history or a prefix grows; one manifest lookup costs exactly one.
No pack, manifest or object namespace is enumerated on any lookup path.

Publication is **additive**: nothing here retires a hot original, and a repack
publishes new packs and swaps the root while leaving the predecessor packs in
place (publish-new-before-retire-old). Enabling deletion needs the generation,
fallback and retention proofs that follow. Publication order is payload pack
bytes, then the inner prefix maps, then the packed inner-root descriptors, then
the outer and manifest maps, then the root marker; before the marker installs,
every staged pack and index node is unreferenced residue and the predecessor root
is untouched, so an interrupted publication never becomes authority through
filename presence.

A repeated identity is "already archived" only when its **bytes** are
byte-identical to what cold history already holds, compared through the shared
reader. A `BatchId` is an identity, not a content address: two valid canonical
manifests may legitimately carry the same `BatchId` and differ (a different
`SessionId` alone suffices), so a repeated `BatchId` with different bytes is a
collision, refused as `ColdManifestConflict` with the batch named. The exactness
comparison runs before any record is appended, so a refused conflict leaves the
predecessor root, the original bytes and the pack set exactly as they were.
Publication, repack and footer reconstruction all apply the same rule; object
records are additionally bound by their content digest, which every read
re-proves. Republishing byte-identical content is a counted no-op that leaves the
roots byte-identical.

The root marker is derived state; the self-describing packs are the truth. No
cold directory, or a cold directory with neither marker nor packs, is ordinary
absence. A cold directory whose packs survive but whose marker is **gone** is a
named damaged state, `StoreError::ColdHistoryRootMissing` — never ordinary
absence, and never a licence to publish a fresh empty-based root over the old
history: reads, publication and repack all refuse it by that name. A **torn or
otherwise malformed** marker over surviving packs is the same damaged class:
ordinary reads reject it as `ColdHistoryIndexUnavailable`, and repair treats it
exactly like a missing one.

`repair_cold_history_root` is the explicit recovery for both. It reads the raw
marker bytes once — using them only as the audited replacement guard for the
marker-last swap, never as history — rebuilds the locator index from the pack
footers, and reuses every surviving record exactly where it already lies, without
rewriting, relocating or re-encoding a byte. A healthy marker makes it a bounded
no-op. A damaged marker with **no** surviving pack is never replaced with an
empty history: there is nothing to rebuild from, so it refuses and leaves the
bytes alone. Repair is the only operation that enumerates cold history; healthy
opens stay bounded point operations, and the damaged-state check short-circuits
at the first pack it sees. Throughout a damaged state and its repair, the hot
tier and current state stay usable.

Read resolution has exactly one implementation, on `ObjectStore`. Ordinary reads
-- `inspect_batch`, `read_object`, `read_object_bytes`, `read_manifest`,
`read_manifest_bytes`, `contains_object` -- are **hot-only** and have no cold
branch at all: page open/save/move, the SQLite applier, projection payload pins
and the operational coordinator's admission path can never reach a pack byte, and
`ObjectStoreStats::cold_object_reads` / `cold_manifest_reads` are the oracle for
that. The **indexed-cold** surface is `resolve_logical_object_bytes`,
`resolve_logical_object`, `resolve_logical_manifest_bytes`,
`resolve_logical_manifest`, `contains_logical_object` and
`inspect_batch_with_cold_history`: each reads the hot original first and falls
through to one indexed cold lookup. It is reserved for the named historical
consumers -- Restore, historical admission, republication, previous-generation
fallback and full replay. SQLite projection rebuild (the full replay of accepted
history) resolves through it today; the remaining historical consumers are routed
as their owning modules are cut over.

Missing or corrupt cold data is history authority, never active-state authority.
A damaged pack, record, locator or map node refuses with the affected logical
object named (`cold logical object <digest> is unavailable: …` /
`cold logical manifest <batch> is unavailable: …`), while every other object, the
whole hot tier and current state stay usable; a damaged or noncanonical root
marker refuses only the index. An archive that has never relocated anything is
ordinary absence, not a refusal. No asset bytes enter these packs.

The separate **live graph document closure** is captured at an exact accepted
cutoff from the existing canonical graph view: catalog (when accepted), live page
homes, and every visible block/membership's immutable home. Deleted page homes
remain live when they own moved blocks. Fully historical shards are excluded from
this eager graph closure while their archive/history obligations remain intact.
Stale closures are refused. Full-history and live-closure roster construction and
qualification share the same entry writer and equivalence checker; the selected
key set is explicit and cannot substitute for full accepted-history completeness.

This capture still uses the current canonical graph view and is a bootstrap seam,
not bounded ordinary maintenance. A fixed-live create/delete probe demonstrates
that the current catalog's application tombstone values survive shallow snapshots.
The live capsule count can be bounded while that one catalog capsule still grows.
Historical catalog representation, retention roots and incremental capture must be
resolved before this closure is used for live adoption or portable join; no ordinary
runtime caller exists yet.

Provider frontier publication likewise consumes an incrementally maintained
set of direct frontier tips rather than materializing every document frontier.
Clean projection attach rebuilds an exact path-to-latest-batch map during
accepted replay and decodes only current path heads after the endpoint becomes
available; it must not replay all accepted manifests merely to locate terminal
projection work. While a later foreground suffix remains application-visible,
projection of its accepted prefix is authorized against accepted state, not
against those later journal-only catalog heads.

A block-only peer operation is also page-local at this boundary. A receiver may
have concurrently advanced the catalog by adding or renaming an unrelated page;
that graph-wide frontier difference must not refuse the peer operation. The
receiver authenticates the exact current identity rows for every affected page
and holds their name, path, home, and kind to the manifested projection. A
conflict on one of those rows remains a refusal; an unrelated catalog advance
does not.

These are work-shape requirements, not thread-placement advice. Moving an
O(graph), O(history), or O(pending-prefix) operation to a background turn does
not satisfy the contract. The 100/10,000-page move receipt and forbidden-work
counters enforce graph-size-invariant foreground work and the absence of a
whole-catalog derivative validation.

The clean baseline-plus-manifest runtime and the retired legacy coordinator are
two **distinct** retained-publication state machines, and a request may never be
routed from one into the other.

A clean local mutation that reaches its manifest commit and then fails to apply
disposable derived state (SQLite and/or exact Markdown projection) returns
`CleanActorMutationOutcome::DurablePending` and retains an affine continuation
in `CleanRuntimeActorCore::pending`. That continuation is advanced only by
`retry_pending_with_turns`. The legacy coordinator's `PendingLocalMutation::Published`
continuation is a different object that the clean actor never writes.

Every clean continuation resume — inline after the manifest commit, and on
every retry — drains Markdown projection through the projection-turn journal:
the resume appends the batch's `IngressLocal`/`IngressForeign` turn when the
journal does not already retain it, and replays turns in order. There is no
turnless projection arm: a managed projection mutation without a turn-derived
attempt identity is refused in production
(`ProjectionStoreError::MissingTurnAttemptContext`), and the 2026-08-28
regression fix deleted the pre-turn resume executor that violated this. The
refusal's in-scope scenario is not an external adversary but the codebase
itself: it turns a silently identity-less projection write into a loud,
recoverable failure, and the `clean_recovery_turns` integration test holds the
drain green at the non-`cfg(test)` boundary where the unit suite's
deterministic attempt-identity fallback cannot mask it.

Therefore, when the clean runtime is installed, an application save that lands in
`DurablePending` settles through the clean actor, bounded by
`MAX_EDITOR_SETTLE_TURNS`. Exactly two outcomes are permitted:

- the retained continuation settles, and the request reports **applied/saved**;
- it does not settle within the budget, and the request reports
  **`Deferred { RetryableRetainedPublication }`**.

A **refusal is forbidden here**, and has no entry in the §3.1 table, because it
would defend against no in-scope failure: the manifest commit is already
durable, and the outstanding work is disposable derived state whose contract is
recovery, not refusal (G2). A refusal with no in-scope scenario is an
availability bug. This is not hypothetical — routing the clean outcome into the
legacy settlement returned `ActorRefusedAt("require_pending_publication_absent")`
for *every* clean-runtime save, which is why Android managed saves never worked.

Two further rules keep the settlement honest:

- A retained continuation belonging to an **earlier** batch is reported as
  `CleanActorMutationOutcome::RetainedPriorPending`, never as the caller's own
  `DurablePending`. That submission never executed, so settling the earlier
  batch must defer the request rather than report it saved with the page's old
  bytes.
- The failure that caused the retention is reported separately from the save's
  own outcome (`SyncRuntimeHandle::last_retained_publication`). A converged
  retry produces an ordinary successful save while the underlying cause still
  costs a retry on every write; the Android instrumentation receipt carries this
  report on both the success and the failure path so that cause stays visible.

The structural claim — a clean runtime never reaches the legacy publication
settlement — is enforced by
`clean_runtime_application_save_never_enters_legacy_publication_settlement`,
which asserts the actor's legacy-settlement counter stays at zero, not by this
paragraph. Durable foreground saves now enter the journal continuation directly;
the pre-journal retained-publication settlement described by older revisions of
this section is retired.

The settlement budget is an upper bound, not a target. A retry that reproduces
the **same phase and the same failure detail** as the previous turn has made no
progress against a deterministic failure, so the loop stops at that second
identical observation and defers once. Spending all `MAX_EDITOR_SETTLE_TURNS`
turns on a permanent failure buys no chance of settling and charges the whole
cost to the user's save. Any change of phase or detail counts as progress and
keeps the loop running to the budget.

### 3.2a Projection turns, recovery input, and independent local journals

A **projection turn** is one authoritative unit of graph-tree publication work.
A durable turn is the whole authority for the names its replay may create and
for the pages it may publish. Turns are `oplog::hot_engine::ProjectionTurn`.

**Three sequence domains, three physical segments.** Managed-local append requires
its physical journal sequence to equal the hot semantic overlay's next
sequence, and only applying a *semantic* managed-local record advances that
overlay. A projection-only record placed in the foreground journal would
therefore consume a physical sequence with no semantic transition: it could
never drain, and the next ordinary save would fail the equality check. So:

| Domain | Segment | Counter | Producers |
| --- | --- | --- | --- |
| `ManagedLocal` | the foreground journal, `managed-local-journal/clean-workspace-{workspace}-{lineage}/` | the hot overlay's sequence, physical == semantic | foreground local authoring |
| `ProjectionTurn` | the projection-turn journal, `managed-local-journal/projection-turns-{workspace}-{lineage}/` | its own monotonic counter, which never meets the other | ingress, terminal repair, superseded repair |
| `RecoveryInput` | the receiving-endpoint-scoped recovery-input journal under private app data | its own monotonic counter and selected durable generation/prefix | exact below-floor provider originals awaiting full-history admission |

All three use the same `LocalJournalSegmentV2` WAL and checkpoint independently.
Recovery-input frames bind transport identity and exact canonical
manifest/object bytes; unlike a managed-local frame, they are custody rather
than locally authored semantic work. Projection-turn **anchors are keyed by endpoint** (
`endpoint-{endpoint}-selector-{generation:020}.anchor-v2`), from store creation:
one grammar, no dual format, because a pre-(c) private store never reaches
journal selection — the receipt-store claim precheck refuses it first.

**Projection-domain turns carry authorization and names, not bytes.** Only
`ManagedLocal`-domain pages may carry precondition/target bytes, because the
foreground frame already carries that material. Projection-domain replay
re-derives the current bytes from the accepted state *at replay time*, so a turn
recorded before a newer merge simply publishes the newer render.

#### Projection turn derivation schemes

Every name a turn's replay may create is derived from the record alone. The
`derivation_scheme` field is both a field and a hash input, so a build never
re-derives with a scheme other than the one the record names, and a scheme
implementation is never deleted while any on-disk record may reference it.

| Scheme | Status | `turn_id` domain separator | Attempt-id domain separator |
| --- | --- | --- | --- |
| `derivation_scheme` = **1** | live | `tine/projection-turn/v1\0` | `tine/projection-attempt/v2\0` |

Count of live projection-turn derivation schemes: **1**.

`turn_id` is `SHA-256` over the domain separator, big-endian
`derivation_scheme`, workspace UUID, lineage digest, device UUID, endpoint
UUID, the one-byte sequence-domain discriminant (`0` = `ManagedLocal`,
`1` = `ProjectionTurn`) and the big-endian sequence. `attempt_id(i)` is an
RFC 9562 version-8 UUID over `turn_id`, the big-endian page index and the page
UUID. The receipt-store resource id deliberately does not participate, so a
turn's identity is a function of the record and nothing else. A record naming a
scheme this build cannot evaluate is `MS-REF-PROTOCOL-INCOMPATIBLE`: the record
is preserved, the turn is not replayed, and the page is reported. It is never
treated as absent.

#### Torn versus corrupt: the local-journal WAL rule

The discriminator is the segment's **durable frontier**, not a heuristic.
`LocalJournalSegmentV2` file-flushes a frame and durably publishes the successor
frontier before append returns, and open validates every byte inside the
committed frontier while truncating only bytes beyond it.

| Case | Classification | Action |
| --- | --- | --- |
| bytes beyond the durable frontier | the append never returned, so by turn-before-mutation no graph mutation for those bytes can have started | the segment truncates them; nothing is owed; no residue probe exists or is needed |
| any invalid frame at or below the frontier, tail or interior, checkpointed or not | `MS-REF-DISK-CORRUPT` — a disk/media error damaged an authoritative record whose effects may exist | refuse activation; retain the segment bytes as evidence; report the component. Never truncate, never skip |

Open failures are classified **per variant**, because `open_selected` also
reports states whose in-scope scenario is not disk corruption
(`oplog::local_journal_drain::LocalJournalOpenRefusal`):

| Open failure | In-scope scenario | Refusal |
| --- | --- | --- |
| corrupt frame, frontier violation, device/sequence binding failure | disk or media error damaged an authoritative record | `MS-REF-DISK-CORRUPT`, evidence retained |
| segment already open, prepared artifact exists | an honest second Tine instance holds the segment | the existing concurrent-instance refusal; nothing is corrupt |
| unsafe segment name, unsupported durable replacement | the journal namespace holds an entry this platform cannot safely open | the existing unsafe-filesystem refusal |
| I/O or capability failure | transient storage unavailability | retryable; asserts nothing about record integrity |

**Current status.** Production uses this record shape universally. Foreground
local authoring still appends its semantic managed-local frame and the drain
views that frame as a turn; ingress-local, ingress-foreign, terminal-local,
terminal-foreign and superseded-repair producers append description-only
records to the projection-turn journal. The common own-endpoint executor does
not publish or recover projection receipts: its only recovery authority is the
turn/journal and its only durable completion suppression is the local completion
index. The foreign receiver executor continues to publish and recover the full
receipt protocol, including base bytes and records, attempts, mutation
authority, completion, pending cleanup, and forensic evidence.

**Cold-open order is part of the write-safety contract.** After retained
authority and archive/SQLite reconstruction, startup opens both local journals
before terminal projection can mutate the graph. It drains the semantic
managed-local domain first, then projection turns, recomputes terminal work,
and probes each page for exact current bytes before appending a terminal turn.
A journal that cannot open therefore refuses before terminal graph mutation;
a completed-and-reclaimed terminal turn is not recreated on the next open.

### 3.2b Device-wide absence-decision map and `DeferredAbsence`

Every successful own-endpoint manifested projection passes through the common
executor. Immediately after the graph mutation and exact-identity in-turn
cleanup have completed, that seam stages one local-completion value in the
engine: exact `ProjectionIntentId`,
page id, path, `attempted | completed` state, `Present | Absent` target kind,
and post-frontier. The intent id binds page, path, frontier, precondition and
target; lookup is never by bare path. A completion by page P at X therefore
cannot suppress a later creation by page Q at X.

The engine coalesces staged entries into immutable generation-named objects at
`archive/operations.<generation>/sweeps/local-completion-index-v1/`. A flush installs one
delta, and every `N = max(256, 2 × pages-at-compaction)` deltas the same staged
publication also installs a full-map compaction. Compaction retains every exact
intent still named by an uncheckpointed foreground frame or unretired
projection continuation. An unreferenced local entry may be pruned only when
either a later retained completion at the same `(page, path)` in the local or
receiver half strictly dominates its frontier, or the receiver store retains no
record at all for that key. Thus a local Absent completion that is the merged
map's current answer is never removed while older receiver Present history
remains beneath it; removing it would mis-defer a legitimate recreation.
Superseded objects are pruned under the workspace lease subject to the same
R16-C2 rule.

Open performs one names-only enumeration. A compaction records the count and
set digest of covered delta filenames; a current horizon reads that compaction
plus only newer delta names. Extra names are behind-truth and are read as the
delta. A torn or invalid summary chain rebuilds from valid retained delta
objects, so this disposable cache adds no durable refusal.

The coalesced O-C5 flush points are actor idle, clean shutdown, lease release,
and the earlier of 60 seconds after the first buffered entry or 64 projecting
turns. The first-entry deadline participates in the actor's receive timeout,
including an otherwise quiet graph. Cold-open repair flushes synchronously at
the repair-to-assembly boundary; every error exit after executing a projection
uses an engine-plus-lease scope guard to attempt the same flush before unlock.
`stop_without_clean_drain`, last-handle drop, and a failed guarded flush are
crash-equivalent releases, not clean flush points.

For an own-endpoint `Present` replay whose file is absent, a captured Present
base proves update-shaped work and an exact matching completed intent proves a
previously executed creation. Either case returns `DeferredAbsence`: finished
without mutation, continuation retired, no completion/index entry written, and
the Present terminal head left untouched. Foreground archive publication,
engine admission, SQLite projection, checkpointing, and ordinary startup or
differs-scan reconciliation continue. The current C-4 runtime immediately hands
the deferral to §3.2c's sweep coalescer; the startup/differs scan remains its
crash backstop. Absent-precondition work with no exact matching entry creates
as before.

Foreign replay builds a disposable absence-decision map once per managed open
from the receiver summary plus the local completion index. The receiver summary
is a chain-versioned, disposable object at
`archive/operations.<generation>/sweeps/receiver-absence-summary-v1/`; retained receipt
records remain the truth. Its horizon is the count and set digest of the exact
receiver evidence filenames it covers - completion AND intent names, because a
durable intent without a completion is itself map evidence (incomplete-intent
recovery and the local-index pruning guard consume it), so behind-truth must be
detectable for both namespaces. Every summary install strictly follows the
durable evidence it names. Open performs one names-only readdir of each of the
two evidence namespaces. An equal horizon reads no receipt content; extra
names are behind truth and delta-read exactly those completion/intent records;
a summary naming evidence the directories lack, or any missing/torn/invalid
chain, triggers exactly one full validated-catalog rebuild. Receiver evidence
is immutable, add-only, and never production-deleted, so a valid summary can
only be behind truth. Losing or corrupting it therefore
changes cost only, never an absence decision or refusal outcome. The map is
keyed by `(page, path)`. Its answer is the frontier-maximal completion across
both halves; a defensive incomparable maximal set with mixed target kinds
chooses the reversible Present/defer direction.

The completed receiver half of that map is **not resident**: it is the
point-addressable `receiver-absence-rows-v1` index of §3.2d, read one exact
`(page, path)` at a time and merged with the bounded live overlay by the same
single decision algorithm. What the roots object carries, and what the map holds
in memory, is current work — unfinished receiver intents plus the already-pruned
own-endpoint completion evidence.

Normal Managed opens attach the clean archive store **before** they open the
absence-decision map, on both the activation and the clean-reopen path, so
`archive_store == None` — whose full-validated-catalog fallback
`HotEngine::open_absence_decision_map` retains — is the generic/offline-engine
case, not a normal-session fork. That ordering is pinned by
`w4_p1_storage_contract_pins_receiver_summary_frequency_schema`, which also
keeps the measured table below in step with the probe that produced it.

**Measured open attribution (Harvest W4-P1 item 5, B052).** Ordinary
single-device desktop Managed cycles on a real-scale anonymized graph copy: each
cycle performs the stated accepted saves, a clean shutdown, and one cold
`SyncRuntimeHandle::open_with_progress` whose `SyncRuntimeCleanOpenCounters` are
captured. Reproduce with
`sync_runtime::tests::w4_p1_receiver_summary_reopen_frequency_probe`
(`#[ignore]`, release-only, `TINE_MS_AUDIT_GRAPH_COPY`). Content-read and delta
figures are totals across the cycles.

| field | value |
| --- | --- |
| `checkedHead` | `d1f98c61fe9422ab70b58d38b374604ae499b6da` |
| `corpusFiles` | `1046` |
| `corpusPages` | `1045` |
| `corpusBlocks` | `4758` |
| `cycles` | `20` |
| `savesPerCycle` | `1` |
| `shutdownKind` | `clean-safe` |
| `archiveStoreAttached` | `yes-by-production-call-order` |
| `fullCatalogPass0` | `20` |
| `fullCatalogPass1` | `0` |
| `summaryRebuiltFalse` | `20` |
| `summaryRebuiltTrue` | `0` |
| `receiptContentReads` | `0` |
| `summaryContentReads` | `20` |
| `deltaCompletions` | `0` |
| `deltaIntents` | `0` |

The full-catalog fallback fired in none of the 20 opens and the delta path ran
in all 20, so it is reachable in a normal session rather than dead code. The
measurement is bounded: this corpus carries no foreign receiver evidence, so the
delta path was exercised against an empty evidence set; it says nothing about
delta-read cost under a populated evidence set or about multi-device sessions.
It also attributes no cause, because `open_cache(..).ok().flatten()` collapses
`Ok(None)` with every `Err`, a coverage mismatch reaches the same
`rebuilt`/`full_catalog_passes` values, and the no-archive branch synthesizes
those same values; separating them needs a producer reason counter that does not
exist yet.

The receiver executor consults that answer only after a fresh,
capability-bound reread of the target path and before publishing a new intent:

* maximal Present plus disk absence returns `DeferredAbsence`;
* maximal Absent, or no completion in either half, preserves today's create;
* a present disk file whose bytes mismatch keeps today's conflict flow.

Exact-intent completion suppression remains a fast path only while disk still
matches the recorded target. Disk absence always reaches the map, including a
re-derived creation-shaped intent that finds its old completion.

Retained incomplete receiver intents keep today's phase-driven protocol on the
original precondition: re-authorization, exact recovery, then the existing
evidence-gated fallback. Only an exhausted terminal is remapped, and only by a
second fresh capability-bound reread: disk absence defers; a present byte
mismatch remains the existing conflict. The phases, not an attempt-present
shortcut, decide recovery.

A receiver `DeferredAbsence` is finished without mutation: its continuation
retires, no completion or local-index entry is written, and its Present
terminal head stays untouched. Its provenance is `replay-deferred`. The engine
hands that observation to the packet C-4 coalescer before another outbound
actor turn; a crash before handoff remains covered by the ordinary mandatory
startup/differs scan.

O-C5 accepts two bounded residuals. A crash after own-endpoint execution but
before its coalesced flush can lose at most the 60-second/64-turn suffix, so a
later replay may recreate that user's own just-accepted save; current-state
authorization prevents stale or foreign bytes from using this route. Once a
local Absent completion has executed, the same crash window can lose its index
entry and expose an older retained receiver Present completion. A later foreign
recreation then mis-defers conservatively instead of projecting. The O-C5 cap
bounds this second residual too; it never resurrects content and never silently
loses it, because the downstream sweep retains the recoverable disposition.

### 3.2c Absence sweeps, tiers, and publication hold

An **absence observation** is a startup full-scan difference, a live watcher
difference where accepted state is Present and disk is absent, or a
`replay-deferred` disposition from §3.2b. The first observation durably opens a
sweep. A sweep absorbs another observation only while it arrives less than
`W = 60 seconds` after the previous observation, and closes after 60 seconds
of quiet. All absences discovered by one startup scan form one sweep regardless
of how many bounded scan turns it needs. Membership is the set union by
deletion batch id; `k` is its count at evaluation and the percentage denominator
is the accepted page count captured when the sweep opened.

Tier precedence is exact and accept-by-default:

1. tier 3 when `k >= min(50, ceil(0.10 × pages-at-open))`;
2. otherwise tier 2 when `k >= 4`;
3. otherwise tier 1.

All tiers author ordinary accepted deletion batches immediately; local
acceptance and inbound provider admission never wait for classification. Tier 1
is quiet. Tier 2 and tier 3 become current `SyncAbsenceSweepEvent` snapshots on
the runtime's read-only list surface. Each snapshot carries the tier and timing
summary, explicit disposal, ordered `(page id, path)` members, and the latest
durable action state including Restore cursor or recorded failure cause. An open
sweep escalates in place as `k` crosses a boundary, appending its new tier and
members. A read-only runtime subscription publishes the same snapshot at first
surfacing and whenever its durable action state changes; Tauri relays it to only
the window and binding generation that own that runtime. Disposed surfaced
records remain listable as disposition history.

The frontend keeps tier 1 quiet, raises a tier-2/tier-3 warning, and retains a
dock/list/details surface with the member pages and live action state. Its
Restore, Re-apply, and Keep-deletion controls map one-to-one to the three backend
actions on `SyncRuntimeHandle`; a failed Restore shows its recorded cause and
the re-run control invokes whole-sweep Restore again. Dismissing the warning or
closing the surface changes presentation only. It never invokes a disposition;
Keep-deletion is an explicit deliberate action.

Each logical record is the append-only immutable chain in the layout table.
The current state is its highest valid linked version and records: sweep id;
open, last-observation, and close timestamps; pages-at-open; tier; tier-3 grace
deadline; ordered `(path, page_id)` members; each member's deletion batch id,
predecessor accepted-state frontier, and best-effort prior-Present intent id;
explicit disposal; and versioned action history. Restore progress uses the
already-defined authored-batch list and cursor containing chunk ordinal,
remaining-operation watermark, and monotonic retry count; packet C-5a does not
change this record format. O-C3 remains binding on future re-baselining:
before it retires any batch, predecessor state, intent, or annotation object,
it must pin or copy forward everything required to render every retained
Restore record at its existing fidelity grade. The best-effort intent reference
is provenance and layout evidence, never the restore payload source. Records
are retain-all; there is no record-GC knob.

The `sweeps/` directory is created idempotently only after the workspace lease
is acquired and archive repair has completed. Open enumerates only the positive
`<uuid>.<20 decimal digits>` grammar. It reconstructs records before drain
resumption and before every publication-capable step, ignores unrelated residue,
and falls back from a torn highest object to the preceding valid chain object.
Terminal records are inert. Open or in-grace records resume under their
recorded id, re-establish the barrier, repeat their structured notification,
and re-arm the earliest deadline wake.

The ordering invariant is tier-independent: the record is durable at sweep
open before any member deletion batch commits. `execute_clean_external`
finalizes the batch and absence set, then calls the `SweepRecorder` seam to
append `{batch id, members}`, and only then crosses `commit_clean_prepared`.
A crash between those steps leaves a recorded but uncommitted member. Reopen
reconciles member batch ids against the accepted-batch set, drops that member,
and lets the startup scan re-observe the still-absent file. No accepted deletion
can therefore predate its sweep record.

One named `publication_barrier_active()` predicate is true from sweep open. For
tier 1/2 it ends at close. For tier 3 it extends until exactly five minutes
after close or explicit disposal. It retains but excludes from runnable work
all three history-bearing outbound families: forced batch plus frontier-head
publication; full archive/descriptor/head repair; and prepare-share/baseline
publication (which refuses as named in §3.1). Descriptor-only republication is
the named exemption because it contains no batch, head, or frontier data.
Inbound admission, conflict resolution, local durability, and local acceptance
remain runnable. Both the actor receive timeout and the native watcher sleep
are capped by the earliest close/grace deadline, and both timeout ends force a
deadline turn on an otherwise quiet graph.

This is the approved O-C4 propagation-timing delta: accepted external
deletions are locally durable immediately, but their history-bearing outbound
publication is delayed through the coalescence window and, at tier 3, through
the five-minute grace. Dependency order is unchanged when the retained queue
is released.

Re-apply is an actor-owned disposition action. It compares every member with
current accepted state and re-authors only still-live pages as ordinary
`DeletePage` batches; already tombstoned members are no-ops. It performs no
direct user-file operation. The ordinary guarded Absent projection removes the
files, so watcher observations cannot fight the action. Started/progress/
completion records make it restart-resumable and idempotent. Keep-deletion is
also recorded before it releases grace.

Restore is a whole-sweep actor action. For an ordinary member it point-resolves
the exact deletion batch's manifest-bound semantic effect and reconstructs the
page from complete block, membership and preamble before-images. This reads no
SQLite content and performs no history-namespace scan or unrelated frontier
replay. A legitimate activation-era member with no `deletion_batch_id` retains
the recorded predecessor-frontier reconstruction path; it never guesses the
latest deletion. A retained
Present intent can contribute exact layout and annotation evidence, yielding a
`byte_identical` fidelity grade. Without that evidence the ordinary canonical
renderer yields `semantically_identical`: the accepted semantic state is exact,
but layout may be regenerated. Activation-imported pages with no prior intent
remain restorable. Completion disposes the sweep exactly like Re-apply and
Keep-deletion, releasing any tier-3 grace hold immediately after all restore
batches have authored. Projection remains ordinary manifested publication; a
restored page therefore reappears through the common executor and its completed
Present entry becomes frontier-maximal `(page, path)` evidence for later
absence decisions.

Restore uses `RevivePage`, a semantic operation that changes a catalog entry
from Tombstone to Live at the recorded predecessor name, path, kind, home
shard, and **same page id**. The semantic-effect schema carries a required,
authenticated `revive_page` lifecycle discriminant on that `PageDelta`.
Authoring validation and receiver decoding refuse Tombstone-to-Live without
it. This protects malformed/imported content and honest peer divergence from
silently resurrecting a tombstoned page. `CreatePage` continues to refuse a
tombstoned id. This is one exact 0.7 decoder/schema version—there is no dual
decoder or wire-version peek—and the enrollment graph-schema floor is
unchanged. Already-shipped pre-0.7 builds treat the unsupported clean
descriptor as benign non-join.

`RevivePage` carries the optional exact deletion selector. With one present,
its target is the accepted deletion effect rather than current raw shard keys;
each absent block becomes a source-marked `RestoreSubtree` that creates a new
text container while retaining block/home/UUID identity. Catalog revival stays
first, and the exceptional selector-less case uses only its recorded
predecessor frontier.

The first operation in every revival batch is the catalog flip, making the
following content operations legal in vector order. The remaining operations
are a state-targeted semantic-tree diff from the **current** shard to the
immutable accepted target: block reconstruct/remove/move/edit, membership,
preamble, and metadata changes. An already-equal page emits no content work.
Peers admit and replay these as ordinary CRDT operations, so concurrent remote
edits resolve through the normal merge rather than a special restore channel.

An over-limit restore takes a bounded prefix and commits it as an ordinary
batch, then appends a durable action cursor before planning the next chunk.
If that accepted chunk retains derived projection work, Restore settles that
local continuation before authoring another chunk; the absence publication
barrier remains active throughout and still gates only the history-bearing
outbound families. Every chunk re-diffs current state. Its cursor records `{chunk ordinal,
remaining-operation watermark}`, where the watermark is the size of the full
fresh diff before that chunk. The next recomputed watermark must be strictly
smaller. A non-decreasing value means concurrent admission re-grew the diff;
Restore retries at most three times, then appends a Failed action with the cause
and returns a failed backend action for an explicit re-run. It never records a
partial restore as successful. The final step recomputes and asserts an empty
whole-page diff before appending Completed. Startup automatically resumes
Started or Progress actions from their durable cursor.

### 3.2d Current-action roots, the receipt discovery cursor, and Restore pins

Ordinary open answers "what work is still owed?" from **current-action roots**,
never by enumerating retained history. Two roots objects exist, both under
`archive/operations.<generation>/sweeps/`:

* `receiver-absence-summary-v1/` holds the receiver receipt roots: the exact set
  of durable receiver intents that have no completion, the cursor coverage, and
  the **root of the point-addressable absence history** described below. It does
  not carry completed receiver decisions. Schema 4.
* `receiver-absence-rows-v1/` holds that history: one immutable record per
  `(PageId, exact ManagedPath)` receiver decision, addressed through three
  composed `tine_storage::sealed_accepted_index` authenticated maps — page id,
  then the high and low halves of `portable_path_index::exact_path_digest` of the
  exact path bytes. Page identity and managed-path identity stay separately
  keyed and full width: nothing truncates, and no tuple is hashed into a single
  key. Every record carries and revalidates the identity it is bound to, so a
  substituted record fails rather than answering for another page or path.
* `sweep-action-roots-v1/` holds the complete active sweep roster: every sweep
  that is open, barrier-active, has an unfinished action, or awaits an explicit
  user disposition, each pinned with its exact chain version and digest.
  Terminal chains are counted and otherwise absent. Their records are never
  deleted and stay point-addressable by exact sweep id, which is how Restore
  after completion still reads the original record.

A healthy open enumerates **no** receipt evidence filename and **no** sweep
record filename, decodes no terminal sweep chain, and reconstructs nothing. It
reads one roots object per producer (plus its chain predecessor for the digest
check) and resolves only the work the discovery cursor still points at.

**Completed absence history is on disk, not in memory.** Neither the ordinary
open nor an ordinary receipt update serializes or materializes the historical
decisions. One absence answer is one point read of that key's record merged with
the bounded live overlay, and it is not cached afterwards, so repeated point
loads cannot accumulate. Exactly one decision algorithm runs, over the historical
record and the live overlay alike. Persistent bytes may grow with history — D-5
grants that, and rebaselining is the terminal bound — while resident state tracks
current and pending work only: unfinished receiver intents, the already-pruned
device-local completion evidence, and rows whose durable write-through failed.
Committing a receiver completion retires that intent from the resident set, and
flushing the own-endpoint completion chain re-derives the resident own half from
whatever the chain's prune kept, so a long activation converges on the same
bounded state the next open would seed. Publishing the record and the map nodes
precedes publishing the roots object that names the new root, so a crash leaves
unreferenced objects rather than a root pointing at bytes that were never
written. The receiver receipt publication's pinned barrier budget is 31, of
which 4 are the discovery cursor's and 2 are the separate row-namespace
publication that a multi-namespace `ObjectStore::publish_coalesced_private_derived`
would remove.

**A missing record is damage, never permission.** The authenticated map root is
the membership authority, so a record the root names but disk cannot supply is
detectable damage. It is refused by name, never answered `Create` — that would
recreate a file the receiver deleted — and it retires the derived roots so the
next open runs the named counted repair from retained receipts. Every point-read
caller routes damage the same way, including the own-endpoint completion prune,
so a row discovered missing while pruning cannot become an endless reopen
refusal. Once an activation has proven the index damaged, it publishes nothing
further over it: later receipts in that activation would otherwise restore an
apparently healthy roots chain and cancel the repair the damage requires. Those
receipts stay durable truth in the receipt store and are folded by the repair. Every
counter is reported on the clean-open trace: `receipt_evidence_names`,
`receipt_cursor_marks`, `sweep_record_names`, `sweep_chain_objects_read`,
`sweep_roots_repaired`, `sweep_retired_chains`,
`current_action_receipt_obligations`, `current_action_sweep_pins` and
`current_action_retained_documents`.

**The discovery cursor.** A receipt is truth; the roots are a disposable cache
of it, so the window between publishing a receipt and folding it into the roots
needs its own durable record. `sweeps/current-action-cursor-v1/` holds one
**head** object — a random `incarnation` plus the monotone `reserved` sequence —
and one **mark** per receipt publication, named by that sequence. The receipt
store reserves the mark *before* it publishes an intent or a completion; the
mark and the advanced head are staged into one coalesced audited publication,
so a reservation costs the same barriers as the mark alone. A reservation that
cannot be made durable repudiates the head rather than letting a later open
prove coverage it does not have.

**Coverage is durable state, not an observation.** Each roots object records
the exact `{incarnation, covered_through}` it has folded, published by the same
write that publishes the state it claims to cover. An open trusts the roots
only when the cursor's incarnation matches, no reservation between
`covered_through` and the durable head is missing, and every mark binds its own
name. Recreating a lost cursor directory mints a new incarnation, so it can
never be mistaken for "everything was already covered", and that binding
survives a second crash inside the repair window — an in-memory
"directory was created" flag does not. A lost individual mark is a detected
sequence gap; a torn mark fails its name/content binding.

Any of those conditions takes one **named, counted repair**: a single validated
pass over retained receipt truth that rebuilds the roots, records the new
coverage, and returns the next open to the bounded path. It is never a refusal
(D-3, I-10), never disguised as ordinary work, and never permanent.

**Occupancy is not damage.** There is no cap on outstanding work. A legitimately
large uncovered window is streamed in chunks of at most 256 reservations, with
the roots — and the coverage watermark inside them — installed durably after
each chunk, so a crash mid-catch-up resumes at the last installed chunk instead
of restarting or falling back to history. Marks are reclaimed only at the
minimum watermark over every *registered* consumer, so one producer can never
clear the only discovery mark for another that has not caught up.

**Restore pins.** `CurrentActionRoots::retention_closure()` is the bounded
interface a generation capture reads: the documents, dependency heads, pages,
intents, exact deletion batch ids and sweeps that unfinished actions and
explicit pending Restore still require. The predecessor frontier and prior
Present intent remain fidelity/layout evidence; the deletion batch's semantic
effect is ordinary Restore text authority. Membership means "cold relocation
must keep this logical object
reachable", not "keep this record active". Completing a sweep removes it from
current actionable state; it does not erase its Restore predecessor page
identity, its predecessor `FrontierV2`, or its prior intent, all of which the
original record still carries verbatim and `begin_restore` reads back by exact
sweep id. Historical restoreability alone never keeps a record active.

### 3.2e Persistent CRDT writer lanes

Every CRDT operation a device publishes is authored by a *writer lane*, not by a
fresh per-batch peer. One device holds exactly two lanes per admitted endpoint:
`Local` (every ordinary mutation and maintenance seal) and `External` (external
editor reconciliation). Lanes are reused across transactions, imports, seals and
ordinary restarts, which is what keeps a repeatedly edited document's version
vector — and every later manifest's before-vector, and every shallow snapshot —
O(writers) instead of O(edits). `ImportId` keeps its batch, session and
observation roles; only the CRDT peer stops being per-import.

Alongside those two Loro peers the same record saves a third identity: one
**causal writer incarnation** (`WriterIncarnationId`, a UUID). It is the
`CausalPeerKey` of `CoreDurableBatchContract`, so a manifest's `BatchCausalDot`
names an incarnation, never the enrolled `DeviceId`. Device, endpoint and
enrolment identity are untouched by any of this: `OperationBatch::author_device_id`
still carries the one enrolled device, and one device may own several
incarnations over its lifetime. Every batch of one incarnation — ordinary,
local-journal, external and seal alike — shares that incarnation's single
sequential counter chain; the two Loro roles stay distinct within it.

**Allocation.** Every one of the three identities is allocated at random and
then saved, never derived. A derived identity would let a rebuild that has lost
the record recreate the very lane — or the very causal chain — whose published
prefix it can no longer qualify. Production never hashes a `DeviceId` into an
incarnation and never allocates one per batch, session or import; only tests may
pin explicit stable fixture incarnations. The two deterministic lazy-genesis
peers and the zero peer are never allocated.

**Where the record lives.** One postcard record of at most 1 KiB per device, in
the device-private application runtime root under `crdt-writer-lanes/lanes-<workspace>-<lineage>/writer-lanes-<device>.postcard`.
It is app-data keyed by graph: it must not travel with the graph and it is never
in the graph directory or in `.tine-sync` (D-11). It is not a receiver-visible
proof of anything.

**Exclusion.** No archive lease covers this namespace — two honest concurrent
graph copies of one workspace hold two independent archive-rooted
`WorkspaceRuntimeLease`s and would both reach the same record. Exclusion is
therefore taken here, with the existing platform lease helper on
`writer-lanes-<device>.lock`, and re-proved (held handle identity versus the
currently named file) before every peer vend and every reservation. Exact-byte
replacement is a torn-write guard, never a cross-process compare-and-swap.

**Prefix durability.** The record carries a monotone `confirmed_own_counter` —
the highest `BatchCausalDot` counter proved durable **for the saved
incarnation**, qualified by that `CausalPeerId` and not by the device — plus at
most one `reservation` bound to the exact `(BatchId, manifest digest)` of the
batch about to become outwardly visible. The reservation is published before the
trusted local journal append and before the external archive manifest commit, so
it covers both commit paths with one barrier. It is idempotent for the same
batch. Cost: one `replace_exact` of a fixed-size record per publishing turn —
one temp-file `sync_all` plus one directory fsync — and nothing at all on a
retry of an already reserved batch.

**Continuation and rotation.** On open, authoritative accepted history plus the
drained local journal supply the covered own counter **for the saved
incarnation**; the accepted path and the trusted local-journal path each keep
their own durability question. At most one dot can be ambiguous, and it is
resolved by asking the archive about the exact reserved manifest fingerprint.
Provably absent frees the dot. Anything else — durable but unrestored,
unanswerable, a different manifest under the reserved id, or more than one
unaccounted dot — spends it.

A covered record continues its saved lanes and its saved incarnation. An
uncovered one is **retired whole**: a fresh incarnation and two fresh Loro peers
are saved (incarnation ordinal + 1, `confirmed_own_counter` back to zero) before
any new authoring. Recovery is a new causal identity, never a reused counter, so
the unprovable older prefix keeps its own identity forever and the replacement
chain may freely restart over counters the retired chain already spent. That is
what makes it safe to author immediately: there is no retained floor, no refused
publication, no permanent wait for a missing offline device, and no dropped
pending work — pending original bytes still drain and replay through the one
existing path. The cost of a rotation is exactly one extra causal peer `P`, paid
only for a real writer incarnation. A missing record mints a fresh incarnation
on the same terms; an undecodable one is preserved under
`<record>.superseded-<digest>` and rebuilt (D-1/D-3). Repeating the reopen over
unchanged unprovable state mints one incarnation per open, not one per batch.
Reading, repair and reopen are never blocked by any of this (I-10).

**Ownership admission (receiver side).** A private lane record is not receiver
authority. Each admitted batch's CRDT payloads are read for the peers they
advance — `validate_update_base` has already proved each carried range starts at
the base document's counter for that peer and ends strictly above it, so the
update's partial end vector *is* the set of lanes it advances. Those peers are
bound to `(author device, origin role)` on first admitted use, atomically with
the batch that used it, and enforced on every later use, on both the accepted
path and the trusted local-journal prefix, before any live document is touched.
A batch claiming another device's lane, or the same device's other role, is
refused whole (`EngineError::CrdtLaneNotOwned`) with its original bytes intact
in the archive. Bootstrap import predates every lane: it neither claims nor binds
one.

Incoming ordinary and external batches may not advance the zero peer or either
immutable lazy-genesis peer. Receiver admission uses the same reserved-peer set
as private writer allocation and record validation, and refuses the whole batch
before installing any document or ownership binding. Rejected originals remain
available through the archive.

The causal writer incarnation is admitted by the same machinery, in the same
place, before any live fragment, ownership or effect changes. On first admitted
use the batch's `CausalPeerId` is bound to its enrolled author device; a later
batch of a *different* author claiming that incarnation is refused whole
(`EngineError::CausalPeerNotOwned`). Ownership alone is not enough, because a
sparse accepted clock covers counters without naming exact `BatchId`s: a
gap-free accepted tip for that incarnation additionally refuses any batch whose
claimed counter is at or below the tip unless it is the known original for that
dot (`EngineError::CausalDotFork`). Ordinary duplicate replay of the original
bytes therefore still succeeds, while conflicting bytes or a conflicting
`BatchId` on an already-spent dot are rejected with the original accepted
fragment untouched and the rejected bytes retained. Both maps are bounded by
writer incarnations, are carried in the clean checkpoint state section
(schema 3), and are rebuilt identically by full accepted replay — one tree, no
second index.

**Schema.** This is one coherent current format with no reader for the previous
one (D-1): `OPERATION_SCHEMA_VERSION` 11,
`SEMANTIC_EFFECT_SCHEMA_VERSION` 9, lazy-genesis schema 7, writer-lane record
schema 2, and clean checkpoint state schema 4. Every persisted point-index key encoding carries the
full 16-byte incarnation UUID; none of them truncates or hashes it.

### 2.10a Durability barriers by artifact class

Platform durability policy is stated **per artifact class**, never globally.
`crate::filesystem_durability::DurabilityArtifactClass` names the two classes,
and every projection directory barrier passes through it:

| Class | What it covers | Policy |
| --- | --- | --- |
| `PrivateDurableAuthority` | The oplog manifest, object archive, local journal and promoted/operational receipt store below app-private storage — and graph-tree artifacts the graph is the **sole** authority for: conflict copies, trash, withdrawn bytes, assets. | Strict on **every** platform, Android included. A barrier the filesystem refuses is a real durability failure. The empty receipt-store initialization exception is confined to §2.10c. |
| `SharedReconstructibleProjection` | The Markdown/Org projection of an already-accepted manifest into the user's graph tree. | Strict everywhere except Android. On Android only, and only for `PermissionDenied`/`Unsupported`/`InvalidInput` (`EPERM`/`ENOTSUP`/`EINVAL`), the barrier **degrades**. Every other errno stays fatal. |

The crash story for the degraded case still holds: the projection is derived
state. The accepted manifest in app-private storage — which keeps its strict
barriers — already records those bytes, and a crash that loses an unflushed
directory entry is repaired by the projection drain on the next open, by the
same mechanism that finishes an interrupted projection. Retrying a capability
refusal cannot ever succeed, so retrying it forever is not crash-safety; it is
an availability bug that strands the user's edit, which is exactly what Android
CI run 32088229039 recorded (`phase:ProjectionDrain`,
`detail:Invalid argument (os error 22)`, `settled:false`, 64 turns).

Because the device is the only oracle for these semantics, every platform
primitive on the projection leg — the directory flush, `renameat2` with
`RENAME_NOREPLACE`, projection file `fsync`, and the no-follow `openat` of a
projection parent or file — names its operation and its location in the error it
returns, and `execute_manifested_projection_work` prefixes the page path. A
device receipt therefore reads `projecting "pages/X.md": fsync of the projection
parent directory failed at chain depth 2/2 (…): Invalid argument (os error 22)`
rather than a bare errno. `ErrorKind` is preserved, because guarded-conflict
classification and the durability policy both match on it.

The class split is enforced by tests, not by this table:
`filesystem_durability::tests::only_the_reconstructible_projection_class_degrades_and_only_on_android`
and `model::tests::only_the_reconstructible_projection_barrier_degrades_on_android`
at the primitive, and
`sync_runtime::tests::clean_runtime_save_survives_an_android_projection_directory_barrier_refusal`
plus `…::a_projection_directory_barrier_refusal_stays_pending_off_android`
at the save boundary.

### 2.10a-i Durability barriers and the batch commit point

A **durability barrier** is a syscall that forces bytes to stable storage:
`fsync` of a file, `fsync` of a directory, or `syncfs` of a filesystem. Each one
is a device round trip. Their *count per accepted operation* — not the time any
single phase reports — is what turns an ordinary edit from milliseconds on a
local SSD into hundreds of milliseconds on a slow or network filesystem, and it
is invisible to phase timers because the multiplicity is spread across modules.

**Invariant — acceptance authority precedes relaxed archive publication.** The
managed-local journal frame is durable before archive materialization begins and
is checkpointed only after publication completes. While that exact frame remains
undrained, its canonical object bytes may authorize archive-object recovery. No
other caller receives this relaxation: ordinary archive publishers take a strict
pre-install data barrier, and the batch manifest always remains a strictly
flushed commit marker.

**The protocol** (`oplog/object_store.rs::ArchiveBatchPublication`):

1. **Stage.** Each artifact is written under a temporary name in its own
   namespace, with no barrier. Temporary names are not archive entries: every
   reader — `ObjectStore::validate_namespace`, `inspect_batch`, every replay —
   addresses artifacts by content-addressed or batch-addressed *final* names.
2. **Journal-covered object install.** Only the managed-local drain inserts its
   object final names at this point. Its exact frame is still undrained, so a
   crash-torn object remains repairable. Strict callers skip this step.
3. **One data barrier.** `syncfs` flushes the whole staged set. Strict callers
   take it before any final name is visible; the managed-local drain takes it
   after object installation while the manifest is still temporary.
4. **Remaining installs.** Ordinary publishers insert every final name. The
   managed-local drain inserts only its now-durable manifest commit marker.
5. **Directory barriers.** One `fsync` per distinct namespace touched — two for
   an ordinary batch (`objects`, `batches`).

The strict path's barrier-before-install ordering remains unchanged. The
journal-covered path deliberately admits one additional crash state only
between steps 2 and 3: an object final name whose bytes were not fully flushed.
Cold open first authenticates the archive structure, then—under the workspace's
sole-writer lease—decodes only uncheckpointed managed-local frames and builds an
exact digest-to-canonical-byte repair set. It replaces a mismatching object only
when that set names one unambiguous exact replacement, rereads the result, and
then runs the ordinary full namespace validation. An uncovered mismatch,
ambiguous coverage, a noncanonical replacement, a workspace mismatch, or a torn
manifest still refuses activation. Drained history is never recovery authority
because step 3 makes every object durable before publication returns and before
the caller may advance the checkpoint.

`tine_storage::ExactImmutablePublicationBatch` now implements the strict form in
the shared storage primitive: stage, flush all staged data, install no-replace,
then flush every destination directory. Tine's local drain retains its narrower
archive publisher because only it can apply the undrained-journal authorization
and manifest carve-out.

**Ordering between artifact classes.** The archive's **lineage claim** remains
durable before every manifest that asserts it. The **local journal frame** is
already durable before the drain starts. Objects install before the strictly
flushed **manifest commit marker**, and the journal checkpoint advances only
after both namespace directory barriers complete. Thus a durable manifest never
points to an object lacking either durable archive bytes or exact undrained
journal recovery authority.

**Crash points.**

| Crash at | On disk | Recovery |
| --- | --- | --- |
| During staging | Temporary names only, contents arbitrary | Batch not accepted. The journal frame is undrained, so the drain republishes the byte-identical batch. Temporaries are invisible to readers. |
| After staging, before an install | As above | As above. |
| During journal-covered object installs, before the barrier | A prefix of object names whose bytes may be torn; no manifest name. | Cold open repairs only exact undrained-journal-covered object mismatches before full validation. |
| After the barrier, during remaining installs | Every surviving final name has durable, byte-correct content. | The drain republishes and verifies exact existing names. |
| After installs, before a directory barrier | Some name insertions may disappear; every surviving name has durable, byte-correct content. | The drain republishes; the journal checkpoint has not advanced. |
| After the directory barriers | The whole batch durable | The drain proceeds to checkpoint. |

In every row the *accepted operation* is unaffected: it became durable in the
local journal during the foreground save, before any of this runs.

**In-scope scenarios defended:** crash/power loss, torn write, disk error,
interrupted delivery, honest concurrent instance (the archive is a single-writer
namespace held under lease), honest multi-device divergence (unchanged — this
publication is device-local). **Out of scope:** an adversary who can write
arbitrary bytes to the user's filesystem
(`specs/notes/2026-08-07-trust-model-and-threat-model-decision.md`).

**Platforms without `syncfs`** (Windows, macOS) publish each artifact through
the ordinary durable publisher, so this optimization and repair state do not
arise there. On Android, a strict caller whose vendor filesystem denies the
filesystem-wide flush as a *capability* (`EPERM`/`ENOTSUP`/`EINVAL`) falls back
to one `fsync` per staged artifact. Every other errno stays fatal. The
journal-covered Android path installs object names while its journal frame is
undrained, then uses the same whole-batch flush and exact cold-open repair
authorization as Linux before it installs the manifest.

**Read paths take no barriers.** `fsync` before reading a file defends nothing:
a read is served from the same page cache the writer wrote into, so forcing
those bytes to the platter cannot change the returned bytes, and it cannot
detect corruption either. Three such helpers existed on the managed projection
paths and fired three times per save and eight times per cross-page move; they
are deleted, and no read path may reintroduce one. See the `MS-REF-` note below.

**Invariant — managed projection takes one directory barrier per turn per leaf
directory.** A projection turn defers reconstructible graph-directory barriers
until its final name change, deduplicates them by opened leaf-directory
identity, and flushes each distinct leaf exactly once before checkpoint. File
barriers remain one per distinct staged inode. Strict private-authority and
conflict-trash barriers are not coalesced into this reconstructible point.

This collapse is managed-class-conditional. Direct Files does not execute a
projection turn and its publication barriers are unchanged; the same
`sync_projection_chain_with_class` primitive still flushes `chain.last()` and
nothing above it immediately. The leaf-only argument has two halves and they
are exhaustive:

* An ancestor **Tine created during this operation** is made durable when it is
  created, not afterwards: `model::create_projection_chain_component` is the
  only place a chain component is created, and it flushes the parent that now
  holds the new name before the chain builder descends into it. A crash between
  the `mkdir` and the operation's own barrier therefore cannot lose the path the
  operation is about to publish into
  (`model::tests::projection_retry_resumes_after_synced_partial_parent_chain`
  drives exactly that crash point and proves the retry converges).
* An ancestor **Tine did not create in this operation** already had a durable
  entry in *its* parent before the operation began. No in-scope scenario
  un-durables an entry that is already on stable storage: crash/power loss,
  torn write, disk error, sync-service delivery, external-editor race, honest
  concurrent instance, honest multi-device divergence and malformed imported
  content can all destroy or replace such a directory, but none of them can be
  repaired by this process re-issuing `fsync` on it, and every one of them is
  already handled by the guarded-conflict, no-follow and recovery machinery
  above. The removed flushes are removed because **no in-scope scenario needs
  them**, not because they were expensive — the refusal-scenario rule in
  `AGENTS.md` §5 cuts both ways, and a barrier with no scenario is latency the
  user pays for nothing.

The one failure the removed flushes did cover is **another** process creating an
ancestor directory and not flushing it itself — a durability obligation that
belongs to that writer, that Tine cannot discharge on every subsequent write
without paying the barrier forever, and that Linux's ordered metadata journals
largely subsume anyway (a directory `fsync` commits the transaction that created
its parent). It is recorded here rather than defended.

Enforced by
`model::tests::a_projection_operation_flushes_one_directory_whatever_its_depth`
(chain depth must not change the barrier count) and
`model::tests::a_created_projection_ancestor_costs_exactly_one_extra_barrier`
(each created ancestor still costs its own barrier, exactly once), plus
`model::tests::a_same_directory_move_takes_one_directory_barrier` and
`model::tests::managed_barrier_collapse_does_not_change_direct_files_retire_publish_barriers` for the
turn boundary and the class guard.

**The budget, and where it stands.** `crate::durability_counters` counts every
barrier `tine-core` initiates and
`sync_runtime::tests::managed_save_and_move_stay_within_their_barrier_budget`
asserts the per-operation totals against
`MANAGED_SAVE_BARRIER_BUDGET` = **10** and `MANAGED_MOVE_BARRIER_BUDGET` =
**13**. Those are *core-initiated* barriers: `tine-storage`'s own local-journal
appends and SQLite file-set publication are not reachable from this crate and
are excluded (measured at three more per save, four per move).

The packet-2b collapse reduced the complete packet-2b-pre ledgers from 35 to 29
for a save and from 112 to 87 for a cross-page move. Packet C-2 adds exactly
one coalesced local-completion chain install when this fixture reaches idle:
one directory barrier plus one staged-publication filesystem barrier for the
whole operation, never per page. Packet 2c removes own-endpoint receipt
publication from that executor. MS-05 keeps the exact totals and barrier kinds;
it moves the archive filesystem flush after the journal-covered object installs
but before manifest install and journal checkpoint. The final exact attribution
is: save foreground
`file_fsync=1 dir_fsync=2 syncfs=0 total=3`, save total
`file_fsync=2 dir_fsync=6 syncfs=2 total=10`; move foreground is zero and move
total is `file_fsync=6 dir_fsync=5 syncfs=2 total=13`. These are exact-equality
assertions, not ceilings: either upward or downward drift requires a new
attribution before the pin moves. The MS-05 barrier delta is therefore zero:
the packet changes crash-safe ordering and recovery, not the number or kind of
durability syscalls. The packet-2b removed 6/25 directory barriers were repeated
`SharedReconstructibleProjection` barriers within turns; no strict authority or
quarantine barrier was removed.

The 2026-08-27 counter-completeness sweep raised those enforced numbers from
25/74 without adding or moving a single durability syscall. This is measurement
visibility, not a latency regression: the save fixture made five previously
unattributed own-endpoint projection-receipt publications visible (five file +
five directory barriers); the move fixture made sixteen such publication
pairs, five mutation-authority replacement file barriers, and one clean-
foreground retirement directory barrier visible. Packet 2c removes those own-
endpoint barriers; the retained foreign receiver protocol remains outside the
save/move fixtures. The exact ledgers and the source guard reject unreviewed
barrier drift or any future raw barrier outside the counted wrappers.

Packet C-4 adds no save- or move-path barrier. Sweep-chain installs occur only
on the absence/observation path and use the ordinary staged/no-replace archive
publication discipline. Each appended sweep version pays one staged-object
file flush, its staged-publication filesystem barrier, and one coalesced
`sweeps/` directory barrier. A sweep normally appends at open/membership,
escalation or additional membership, close, and action progress; the cost is
per sweep transition, never per ordinary save. The exact save/move pins above
therefore remain 10/13 with foreground 3/0.

**Barriers with no in-scope scenario are deleted, not budgeted.** Three
mechanisms left the count on 2026-08-27 (save 28 → 25, move 77 → 74), each
because no in-scope failure needed it, not because it was expensive:

* The **post-publication re-`fsync` of the committed journal target** (one per
  save). The published inode is the staged inode, already flushed before the
  no-replace rename; re-syncing it proves nothing the staging barrier did not.
  The reread and identity refusals around it stay — they are preconditions.
* The **pending-cleanup round flip on an empty queue** (two directory barriers
  per save; one per projected page on a move, when that page's queue is empty).
  `ProjectionReceiptStore::pending_projection_cleanup_bounded` flipped the round
  state durably whenever the *active* round was empty. When the *whole* queue is
  empty — the ordinary save — the flip makes nothing reachable. It is now elided
  when both rounds are empty and unchanged otherwise, including the case that
  motivates it: new markers are appended to the inactive round, so an empty
  active round with a retained inactive round still flips.
* The **displaced-file pre-`fsync`** (one per projected page displaced; two per
  cross-page move, none on an ordinary save). The file was about to be moved
  aside and its exact pre-image is already durable in the recovery record by the
  ancestor-chain argument above. The identity capture, the
  `displaced != expected_base` refusal, the bound evidence capture and the
  retirement all stay.

Enforced by `sync_runtime::tests::managed_save_and_move_stay_within_their_barrier_budget`
and `oplog::projection_store::tests::an_empty_pending_cleanup_queue_elides_the_durable_round_flip`.

The cost-model audit's target is 3 and 5. The remaining gap is stated here
rather than hidden:

* The **foreign receiver projection receipt store** publishes five artifacts
  per intent (base, intent, attempt reservation, mutation authority, completion)
  = 10 barriers per received page, plus one forensic-evidence record and one
  pending-cleanup marker for each recovery file a projection displaces. It
  published *nine*
  before the 2026-08-26 refusal census cut the four per-intent namespace
  bindings; the survivors each name an in-scope crash/torn-write scenario and
  are recorded in the census
  (`specs/notes/2026-08-26-p-census-receipt.md`). They are still published one
  at a time: each is separated from the next by a read-back of the artifact just
  published, so staging them behind one barrier would have to carry the staged
  bytes in memory as well. This cost belongs to foreign ingress, not the local
  save/move ledgers; packet C-3 consumes the receiver history without weakening
  this protocol.

The remaining save/move gap is no longer receipt publication or repeated
managed graph directory barriers; it is the turn/journal, graph publication,
archive, and coalesced completion-index work shown by the exact 10/13 ledgers.
Direct Files' user-visible Markdown publication keeps its
temp + fsync + exact durable name publication + base-revision guard + lock.
The platform-specific typed publication boundary supplies the name-durability
guarantee, including write-through publication on Windows.

### 2.10b No-clobber publication when the filesystem has no rename flags

The directory barrier is not the only primitive Android shared storage refuses.
Android CI run 32091898520 recorded the **flagged rename itself** failing:

```
retained_publication=… phase:ProjectionDrain settled:false turns:2
detail:projecting "pages/Smoke.md": renameat2(RENAME_NOREPLACE) publishing the
projection failed at "Smoke.md" -> ".Smoke.md.49a4ed18…"
```

with `Invalid argument (os error 22)` underneath. Two earlier lanes eliminated
this call by reading AOSP `FuseDaemon.cpp`, whose `do_rename` accepts exactly
that flag. The device disagreed. `RENAME_NOREPLACE` has to be provided by every
layer — the kernel FUSE client, the daemon, and the filesystem underneath it —
and on this path it is not. **Upstream source is evidence about upstream intent,
not proof about the running device; the receipt wins.** The same `EINVAL` is
reachable off Android on any filesystem without `rename2` flags (FAT/exFAT
removable media, some FUSE and network mounts).

`model::rename_projection_noreplace_with_class` therefore applies a capability
policy to the no-clobber publication, keyed on the same
`DurabilityArtifactClass`:

| Class | Policy for the flagged rename |
| --- | --- |
| `PrivateDurableAuthority` | The platform primitive and nothing else, on every platform. There is no second copy to rebuild these bytes from, so a non-atomic publication could leave a reserved-but-empty file at a live graph name after a crash. A filesystem that cannot provide the primitive fails the write. |
| `SharedReconstructibleProjection` | `EINVAL`, `ENOSYS` and `EOPNOTSUPP`/`ENOTSUP` from the flagged rename — and **only** those three, matched on the raw `errno`, not on `ErrorKind` — are read as "this filesystem does not implement that flag" and retried through the reservation fallback below. Every other errno (`EIO`, `ENOSPC`, `EACCES`, `EXDEV`, `EEXIST`, `ENOENT`) describes the operation rather than the flag and stays fatal. |

Unlike the directory barrier in §2.10a, this policy is **not gated on Android**.
The barrier policy gives a guarantee up, so it is confined to the platform that
forces the choice; this one keeps its guarantee and gives up only atomicity, and
failing the write closed on a FAT stick would be an availability bug with no
in-scope threat behind it.

**The fallback (`model::reserve_and_rename_projection`).** Reserve the
destination name with an exclusive create (`O_CREAT|O_EXCL`), then perform a
plain `rename` onto the reservation.

* *What it keeps.* Never silently destroy a file already at the destination —
  the one guarantee `RENAME_NOREPLACE` was there to provide. An occupied
  destination fails the reservation before anything has moved, and is reported
  as `AlreadyExists`, the exact error the flagged rename raises, so every
  guarded-conflict caller above is unchanged. A failed reservation is fatal, never
  a silent overwrite. If the plain rename then fails, the reservation is rolled
  back — but only when the destination is still, by physical identity, the
  placeholder that call created — so a failed publication leaves no zero-length
  file at a live page name.
* *What it gives up.* Atomicity of the name transition. Inside the window
  between the reservation and the rename the destination exists as a zero-length
  file, so (a) a crash there leaves a zero-length name, which the projection
  drain rebuilds from the accepted manifest on the next open exactly as it
  rebuilds any interrupted projection, and (b) an external writer that replaces
  the placeholder inside that window is overwritten rather than winning the race.
  Both are why the fallback is confined to the reconstructible class.

The answer is a property of the mounted filesystem, so it is remembered per
`st_dev` after the first capability refusal instead of costing a failed syscall
on every publication. The memo is consulted only for the reconstructible class
and is never load-bearing: an unknown device simply attempts the flagged rename
and learns from it.

The reconstructible-projection call sites are the ones bracketed by
`preflight_reconstructible_projection_chain` /
`sync_reconstructible_projection_chain`: retiring a live page to its attempt
recovery name, publishing the staged bytes onto the live name, withdrawing an
unsafe publication, restoring a displaced target, retiring a recovery artifact
into quarantine, and preserving a changed recovery artifact as a projection
conflict. The graph-tree write paths that are *not* on that leg —
`managed_atomic_create_with_proof`, `managed_atomic_write_with_conflict`, and
`managed_move_noreplace` — keep the strict class, because in Direct Files the
graph tree is the sole authority for those bytes. `managed_atomic_replace_bound`
is explicitly classified by its caller: Direct Files remains strict, while an
already-journaled managed projection uses the reconstructible fallback for all
three replacement transitions and their directory barriers.

Enforced by `model::tests::only_the_reconstructible_projection_rename_falls_back_when_the_flag_is_unsupported`,
`…::the_projection_rename_fallback_refuses_an_occupied_destination_rather_than_clobbering_it`,
`…::a_projection_rename_fallback_that_cannot_complete_leaves_no_empty_destination`,
and at the save boundary by
`sync_runtime::tests::clean_runtime_save_survives_a_projection_rename_capability_refusal`
plus `…::a_non_capability_errno_from_the_projection_rename_stays_pending`.

### 2.10c The shared-provider tree without rename flags, including the exchange

§2.10b left the shared-provider transport (`oplog/wire.rs`) alone as a
follow-up. Android CI run 32094662514 turned that into the next failure: the
managed save landed for the first time and the journey stopped one step later at

```
AssertionError: prepare shared failed: sync actor refused request:
scenario filesystem operation failed: Invalid argument (os error 22)
```

The flagged renames in that module operate under `<graph>/.tine-sync/v2/shared`,
and one of them — quarantining a publication's own abandoned staging entry — is
on the **happy path of every provider `Put`**. On a filesystem without `rename2`
flags, no object can be published at all, so share preparation cannot start.

The six shared call sites are classified individually. The two remaining flagged
renames in the same module belong to the **private** retry journal
(`ProviderRetryJournal`, outside the graph) and keep the strict class untouched.

| Site | Artifact | Class | Policy |
| --- | --- | --- | --- |
| `quarantine_unowned_staging` → `removed/orphan-<op>-<gen>` | An abandoned staging copy of bytes whose authority is the private retry-journal blob; the caller deletes this diagnostic again as soon as its identity matches. | Reconstructible | Reservation fallback (§2.10b). |
| `quarantine_provider_name` → `removed/<prefix>-<digest>` | FOREIGN bytes that took a name this device expected to own, preserved for forensics before the operation refuses. | Reconstructible — *only because the fallback keeps the no-clobber guarantee*: an occupied destination fails the exclusive reservation before anything moves, and a rename that then fails leaves the foreign file exactly where it was. | Reservation fallback. |
| `preserve_retirement_race` → `rename-evidence/retirement-race-<digest>` | The same, for a name re-created during a retirement. | Reconstructible, same argument. | Reservation fallback. |
| `reconcile_provider_retirement`, the `RENAME_EXCHANGE` | The validated original moving to its diagnostic name, swapping with this operation's journaled placeholder. | Reconstructible; see below. | Single placeholder-consuming rename. |
| `reconcile_provider_retirement`, the rollback exchange | Undoing the above when post-validation fails. | — | Attempted **only** while this device still holds the placeholder at the source name, i.e. only on the exchange path. After the fallback there is nothing to swap back and the refusal says so rather than pretending. |
| `reconcile_provider_retirement` → `rename-evidence/retire-placeholder-<op>` | The displaced zero-length placeholder moving to private evidence. | Sole authority of the exchange invariant | **Strict, no fallback.** This step exists only on the exchange path, and a filesystem without `RENAME_NOREPLACE` has no `RENAME_EXCHANGE` either, so the exchange fallback has already made it unreachable there. A filesystem that somehow provided one and not the other gets an honest named refusal rather than a two-step substitute whose crash window recovery cannot read. |

**The exchange decision.** An atomic swap has no no-clobber-shaped substitute, so
the reservation fallback does not apply. A three-step rename through a scratch
name was rejected: it introduces a window in which the retired bytes exist at
neither name, and a second window whose leftover the recovery path cannot tell
from a racing delivery.

What retirement actually needs is narrower than a swap. Before the exchange the
diagnostic name is already occupied by a **zero-length placeholder this operation
created**, whose physical identity was made durable in the private journal
(`staging_identity`) before anything moved. So the fallback is a **single plain
`rename(2)` of the validated original onto that placeholder** — atomic on every
POSIX filesystem, no scratch name, no third step. Its end state is exactly the
state the exchange path reaches one step later: source name gone, original at the
diagnostic name, placeholder inode unlinked.

*Crash windows.* There are exactly two, because it is one rename:

* **Before it.** The original is still at its name and the placeholder still
  holds the diagnostic name — byte-for-byte the state the reconciler starts from.
  Recovery re-enters the same branch and retries. No residue.
* **After it.** The source name is free and the diagnostic name holds the
  original. Recovery takes the "source absent" branch, validates the retired copy
  against the recorded identity, digest and length, and completes. If the rename
  was not yet durable in the parent directory the state is the first window
  again, which also converges.

There is no third window: `rename(2)` over an existing destination is atomic, so
the retired bytes are never absent from both names.

*What it gives up.* The exchange's **other** guarantee — that the source name is
never free. After the fallback the source name is free, so an honest concurrent
instance or a sync-service delivery can re-create it, and the placeholder is no
longer available as proof that the transition happened. Recovery therefore keys
on the diagnostic name holding the recorded original identity, digest and length;
anything found at the source name afterwards is treated as a racing replacement,
preserved as `rename-evidence/retirement-race-…`, and the operation refuses —
the same terminal shape the exchange path produces for a race, and strictly
better than the flat refusal the previous code gave in that state.

*Inode reuse.* Because the fallback unlinks the placeholder, a filesystem is free
to hand its inode number to the next file created at the freed source name, and a
racing delivery would then match the recorded `staging_identity` exactly.
"Is this the placeholder?" therefore requires **zero length as well as identity**.
That cannot exclude the real placeholder, which is always empty, and a
zero-length impostor that still slips through costs zero bytes.

**Reservation residue in the diagnostic namespaces.** The reservation fallback
publishes in two steps, so a crash between them leaves a zero-length file at a
deterministic diagnostic name with the source still in place — and every one of
these sites refuses an occupied destination, which would refuse that operation
forever. `removed/` and `rename-evidence/` are diagnostic-residue namespaces and a
zero-length entry in one of them holds no bytes to lose, so an EMPTY occupant is
reclaimed and the next attempt converges. A NON-EMPTY occupant is still reported
as occupied and left untouched: that is either a real quarantine copy or a file a
sync service delivered, and neither may be destroyed.

**Receipts.** Every refusal in this module names its primitive and both names —
`renameat2(RENAME_NOREPLACE) quarantining abandoned shared provider staging
failed at "publish-859b1a…-0" -> "orphan-859b1a…-0": Invalid argument (os error
22)` — because Android CI returns only a string and a bare errno costs a
~20-minute round trip to localise.

Enforced by
`oplog::wire::tests::shared_provider_publication_without_rename2_flags_matches_the_flagged_end_state`
and `…::shared_provider_retirement_without_rename2_flags_reaches_the_exchange_end_state`
(both compare the whole provider tree against an uninjected control run on the
deterministic simulator), `…::shared_provider_retirement_fallback_crash_windows_converge`
(both windows converge to the uncrashed state, and the two boundaries that belong
to the exchange path alone are asserted unreachable),
`…::shared_provider_retirement_fallback_preserves_a_race_at_the_freed_source_name`,
`…::a_non_capability_errno_from_a_shared_provider_rename_still_fails_closed`, and
at the sharing boundary by
`sync_runtime::tests::clean_share_preparation_survives_a_shared_provider_rename_capability_refusal`
(share preparation completes, the provider tree has the control run's shape with
no residue, and a peer joins from it) plus
`…::a_non_capability_errno_from_a_shared_provider_rename_still_refuses_share_preparation`.

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
available. Honest concurrent Tine writers remain excluded by the runtime lease;
a hostile process inside the same application sandbox is outside this threat
model.

Directory-barrier refusal is classified at the receipt store's promotion
boundary, not by platform or by syscall alone:

| Receipt publication | Durability class | Android capability refusal | Required response |
| --- | --- | --- | --- |
| Initialization claim, top-level namespaces, pending-cleanup namespace and rounds, their initialization authority/state, and store claim while initializing an empty receipt store; these artifacts alone carry no operation authority | Reconstructible bootstrap | The exact file bytes remain synced; `PermissionDenied`/`Unsupported`/`InvalidInput` from the parent-directory barrier may degrade | During first activation, retry may archive one diagnostic tree and reconstruct it from unchanged Direct Files. A promoted reopen may recreate only this empty initialization structure; nonempty claimless state is refused. |
| Base, intent, attempt, mutation authority, completion, cleanup, forensic evidence, or any operational namespace after the store is opened for use | `PrivateDurableAuthority` | Never degrade, including on Android | A crash or power loss could otherwise lose a supposedly durable receipt name. Refuse the publication and keep the accepted operation pending/recoverable. Before the first create/recovery acceptance under each promoted parent in every process, establish one strict parent barrier. A successful mutation barrier verifies that parent for later exact names in the same process; a refusal or panic removes verification. Thus a process death cannot erase debt, while read-only inspection and later same-process names pay no extra barrier. |

`ProjectionReceiptStore::initialize` passes the first row's durability phase
through both the top-level helpers and the nested pending-cleanup initializer;
the ordinary receipt publisher and directory creator are the strict second
row. This keeps an Android setup capability limitation from wedging activation
without letting that setup exception leak into retained receiver authority.
On a device that permits exact app-private writes but persistently refuses
directory fsync, setup can still complete because its empty structure is
reconstructible, but the first operational receipt refuses: managed storage is
not usable without durability for its private authority. A promoted store that
has lost only its nested empty pending-cleanup initialization structure also
rebuilds that structure strictly and may refuse during open; it never discards
or silently reconstructs operational receipt authority. The verification set
starts empty in every process. Consequently an exact name left visible by a
refused barrier is never accepted after restart merely because the in-memory
record of that refusal disappeared; its parent is synchronized first.

The analogous Android early returns are classified explicitly rather than
sharing one object-store default. `object_store::ensure_directory_nofollow` is
strict: an existing parent is synchronized before it may carry a private local
journal, projection-turn journal, move episode, provider retry journal, absence
disposition record, recovery-trash name, archive namespace, or other durable
authority. `ensure_reconstructible_directory_nofollow` is the narrow exception
for the local-completion and receiver-summary caches whose authoritative inputs
remain elsewhere. Call-site source guards pin that split. `enrollment::open_component`
creates its component chain before promotion, while promoted readers use
`create=false`, so it retains the pre-promotion policy.

Before an enrollment binding exists, the empty receipt store's initialization
artifacts are reconstructible bootstrap state rather than authority; no
operational receipt is published before the activation marker. If Android
cannot reopen a receipt tree left by an interrupted or older activation, retry retains one sibling
`receipts.pre-promotion-failed` diagnostic tree and initializes a clean receipt
store from the unchanged Markdown/Org source. Once enrollment has promoted the
receipt-store identity, this recovery is forbidden: normal exact identity and
receipt recovery rules apply. Recreating an absent or empty initialization
structure is still allowed because it discards no operation receipt; a nonempty
claimless store is refused.

The same rule governs the archive a clean activation builds. Before the
activation marker is committed, the archive carries no authority and is
reconstructible from current Direct Files, and the clean lane records no private
activation reservation that a later attempt could use to attribute it. An
attempt that refuses before that marker therefore retracts the archive it
created, and only that one: an archive that predates the attempt is left exactly
where it is, so genuinely foreign residue is still refused as
`AmbiguousOrForeignResidue { ArchiveResidue, SyncConflict }`. Without the
retraction, one ordinary external write landing during activation — which makes
the final source proof refuse `Retryable { durable_stage: Absent }` — leaves an
archive that no later attempt can attribute, and every retry refuses
`SyncConflict` permanently for a graph whose only authority is still the
Markdown/Org tree beside it.

### Harvest W4-R2 — unmarked activation-generation recovery

The marker publication is the sole activation commit. A process abort can skip
Rust destructors and the attempt-level retraction, so a current-layout
generation published before that marker remains inert rather than becoming
authority. On the next explicit activation, the existing generation resolver
retires a wholly recognized unmarked archive, removes the disposable SQLite
file set, and rebuilds from the unchanged Markdown/Org source. An unknown entry
prevents that attribution and remains untouched for the ordinary foreign-residue
refusal.

| Crash cut | State observed by the next activation | Recovery |
| --- | --- | --- |
| after durable baseline publication, before SQLite publication | unmarked `lazy-genesis.0` and `operations.0`; no authority marker; SQLite absent | retire both inert generation directories and rebuild generation 0 from Direct Files |
| after SQLite publication, before the final source proof | the same unmarked generation pair plus a disposable SQLite file set; no authority marker | retire the SQLite file set and both inert generation directories, then rebuild |
| after the final source proof, before marker publication | complete unmarked baseline, operation archive, and SQLite projection; no authority marker | retire all uncommitted derived state and rebuild; source identity is proved again |
| during atomic marker publication | either one of the no-marker states above or one complete valid marker | no marker follows the corresponding recovery row; a valid marker selects generation 0 and ordinary managed open follows it |

`managed_activation_abort_cuts_retire_unmarked_generation_and_retry` pins the
three pre-marker process-abort cuts. Marker atomicity is provided by the audited
marker publication primitive and is not reproduced by a test-only partial-file
shape.

A refusal from that final source proof names what moved: the row count and, for
the first rows, the exact path together with the field that changed (filesystem
resource identity, link count, or content description), and whether the row
appeared, vanished, or changed. A file that merely appears changes neither the
source-file nor the source-chunk count, so the inventory report is the only
thing that localises it.

Reported paths escape every non-ASCII scalar (`pages/\u{17d} pilot notes.md`);
ASCII paths are reported exactly as they are on disk. A graph may hold two files
whose names differ only by Unicode normalization, and those two names print as
one glyph sequence in every log and issue tracker — a refusal that named such a
row unescaped named a row nobody could tell from its neighbour. Escaping is a
reporting rule only: nothing normalizes, folds, or rewrites a name or a byte on
disk.

`Retryable` from that proof means retryable, and callers are expected to retry
rather than surface the first refusal as a failed activation. An external
editor, a filesystem sync provider, or a second window saving while Tine is
still importing is an ordinary in-scope event; the attempt retracts the
disposable archive it created, and the next attempt rebuilds from the current
Direct Files bytes. A caller that retries must still carry every refusal it
retried past, so that a graph refusing on every attempt cannot read as a graph
that never refused.

The graph-local shared-provider tree is transport rather than local authority.
Tine still creates and opens it no-follow, requires ordinary directories and
regular files, flushes published file contents, and validates bounded bytes and
digests. On Android, inability to fsync a shared-storage directory is treated
as a platform durability limit rather than a durable refusal. App-private
enrollment, archive, journal, and SQLite directory barriers remain required.
The same limit applies to the Markdown/Org projection under §2.10a; it does not
apply to graph-tree artifacts the graph is the sole authority for.

During uninterrupted activation, SQLite's terminal builder is the single
bounded producer of parser-owned terminal page states. An activation-only
consumer derives exact-source shadow manifest evidence from those same chunks,
but cannot publish it until SQLite has completed and supplied the projection
proof that names the final shadow publication. It retains compact canonical
manifest entries, never a second graph-sized page cache. A crash or later
reopen discards that process-local evidence and uses the independent sealed
source plus archive reconstruction path; differential and crash-cut tests
require the two paths to publish identical durable shadow bytes.

The ordinary release suite tests the clean baseline-plus-manifest runtime,
including activation, cold reopen, editor/application saves, external
reconciliation, cross-page moves, graph/PDF/guide reads, sharing, late join,
restart, and clean shutdown. Every current and newly added non-ignored
`tine-core` test is selected automatically. The known-red legacy actor failure
corpus remains a regression oracle for the retirement campaign, but retired
enrollment, Patricia, persistent projection-work, and promoted-runtime
mechanics are not compiled production alternatives and cannot redefine the
release contract. The only tests the release gate does not run are enumerated
by behavior family and exact name in
`KNOWN_RED_SYNC_RUNTIME_FAILURE_FAMILIES` in
`scripts/tine-core-nextest-contract.mjs`; the contract fails both on any other
omission and on a listed name with no test behind it. The 2026-08-25 honest
unfiltered run established the current boundary: 2,071 passing, 45 normally
failing, 41 ignored, and no hangs or timeouts. A legacy-oracle failure does not
authorize a production change without an independent current-runtime
fail-before. Architectural guards that bind this document to the code therefore
enter the release suite without a second hand-maintained allowlist.

The current disposable SQLite schema identity is 22, owned by
`tine_storage::formats::SQLITE_SCHEMA_VERSION`. Bumping it invalidates only the
derived SQLite representation and costs one rebuild; it must not reinterpret
authoritative oplog bytes. Before 0.7, an unrecognized private Managed Storage
format is preserved as a backup and rebuilt from Markdown/Org into the sole
current format. Production does not carry an old-format reader, dual schemas,
or an in-place migration bridge.

### 2.10c-i The provider retry journal's completed store is bounded by provider state

`ProviderRetryJournal` (`oplog/wire.rs`, device-private, outside the graph)
keeps one record per provider filesystem operation it performs. Records in
`records/` describe operations still in flight; records in `completed/` are
crash-recovery and exact-operation idempotency evidence for operations that
finished. Both are named by a content-derived operation id — a hash over the
operation, its binding, its provenance, the paths, and the source length and
digest — so **neither directory has a chronology**. There is no "oldest"
completed record, and a time or count window over them could replay or
suppress the wrong operation after a provider namespace was lost, replaced, or
rolled back.

**Retention bound.** `completed/` is bounded by *live provider state*, never by
the lifetime of the store. Before an operation adds a completed record, if the
store holds `PROVIDER_JOURNAL_COMPLETED_COMPACTION_TRIGGER` (64) records or
more it is first compacted against the provider
(`reconcile_completed_against_provider`). The structural scan bound
`MAX_PROVIDER_JOURNAL_COMPLETED` (16,384) remains, and compaction keeps the
store far below it. The trigger is deliberately small because
`ProviderRetryJournal::load` decodes and authenticates *every* completed record
on *every* operation, so the completed count is also the ordinary path's
per-operation cost; a wider window buys nothing, because what makes an exact
repeat settle after retirement is provider state, not a retained record.
Reaching the trigger is an instruction to re-observe the provider, **not** a
reason to fail the user's next publish, rename or remove.

**What compaction retires, and why each is safe.** The predicate is derived
from the operation type and current provider state; it never consults age or
arrival order. It is the generalization of the two `recycle_completed_*`
functions that already existed for two of these cases.

| Record | Provider-state question | Retired when | Why an exact repeat still reaches the same outcome |
| --- | --- | --- | --- |
| `Put` | Is the published destination still present? | Always — present is *reflected*, absent is *moot* | Present: the repeat compares the destination bytes and settles, or refuses `ProviderConflictingBytes` when they differ — the same answer the retained record's `validate_put_destination` gave. Absent: the repeat republishes, which is what `recycle_completed_put_for_absent_destination` already arranged at operation entry. |
| `Rename` | Is the retired source back? | Always — gone is *reflected*, back is *moot* | The repeat settles from this device's own retirement diagnostic `removed/retired-<operation id>`, whose name is derived from the retired bytes and which must still hold them, with the destination still holding them too. Retaining the record is what would make a repeat over a returned source fail. |
| `Remove` | Is the removed source back? | Always — gone is *reflected*, back is *moot* | Source back: the repeat settles from the same retirement diagnostic. Source gone: a caller whose missing-source policy is `SettleIfAbsent` settles; a caller whose policy is `RequirePresent` gets `UnknownProviderPath`, the same state-derived answer that policy gives for any absent source (see the refusal row below). |

A record whose `operation_id` also appears in `records/` is never retired: a
crash can leave the same authenticated Cleanup record in both directories, and
the pending copy is what the ordinary retry validator reads.

**Compaction commits per record, and needs no generation pointer.** Each
completed record is *independently* retirable and its retirement is idempotent,
so a crash part-way through a sweep leaves a prefix retired and the rest
untouched — a state the next sweep reaches again by itself. There is no mixed
generation to publish atomically, so the generation-directory/commit-pointer
shape that multi-file compaction would require does not apply here. Every
individual removal is a `remove_file` followed by a directory `fsync`.
`oplog::wire::tests::a_crash_across_completed_record_retirement_reopens_and_still_settles`
cuts a sweep at the `CompletionRetired` boundary, reopens, and proves both
operations still settle.

**One publication answers one question.** `SharedProviderTransport::publish`
and `publish_exact` are one implementation. `publish` used to be a second,
subtly different answer that refused any destination that already existed, so
manifest, descriptor and frontier-head publications depended on a retained
completed record to make an exact repeat settle. They no longer do.

**Proof.** `oplog::wire::tests::provider_journal_completed_records_retire_against_live_provider_state`
drives 20,000 completed provider operations — past `MAX_PROVIDER_JOURNAL_COMPLETED`
— and asserts no operation fails and that the steady-state `completed/` count
stays at or below the trigger.
`…::retired_completed_provider_records_still_settle_exact_repeat_operations`,
`…::retired_completed_rename_settles_only_on_its_own_retirement_evidence` and
`…::retired_completed_remove_settles_for_the_policy_that_tolerates_absence`
prove exact idempotency after retirement, and that the settle is bound to this
device's evidence for that exact operation rather than to a destination that
merely exists.

### 2.10c-ii The provider residue directory is bounded by live journal state

`{inbox,outbox}/removed/` is the shared-tree half of the bound §2.10c-i states
for the journal. It holds two kinds of residue this store writes, and neither
is history:

* `removed/retired-<operation id>` — the exact bytes ONE completed rename or
  remove retired. While that operation's journal record exists,
  `reconcile_provider_retirement` and `validate_retired_source` read it; once
  the completed record has been compacted away (§2.10c-i), it is what an exact
  repeat settles from.
* `removed/orphan-<operation id>-<generation>` — an abandoned staging copy
  whose authority is the private journal blob. Nothing reads it as authority
  for anything; the publish path quarantines it so a foreign or crash-left
  staging file is preserved rather than destroyed.

**An operation retires its own stale orphan rather than refusing over it.**
`quarantine_unowned_staging` builds `orphan-<operation id>-<generation>` from
the operation id whose transaction gate it holds, so an occupant of that exact
name is always named for the operation doing the writing — there is no
foreign-entry case to refuse, and the refusal that used to be there wedged the
operation until an unrelated sweep happened to run past the trigger (I-10). Two
honest ways an occupant exists: a crash between the quarantine rename and the
identity-matched delete in `cleanup_journal_staging`, and an operation
re-created from scratch at generation 0 while a stale generation-0 orphan is
still in place. The owner now retires it — through the same per-entry front
door the sweep uses, `retire_provider_residue_entry`: validate the entry
no-follow-regular, `remove_file`, `fsync` the directory — and proceeds with the
quarantine. This destroys at most the one entry the sweep is already licensed
to retire once this operation finishes, and its bytes are reconstructible from
the private journal blob, which is the same reason the quarantine itself is a
RECONSTRUCTIBLE move. A NON-EMPTY occupant of any OTHER diagnostic name is
still reported as occupied and left untouched: `shared_diagnostic_name_is_taken`
keeps that answer for the `<prefix>-<digest>` quarantine, whose bytes are
foreign.
`oplog::wire::tests::a_stale_orphan_diagnostic_never_refuses_the_operation_that_owns_it`
drives the whole journey — crash, completion, provider deletion, record
compaction, exact repeat.

**The put path's `orphan-` leak window, and its bound.** The crash between the
quarantine rename and the identity-matched delete leaks one entry that outlives
its operation's record compaction. It is bounded, not absent: the name is
deterministic and the write is no-clobber, so at most ONE entry exists per
(operation, staging generation), and the sweep is what retires it — once the
record naming the operation has been compacted (§2.10c-i), the next sweep takes
it. The sweep is the only bound, and it is a real one because every production
writer of `removed/` reserves through the compacting front door
`reserve_provider_diagnostic_capacity`, so the directory cannot grow past the
trigger unswept (I-14).
`oplog::wire::tests::an_orphan_diagnostic_leaked_by_the_cleanup_crash_window_is_retired_by_the_next_sweep`
plants the crash-window entry, proves it is pinned while a record names it, and
proves the sweep retires it once none does.

A third shape, `removed/<prefix>-<digest>`, quarantines FOREIGN bytes that took
a name this device expected to own. The graph is not their authority, so no
sweep retires them. Nothing in a production build writes one
(`quarantine_provider_name` is `#[cfg(test)]`).

**The defect this replaces.** `ensure_provider_diagnostic_capacity` refused any
write that would exceed `MAX_PROVIDER_RESIDUE_ENTRIES` (512), and nothing ever
retired an entry. The cap was therefore a bound on the LIFETIME of a shared,
sync-replicated tree, and the write it refused first is the conflict-copy
CLEANUP (`remove_identical_generated_conflict`, one entry per sync-service
conflict copy cleaned up, on a boundary that produces them indefinitely). Past
512 cleanups the user kept every further conflict copy, permanently, with no
way out from inside the app (I-10, I-14).

**Retention bound.** `removed/` is bounded by *live journal state*, never by
the lifetime of the store. Before an operation adds a diagnostic, if the
directory holds `PROVIDER_RESIDUE_COMPACTION_TRIGGER` (128) entries or more it
is first swept against the journal (`reconcile_residue_against_journal`).
`reserve_provider_diagnostic_capacity` is the ONE front door every production
writer of `removed/` goes through, including the retirement placeholder in
`reconcile_provider_retirement`; the non-compacting
`ensure_provider_diagnostic_capacity` only refuses, so a second production
caller of it would reintroduce the store-lifetime cap on that one path (D-14).
`oplog::wire::tests::reserving_removed_capacity_has_exactly_one_production_front_door`
counts its callers in production source.
`MAX_PROVIDER_RESIDUE_ENTRIES` (512) remains the structural scan bound, and the
sweep keeps the directory well below it. The trigger sits above the journal's
own completed-store trigger (64) because the two stores compact in lockstep: a
diagnostic becomes retirable exactly when the completed record naming it has
been retired, so the residue steady state tracks the journal steady state.
Reaching the trigger is an instruction to re-observe the journal, **not** a
reason to fail the user's next cleanup.

**What the sweep retires, and why each is safe.** The predicate is one
question — *does any journal record, pending or completed, still name this
operation?* — and it never consults age or arrival order: these names are
hashes over the operation and its bytes, so the directory has no chronology and
a time or count window over it could suppress the wrong operation's evidence.

| Entry | Retired when | Why an exact repeat still reaches the same outcome |
| --- | --- | --- |
| `retired-<operation id>` of a **remove** | No record in `records/` or `completed/` names the operation | Source gone: the diagnostic was read by nobody — a `RequirePresent` caller gets `UnknownProviderPath` and a `SettleIfAbsent` caller settles, with or without it (see the §3.1 row). Source back: the settle shortcut goes with the diagnostic, so the repeat performs the ordinary authorized removal it would have performed had this device never run the operation. Same end state — and for the conflict-copy cleanup that is this store's only production remove, the state the caller actually wants, because the settle shortcut left the redelivered copy in place (I-10). |
| `orphan-<operation id>-<generation>` | No record in `records/` or `completed/` names the operation | Nothing reads it as authority. Retiring it also frees the name, which the no-clobber reservation in `quarantine_unowned_staging` would otherwise refuse on a repeat of the same staging generation. |
| `<prefix>-<digest>` quarantine | Never | These are foreign bytes the graph is not the authority for. No production writer exists, so the class contributes nothing to growth — `quarantine_provider_name` is `#[cfg(test)]`, which `oplog::wire::tests::only_tests_can_quarantine_a_raced_shared_provider_name` pins as compile-time structure rather than intent. |

**The rename caveat, stated rather than assumed.** A retired RENAME is the one
operation whose diagnostic stays load-bearing after its record is gone: an
exact repeat settles from it with the source gone (the destination holds the
retired bytes) *and* with the source back, and without it the repeat either
reports an unknown source path or conflicts with the destination it published
itself. The sweep cannot tell a rename diagnostic from a remove diagnostic —
both are `retired-<operation id>`, and the operation id is a hash that does not
carry the operation's paths back. What makes the sweep sound is that **no
production code writes a rename retirement diagnostic**:
`run_provider_rename_with` is `#[cfg(test)]`, which
`oplog::wire::tests::only_tests_can_write_a_provider_rename_retirement_diagnostic`
pins as compile-time structure rather than intent. A production rename writer
may not be added without giving its diagnostic a retention rule here.

**The sweep commits per entry, and needs no generation pointer.** Each entry is
*independently* retirable and its retirement is idempotent, so a crash part-way
through leaves a prefix retired and the rest untouched — a state the next sweep
reaches again by itself. There is no mixed generation to publish atomically.
Every individual removal is a `remove_file` followed by a directory `fsync`.

**Capacity and validation are two questions, not one.** The capacity check used
to answer "is there room for one more" by walking the whole directory and, per
entry, calling `open_provider_file_nofollow` and `validate_provider_regular_file`
— up to 512 opens and 512 stats on a path the user waits on, to answer a
question that needs only a count (I-13, I-15). Counting is now count-only. The
no-follow regular-file check is real work and it runs where it is used: once
per sweep, and on the exact entry every reader opens through
`open_provider_regular_optional`. Validating 511 unrelated diagnostics told the
512th write nothing, and that write is no-clobber, so a hostile sibling cannot
be published over.

**Proof.**
`oplog::wire::tests::provider_removed_residue_retires_against_live_journal_state`
drives `MAX_PROVIDER_RESIDUE_ENTRIES + 64` conflict cleanups through the
production writer — past the cap, where the old refusal fired — and asserts no
cleanup fails and that the steady-state and peak `removed/` counts stay at or
below the trigger.
`…::retired_provider_residue_still_reaches_the_same_outcome_for_an_exact_repeat`
proves the diagnostic is pinned while a record names it, retired once none
does, and that both exact repeats — source gone and source back — reach the
same end state afterwards.
`…::a_crash_across_provider_residue_retirement_reopens_and_converges` cuts a
sweep at the `ResidueRetired` boundary, reopens, and proves the next sweep
converges.
`…::provider_residue_classification_covers_only_this_stores_own_diagnostics`
pins that a foreign-bytes quarantine is not classified as this store's residue.

### 2.10d When the graph filesystem folds two page names into one file

Android CI run 32123012366 recorded the managed-storage journey's fixture
refusing to write itself on real shared storage
(`/storage/emulated/0/Download/…`):

```
journey graph fixture could not be written: graph filesystem folds two journey
page names into one file: pages/K\u{16f}\u{148} b\u{11b}\u{17e}\u{ed}.md reads
back the bytes written for pages/k\u{16f}\u{148} b\u{11b}\u{17e}\u{ed}.md
(18 bytes, not 8)
```

Two files whose names differ only by case cannot both exist there. This is not
confined to Android: FAT/exFAT removable media, NTFS, APFS in its default
configuration and any `ext4` directory carrying the casefold attribute fold
case, and HFS+ additionally folds Unicode normalization.

**Which folding, measured rather than assumed.** Three axes are probed
independently — ASCII case, non-ASCII (Unicode) case, and NFC against NFD —
because they are separable platform facts and a graph that is legal under one is
illegal under another. On the API-35 emulator the answer was **case folds,
normalization does not**: the fixture verifies its shapes in list order, and the
run above reported the case pair while the normalization pair
(`pages/\u{17d} pilot notes #pilot.md` against
`pages/Z\u{30c} pilot notes #pilot.md`) had already read back byte-exact.

AOSP disagrees with that. Android shared storage folds case through
`ext4`'s casefold attribute, whose comparison (`fs/unicode`, `utf8_strncasecmp`)
is defined over the NFDICF form, and NFC and NFD share that form — so on the
source, normalization should fold too. §2.10b already settled how that
disagreement is resolved: **upstream source is evidence about upstream intent,
not proof about the running device; the receipt wins.** The probe therefore
reports what the filesystem in front of it does, and the managed-storage journey
receipt carries the verdict verbatim as `graph_name_folding=…`, so no future
round trip is needed to learn it.

**Why this is not, by itself, a merge of two pages.** Tine's logical page name
is already case- and normalization-insensitive: `LogicalPageName::key_digest`
hashes `canonical_page_name_key`, which lowercases and then applies NFC,
matching Logseq. Every pair of file names a case-folding or normalization-folding
filesystem cannot tell apart is therefore a pair Tine **already treats as one
page**. Such a filesystem cannot merge two distinct Tine pages, because two
names it folds were never two pages here. This is the load-bearing fact behind
everything below, and it is bound to the code by
`graph_name_folding::tests::filesystem_folding_never_separates_names_tine_already_treats_as_one`.

What folding does change is that the non-authoritative DUPLICATE file — the one
`retain_authoritative_desired_pages` deliberately leaves on disk as ordinary
graph text with no page of its own — cannot exist there at all. Whoever wrote
the second spelling (a sync client, a file manager, the user) overwrote the
authoritative file instead of landing beside it.

**The contract.**

| | On a folding graph filesystem |
| --- | --- |
| Pages | Exactly ONE page per folded name — never two, never none. The twin spelling never becomes a second page, and never displaces the first. |
| Bytes | The page carries whatever the storage actually holds. An outside write to the twin spelling IS a write to that one file, so it reconciles as an ordinary external edit, not as a create for an already-owned name. |
| Availability | Folding never refuses activation, never refuses a reconciliation transaction, and never converts to an `ImportBlock`. One folded pair may not deny the rest of the graph — the same rule §3.1 imposes on the duplicate-name case, in its filesystem-shaped variant. |
| Direct Files | Unchanged and required to work. Tine writes a graph path only when it either learned that exact path from the filesystem's own directory entry or created it with an exclusive create (`O_CREAT|O_EXCL`, §2.10b), so Tine can never be the writer that destroys a folded twin: an occupied fold resolves to `AlreadyExists` before anything has moved. |
| Reporting | A fold performed by ANOTHER writer before Tine ever saw the graph is not detectable and is not reported — Tine has no evidence two files ever existed, and inventing a warning from a bare capability answer would put an unactionable message in front of every Android user. What is reported is the actionable case: a name the user asks for that this storage cannot hold beside a name it already holds, phrased by `GraphNameFolding::explain_one_file_two_names` — both spellings, which one is kept, and the one action that works. Reported once: the runtime bridge (`src/managedStorageRuntime.ts`) advances its notice sequence only for a message the user has not already been shown, so a live condition cannot re-arm the toast on every retry. |

**The probe** (`graph_name_folding::graph_name_folding`). A write/read-back pair
per axis inside one hidden, uniquely named directory under the graph root, which
is removed before returning. Deliberately a write probe rather than an
inspection of the mount table, for the reason §2.10b gives. The answer is a
property of the mounted filesystem, so it is remembered per `st_dev` — the same
key and the same reasoning as `model::FLAGGED_RENAME_UNSUPPORTED_DEVICES` — and
it is **never load-bearing**: a probe that cannot run answers
`GraphNameFolding::UNKNOWN`, which is byte-identical to "folds nothing", so no
behavior depends on it having succeeded. It writes and removes files under the
graph root, so it must not run inside a live source capture, which would report
the graph as moving underneath it; the managed-storage journey calls it before
activation starts, and the memo means the device pays for it once.

**What is deliberately NOT promised.** Tine does not reconstruct a side of a
folded pair that another writer already destroyed, and does not claim a merge it
has no evidence of. On such a device the user's graph can hold only one of the
two spellings; keeping both requires a name that differs by more than
capitalisation or accent spelling.

Enforced by `graph_name_folding::tests` (nine cases: the three axes are
independent, every path component folds, the probe leaves no residue, an
unprobeable root degrades to non-folding, a forced answer is scoped to one graph
root, and the equivalence-class fact above),
`managed_storage_journey::tests::the_fixture_writes_and_accepts_a_tree_a_folding_filesystem_can_hold`,
`…::the_fixture_refuses_a_graph_tree_that_folds_two_of_its_shapes` (a fold the
probe did NOT predict is still a refusal, and now says so),
`…::the_graph_tree_model_separates_the_two_filesystem_classes`, and at the whole-
journey boundary by
`sync_runtime::tests::android_managed_storage_journey_holds_one_page_on_a_case_folding_graph_filesystem`
and
`…::android_managed_storage_journey_holds_one_page_on_a_normalizing_graph_filesystem`.

### 2.10e Projection-turn identities and graph names

Every managed projection name is derived from its durable turn record, not from
receipt-store or process identity. Derivation scheme 1 hashes the domain tag
`tine/projection-turn/v1\0`, the big-endian scheme number, workspace, lineage,
device, endpoint, one-byte sequence domain and big-endian sequence into the
32-byte `turn_id`. For page index `i`, it hashes
`tine/projection-attempt/v2\0 || turn_id || u32_be(i) || page_id`, takes the
first 16 bytes, and applies the RFC 9562 UUID version-8 and variant masks. The
three graph names are then:

```
.{target}.{attempt_id.simple()}.projection.recovery
.{target}.{attempt_id.simple()}.projection.staged
.{target}.{attempt_id.simple()}.projection.withdrawn
```

Integers are big-endian and `simple()` is lowercase hexadecimal without
hyphens. A foreign receiver receipt reservation records this supplied attempt
id; it does not derive another one, and receiver recovery resumes its retained
durable reservation or mutation authority. Own-endpoint replay never reads that
namespace: every attempt id comes directly from the turn. Packet-2a/2b own-
endpoint residue is inert, is reported by validated names only, and is neither
resumed nor deleted. This is I2a: an undrained own turn can enumerate every
graph name its executor may have left behind without receipt evidence.

The live scheme list is `LIVE_PROJECTION_TURN_DERIVATION_SCHEMES`; an unknown
scheme is a protocol refusal, never guessed. The byte-level derivation is
enforced by `oplog::projection_turn_journal::tests`; the real-store recovery-
equivalence oracle proves turn-only own recovery over every specified crash cut
while the foreign receiver protocol remains unchanged.

### 2.10f The interrupted-publication recovery walk

Editor publication is the deliberate exception to derivable graph names (I2b).
It is shared with Direct Files and uses process-scoped recovery names. New
managed names carry four fields,
`.{target}.{pid}.{seq}.{turn8}.editor-recovery`; the parser accepts that exact
shape and the three-field legacy shape. It is a parser, not a suffix glob: the
leading dot, numeric process fields, hexadecimal turn field when present,
known suffix, and text-extension target must all validate.

Every checked graph open performs one bounded, no-follow
interrupted-publication walk before either journal is opened or replayed. The
walk uses the retained managed-text inventory limits, reads no document
contents, restores a sole claimant over a missing live name with no-replace,
and moves every competing claimant to conflict trash. Traversal, bounds,
permission, and unsafe-entry errors propagate through `open_checked`; they may
not be converted into an empty result. This is I2c: a failed walk refuses
activation before replay can publish onto an incompletely recovered tree.

`model::tests::checked_open_fails_closed_when_the_recovery_name_walk_exceeds_its_bound`,
`model::tests::editor_recovery_names_accept_legacy_and_turn_derived_shapes`, and
`sync_runtime::tests::the_open_time_recovery_walk_precedes_every_journal_replay`
enforce the bounds, grammar, API propagation, and source ordering.

### 2.10g Retain-never-delete recovery

Recovery may unlink a graph-tree object only inside the live turn that captured
both its bytes and its exact open-file identity, and only while both still
match. A reopened process has no such capability. Anything unbound, changed,
occupied, or merely byte-identical under another identity is moved intact to
`.trash/conflicts/`; it is never treated as scratch and unlinked.

Quarantine is itself a durability protocol. It validates a no-follow,
single-link source, renames no-replace into strict `PrivateDurableAuthority`
conflict trash, flushes the destination chain first and the source chain second,
then reopens and verifies identity. Created ancestors are flushed eagerly. A
hard-link, folded-name, or other refusal leaves the source in place. On the
foreign receiver path, its durable pending-cleanup receipt records the residue.
On the own-endpoint path, the in-turn exact-identity capability refuses before
the local completion is staged, so the owning journal turn cannot checkpoint
and discard the debt. Thus every derived or discovered leftover is restored,
quarantined, or reported in place before its turn checkpoints.

The crash and external-race coverage lives in the packet-2b C3-C6, R2-R5 and
X5-X6 tests, including occupied staged-name quarantine, in-turn exact-identity
retirement, post-crash retention, and hard-link refusal.

On Windows, backup restore's capability-bound move to graph-local recovery uses
hard-link-create followed by source removal, never check-then-rename. If a sync
service such as Syncthing or Dropbox delivers the same recovery name between
observation and publication, hard-link creation returns `AlreadyExists`; the
delivered entry is not replaced and the original live source remains. Linux,
Android, macOS, iOS, and Windows are explicit compile-time arms; an unknown
target cannot inherit a Tine platform's publication policy by negated fallback.

## Harvest W4-E3 — bounded clean-open source taxonomy

Clean managed construction and recovery map their 16 concrete source classes
once into the crate-private `CleanOpenError`. Its public projection preserves
the existing `OpenRefused { detail }` shape but makes the detail tagged JSON:
`kind` is `clean-open` and `reason_code` identifies the source class. No source
display text, path, or note name is serialized.

The reason codes and their exact source classes are pinned in
`docs/contracts/typed-errors.md`. Their refusal scenarios are existing §3.1
rows: bootstrap, projection, provider-scenario, and batch validation use
`MS-REF-MALFORMED-IMPORT`/`MS-REF-BOUNDS`; damaged authoritative records use
`MS-REF-DISK-CORRUPT`; provider collisions use `MS-REF-SYNC-CONFLICT`;
lease/authority races use `MS-REF-CONCURRENT-WRITER` or
`MS-REF-STALE-GENERATION`; unsafe entries use `MS-REF-UNSAFE-FS-KIND`; and
unknown current-format claims use `MS-REF-PROTOCOL-INCOMPATIBLE`. Plain I/O
unavailability remains retryable. Disposable SQLite damage still rebuilds per
§3.1 and D-3 rather than becoming a durable refusal.

## 4. Concord base ledger (Direct Files)

The Concord base ledger (ADR 0056) is **disposable state**, in the invariant-3
sense: app-private, derived, safe to delete wholesale at any time. It lives
outside every graph tree at `<app_data>/concord-ledger/<root-id>/` (the
backups' root-id convention) and stores, per graph-relative page path, the
last text Tine successfully read from or wrote to disk — sha256-addressed
blobs plus a path→hash index and conflict-copy pins (schema
`concord_ledger::LEDGER_SCHEMA`, currently 1).

It is never an authority: nothing validates against it, no refusal scenario
consults it (§3.1 is unchanged by its existence), and its loss or corruption
changes exactly one behavior — sync-conflict diffs degrade from 3-way with
pre-selected suggestions back to the plain 2-way diff until the ledger
repopulates from ordinary saves and admissions. Ledger updates are best-effort
background work off the save critical path; they may not block or fail an
open, save, or reload. It attaches only to Direct Files graphs; a managed
binding never attaches one (the oplog owns managed merge confidence,
invariant 8 stays intact).

A prune runs at graph open and reclaims everything that can no longer answer:
blobs referenced by no index entry and no pin, index and pin files that do not
parse, and index and pin entries naming a blob that is absent. `record` writes
a blob before the entry that names it, so an entry without its blob means the
blob was removed from outside the ledger — antivirus quarantine, a disk
cleaner, a partial restore. Such an entry is dead metadata whose lookups
already answer `None`; reclaiming it is hygiene, and the ledger never warns,
refuses, or reports a missing blob to the user.

### Join activation marker replacement

A semantically verified shared join holds the workspace writer lease and replaces
its activation marker through `tine-storage::DurableDirectoryPublication::replace_exact`.
The old marker remains named until the atomic replacement; there is no intermediate
rename to a `.prior` name. Exact replacement bytes permit an idempotent retry after
an uncertain outcome. Missing, malformed or unrelated markers fail closed. The
old authority generation remains available before the commit point, and installed
replacement authority is reopened after it. This fixes the existing join path; it
does not enable archive rebaselining or prove its future retention closure.


### Indexed historical callers after cold relocation

Accepted engine replay, accepted projection-work reconstruction and retained
projection-intent loading use the single ObjectStore logical resolver. Current
projection payload pins and admission readbacks remain hot. Relocation therefore
preserves historical logical presence without treating a cold directory listing as
an accepted roster. Explicit full-archive provider repair/sharing publication takes
accepted batch IDs from the engine and resolves those exact originals; it does not
publish every physically committed hot manifest. Single-batch history republication
and retained dependency recovery use the same resolver. Original manifest/object
bytes remain unchanged and manifests are published after their required objects.
These are historical read-path changes, not authorization for hot deletion or a
claim that portable baseline join and bounded active generations are installed.
