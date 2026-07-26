# 0051. Single-user multipart bootstrap authority is one commit-last V1 publication

- **Status:** Accepted — inactive format contract; it neither enables activation nor
  proves the implementation complete
- **Date:** 2026-07-26
- **Amends:** the bootstrap boundary in [ADR 0049](0049-oplog-first-sparse-storage.md)

## Context

The sparse-oplog program needs to turn a complete ordinary graph import into an
initial, multipart operation history without making a prefix authoritative or
retaining graph-sized input in memory. The earlier inactive bootstrap shape carried
an archive issuer and peer probes. Those claims do not improve recovery for the
actual first-rollout fault model and would imply a second authority protocol.

Martin clarified the threat model on 2026-07-26: this is one user operating honest
Tine installations on several devices through fallible filesystem sync providers.
The relevant faults are stale replicas, partition/replay/reordering, crashes and
durability cuts, ordinary I/O failures, accidental corruption, external edits, and
large or unusual valid graphs. Malicious peers, hostile collaborators, untrusted
bundles, intentional forged protocol bytes, and deliberate denial of service are
deferred. Digests, canonical decoding, bounded work, and no-follow filesystem rules
remain required to detect accidental damage and avoid ambiguous recovery; they do
not turn this bootstrap into a Byzantine-peer protocol.

## Decision

Bootstrap has one corrected, still-inactive V1 format. The schema value remains
`1`, and its decoder accepts only the recut canonical bytes. There is no internal
legacy variant, bootstrap V2, operation-witness protocol, issuer identity, secret,
or peer-probe mechanism. `CanonicalGraphResourceId` remains source provenance only;
a receiving graph does not compare it. This does not alter ADR 0049's separate
fence around the old experimental `.tine-sync/v1` store, and it does not bump the
oplog protocol, operation, manifest, envelope, semantic-effect, receipt,
projection, or diff versions. Only the durable engine-history root and cold record
schemas change, from `7` to `8` and `11` to `12`, to carry the bootstrap binding.

The aggregate contains `LineageDigest`, the profile digest, the complete source
inventory and source-blob roots, bounded ordered parts, and the final frontier. Its
portable publication name is exactly:

```text
BootstrapPublicationIdV1 =
  SHA-256("tine/bootstrap-publication/v1\0" ||
          workspace || lineage || import_id || profile_digest ||
          source_inventory_root || source_blob_root)
```

It has no issuer, secret, receiver-local graph ID, or scan-dependent input. Source
leaves carry `ManagedTextKind` and the exact UTF-8 `ManagedPath`, rather than a
layout reconstructed from `pages/` or `journals/`. `OperationRootV1` remains only
partition provenance and the per-part authoring cap; each part reuses normal
canonical `OperationBatch` manifest bytes and `OperationObject` envelopes, whose
descriptors use their normal `ContentDigest`. The existing roots, full-object
commitment, predecessor chain, frontier chain, canonical encodings, chunk digests,
and size checks remain. The final-frontier proof binds workspace, lineage, import
ID, profile, and final frontier.

### Publication and installation

Artifacts are written immutably under `bootstrap-v1/` and loaded only by direct
namespaced lookup:

- `source-inventory-indexes/<inventory-root>/<page>`
- `source-blob-indexes/<blob-root>/<page>`
- `source-chunks/<chunk-digest>`
- `parts/<part-id>` and `part-spans/<part-id>`
- `objects/<content-digest>`
- `aggregates/<aggregate-digest>` and `commits/<publication-id>`

The source indexes are canonical, paged ordered entries with root/count
cross-checks, at most 1 MiB per page and 4,096 pages; part spans are a bounded,
direct index. Inventory and blob descriptors each admit at most 1,000,000 entries.
All reads verify canonical bytes, declared length, and content digest. Immutable
publication uses the existing temp-write, fsync, no-replace rename, and directory
sync primitive: identical bytes are idempotent, while different bytes at a direct
name are typed conflicts. Raw source chunks are provenance and recovery material;
restart does not need to reread every raw chunk before rebuilding validated parts.

