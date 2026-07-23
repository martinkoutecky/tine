# Tine Loro internal 1.13.6 fork

This directory is derived verbatim from the `loro-internal` 1.13.6 crate
published on crates.io. The original package metadata,
`.cargo_vcs_info.json`, `Cargo.toml.orig`, source, tests, README, and MIT
license are retained.

The contained Tine extension supplies an injected external change-store
handle, parsed-cache eviction, and sticky external store error detection.
Its unshipped v5 external checkpoint contains state, frontiers, and the
RustCrypto SHA-256 digest of the exact canonical `xm` store metadata. Every
non-empty reopen reads and validates `xm` even when a checkpoint is present.
Stale, newer, mixed, missing, truncated, or tampered checkpoint/store pairs
fail; lagging checkpoint replay is not promised.

`xm` is the single persisted copy of compact causal DAG spans, dependencies,
lamports, precomputed version vectors, the plain-text import baseline, and the
greatest timestamp. The old `ci` and `ib` keys are not written. Neither the
checkpoint nor `xm` is trusted alone: the caller must authenticate and
atomically publish the checkpoint with its store root. That authenticated root
binds the historical change blocks. The fork derives metadata from the live
OpLog/ChangeStore and validates its VV/frontiers. Every persisted causal span
contains compact authenticated source parts. A source part binds an
exhaustively validated run of stored change boundaries, its causal start, and
the exact predecessor `xm` digest under which newly materialized boundaries
were accepted. Parts can be clipped at arbitrary interior IDs and recomposed
only when their original nonlinear boundaries agree with the new DAG. The node
commitment additionally binds peer/counter span, lamport, dependencies,
precomputed VV, and successor bit.

Each rich-text tracker entry binds the complete encoded tracker snapshot
(including IDs, lamports, real IDs, origins, future/delete status, and delete
cursors) to the stable encoded `ContainerID`. A changed tracker commitment is
anchored to the exact predecessor `xm` digest; unchanged commitments are
preserved. Trackers advance only by applying newly collected text operations to
the authenticated prior snapshot. Reopen validates compact owner/commitment and
structural codec invariants without reading or decoding historical text blocks.
It retains each validated tracker as its canonical authenticated bytes instead
of constructing every mutable Fugue tracker. A tracker is decoded only when a
new operation touches that container; its tracker-local VV may lag the global
baseline across unrelated operations and is caught up before mutation.

Sealing encodes and hashes only mutable changed/new trackers. Sealed entries
reuse the exact prior owner, bytes, anchor, and digest, while each changed entry
is anchored to the immediately loaded predecessor `xm`. The just-sealed
baseline remains sealed in memory rather than being decoded again. Its exact
IB03 bytes are cached, so a no-delta flush performs no tracker codec or
commitment work and no all-container coverage scan. A constant-time arena count
invalidates that cache when even an empty text container is registered.
Borrowed import previews structurally copy sealed byte handles and round-trip
only already-mutable trackers.

An unseen plain-text container born from exactly one full insertion at position
zero uses a compact one-span constructor. It retains the complete ID, real-ID,
origin, status, and Unicode-position data and serializes to exactly the same
canonical tracker bytes as the generic Fugue builder. Any unsupported birth shape
falls back to the generic path, and a later edit promotes the sealed bytes through
the existing full decoder before mutation.

The caller-authenticated root supplies the inductive trust boundary. Before a
flush publishes a successor `xm`, the fork re-reads `xm` and requires its digest
to equal the exact metadata digest loaded with the authenticated checkpoint. It
slices prior source parts for old ranges and validates every newly uncovered
stored change boundary, proving continuous peer/counter and lamport coverage
and rejecting omitted nonlinear dependency boundaries. This uses newly
materialized blocks and prior compact proofs, not a whole-peer or text-history
scan.

In Tine, `AuthenticatedLoroStore` is an immutable COW tree whose
`LoroStoreRoot` authenticates index root, count, bytes, and witness.
`ExternalDocumentStateRecord` carries that history root together with the
matching state checkpoint, and both become visible only under one candidate
`ScratchRoots` LSM root after flush succeeds. That caller boundary binds old
change blocks and the exact prior `xm`/checkpoint pair. The fork does not claim
resistance if an attacker can replace that authenticated caller record/root;
recomputing public SHA-256 values while replacing the trust anchor is outside
the contract.

External old-base imports fail closed before mutation for nested containers,
lists, trees, rich-text styles, incomplete text tracker coverage, or unsupported
tracker metadata. Top-level scalar maps and plain-text inserts/deletes remain
bounded by the incoming/touched branch. Plain-text baseline size is
`O(live Fugue spans + retained deletion cursors/tombstones)`, so front-edit and
edit/delete histories can retain proportional structural metadata. The size
test is a smoke ceiling against accidental blowups, not a sublinear-metadata
claim. This residual is not duplicated in the checkpoint or separate keys.

The extension preserves the upstream change-block and state encodings and
leaves the default in-memory path unchanged.
