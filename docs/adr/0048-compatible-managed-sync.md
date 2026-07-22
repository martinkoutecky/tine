# 0048. Managed sync uses operation truth with an optional shared Markdown projection

- **Status:** Superseded by [ADR 0049](0049-oplog-first-sparse-storage.md)
- **Date:** 2026-07-11

## Context

Syncthing, Dropbox, and similar tools synchronize file snapshots. Concurrent edits
therefore produce conflict copies even when the edits affect different blocks. Git
adds history but has the same snapshot-merge problem. Tine must improve this without
requiring a Tine-operated server or taking plain Markdown access away from existing
Logseq and editor workflows.

A CRDT proves convergence only after replicas receive the same valid operations. It
does not make file watchers, crash recovery, external Markdown reconciliation, or
projection writes correct. Those adapters are the data-dangerous part of the design.

## Decision

Tine's opt-in managed sync has one operation model and two projection layouts:

- **Compatible mode:** the existing graph remains in the provider-shared folder and
  `.tine-sync/` is added beside it. Markdown is an editable projection, so OG Logseq
  and other tools retain access without changing the provider configuration.
- **Private mode (later):** only encrypted `.tine-sync/` state is shared and each
  device projects Markdown into a device-local graph. The CRDT and update format are
  unchanged; only the projection location and encryption policy differ.

The implementation obeys these invariants:

1. **Operations are durable first.** A Tine mutation is committed to the local
   device's immutable update stream before its Markdown projection is published.
   Projection failure can leave stale Markdown, never an unlogged edit.
2. **One writer per stream.** Each installation has a device identity stored outside
   the graph and each process writes a fresh
   `.tine-sync/v1/devices/<device>/sessions/<session>/` stream. Content-addressed
   chunks are immutable, checksummed, and atomically published; a provider never
   merges an append-in-place log file.
3. **CRDT state is sync truth.** Loro supplies the movable-tree/text CRDT, version
   vectors, and update encoding. Tine owns the Logseq-specific page/block model,
   persistence envelope, and projection/reconciliation adapters.
4. **Markdown remains an input, not a second truth.** A projection whose content is
   not explained by the known operation frontier is imported as an external edit.
   Exact persisted block ids anchor reconciliation; ambiguous changes stay visible
   in the existing conflict UI and are never guessed away.
5. **Conflict copies are evidence.** Once all known updates are imported, Tine may
   trash a provider conflict copy automatically only when a projection receipt proves
   its content was generated from a known CRDT frontier. Unexplained bytes are
   preserved for manual import/merge.
6. **Projection policy is versioned.** The compatible v1 schema preserves the
   existing graph's Markdown/Org conventions and records exact-byte receipts. A
   future private-mode schema must additionally define canonical bytes because it
   cannot rely on a shared existing projection. An incompatible client must not
   publish a projection.
7. **The existing write protocol remains singular.** Projection publication and
   external-edit import use `Graph::write_page` / `commit_write` and the watcher
   reload disposition from ADR 0012. The sync engine does not own a second page-file
   writer.
8. **Identity migration is explicit and recoverable.** Enabling managed sync first
   assigns a durable Logseq-compatible id to every synchronized block through the
   normal save path. Preflight must succeed before activation; interruption is
   resumable and never makes an un-IDed block silently alias another block.

Device updates and projection receipts are transport-neutral. A filesystem provider
is the first mailbox; direct P2P and durable provider APIs can consume the same
envelopes later. Encryption is part of the envelope version from the start, but
provider-blind encryption becomes mandatory before private/provider-backed managed
sync is declared stable.

The experimental compatible v1 implementation manages page and journal text only.
Assets, PDF sidecars, and `config.edn` continue to be synchronized as ordinary files
by the user's provider. Backup restore is operation-first: a verified post-migration
snapshot becomes one graph replacement operation before files are copied, and an
immutable projection intent lets startup finish that explicit overwrite after a
crash. Pre-migration backups without durable block ids are refused while managed
sync is active rather than assigned nondeterministic identities.

This decision narrowly supersedes ADR 0020's manual-only policy: a conflict copy may
be removed automatically only when an exact projection receipt proves it is generated
output. ADR 0020 remains authoritative for every unexplained conflict copy.

## Consequences

- Existing Syncthing/Dropbox users can opt in without moving their graph. Conflicts
  caused solely by Tine projections become regenerable artifacts once operations
  arrive, while plain Markdown remains beside the logs.
- Fully automatic convergence applies to Tine-originated operations. An unmodified
  external editor cannot express move/delete intent, so its snapshots are imported
  conservatively and may still require review.
- Every block gains a persisted id on first activation. This creates a large but
  semantically inert initial graph diff and must be previewed, backed up, and tested
  against OG Logseq before the feature leaves experimental status.
- A provider or direct peer is still required to deliver updates. CRDT convergence
  removes central semantic authority, not the physical need for communication or a
  durable mailbox when devices never overlap online.
- The adversarial test oracle is part of the feature: partitions, duplicate and
  reordered updates, crash points, mixed projection arrivals, and external edits must
  converge without silent loss before managed sync may write a real graph by default.