Prefix parts are not authority. Objects, indexes, parts, and an aggregate may exist
without affecting accepted history. Only a validating aggregate commit marker at
`commits/<publication-id>` makes the bundle eligible for admission. Bootstrap is
admitted only into an empty accepted durable history. Generic
`stage_manifest_bytes` and `publish_prepared` reject `BatchOrigin::BootstrapImport`;
bootstrap-specific object-store APIs bind each aggregate descriptor's expected
part/batch ID, manifest fingerprint, and object set instead. Thus multipart
bootstrap batches are replay payloads, not ordinary independently accepted history.

Admission validates each part sequentially in a detached, scratch-backed candidate,
loading at most that part's manifest, objects, spans, and semantic effect. No prefix
mutates the live engine. Prepared immutable history nodes and a root may be harmless
orphans until `DurableEngineHistoryStore::publish_many_exact` makes one durable
transition: it takes the workspace-wide no-follow advisory transition lock plus the
existing mutex, rereads the expected empty head, verifies exact prepared records and
binding, publishes the new root, replaces `engine-history.head` once, and fsyncs the
control directory. Ordinary single-record publication uses the same lock. A
zero-part bootstrap replaces only the authenticated generation-zero root binding.

`EngineHistoryBinding` and every cold history record mirror the publication ID,
aggregate digest, part count, and final frontier so existing root/latest-record
equality checks remain meaningful. After a crash following head replacement,
startup reads that binding, directly loads the one commit and aggregate, rebuilds
the parts detached, and exposes state only when the rebuilt frontier matches.

### Streaming import and compatibility

The author path is bootstrap-specific; it does not reuse `ImportExecutionMaterial`.
It preserves the current `ImportIdDerivation` v2 bytes exactly, incrementally feeding
canonical completion and inventory entries without changing `receipt.rs`. Graph's
configured managed-root classification, exact `ManagedPath`, and `ManagedTextKind`
are authoritative, so nested, Unicode, and nonstandard layouts survive unchanged.

It first captures sorted metadata, then rereads, verifies, parses, and chunks one
source file at a time while spooling deterministic, phase-ordered operation records
and emitting one bounded part at a time. The V1 profile records a 64 MiB per-file
cap, 64 GiB total source-byte cap, and 1,000,000-node per-file parser cap; exceeding
any cap blocks before publication. It retains the existing parser, with peak owned
working data limited to one source file, one parser tree, one semantic effect of at
most 48 MiB, one object set of at most 192 MiB, and bounded index/spool buffers.
Parts are capped at 1,024, with at most 4,096 authoring operations each.

## Consequences

- The same honest-device input has one deterministic portable publication ID and
  direct integrity-checked retrieval. It is not authenticated against a malicious
  publisher, because that is outside this rollout's fault model.
- A crash before the one head replacement leaves no bootstrap authority; a crash
  after it forces direct commit/aggregate validation and detached rebuild before
  state becomes visible. A bootstrap cannot merge into or partially extend accepted
  durable history.
- Memory and file/graph work are explicitly capped while preserving exact existing
  `ImportId`, path, text-kind, and configured-layout semantics. Inputs above a cap
  block rather than being silently truncated, re-laid out, or partially published.
- The format remains inactive. Acceptance records the authority boundary only; it
  does not claim completion, enable `LocalActive` or sharing, or change the other
  activation gates in ADR 0049 and the execution campaign.

## Rejected alternatives and future trigger

We reject issuer/secret-backed archive authority, peer probes, receiver-local graph
identity, a second bootstrap generation or retained internal legacy decoder,
operation witnesses, scanning discovery, prefix-by-prefix history publication,
generic batch admission, path reconstruction from conventional directories, and
graph-sized import buffering. Each either adds authority without an honest-device
recovery benefit or violates the bounded and compatible import contract.

If Tine accepts untrusted sync bundles or gains a multi-user/collaborator boundary,
that is the trigger for a separately approved malicious-input hardening project. It
must define its own authentication, adversarial decoding/resource limits, and
authority/revocation model; it must not silently reinterpret this inactive V1 format
or be inferred from accidental-corruption checks.
