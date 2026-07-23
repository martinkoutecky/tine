# Tine Loro 1.13.6 fork

This directory is derived verbatim from the `loro` 1.13.6 crate published on
crates.io, except for the small external change-store checkpoint extension
documented below. The original package metadata, `.cargo_vcs_info.json`,
`Cargo.toml.orig`, README, and MIT license are retained.

Tine-specific changes:

- use the adjacent exact-version `loro-internal` fork;
- expose construction from an injected change-store handle plus a state
  checkpoint;
- expose fallible external-store flush/checkpoint and cache-eviction methods;
- use an unshipped v5 exact-pair checkpoint: every non-empty reopen reads and
  validates canonical `xm` metadata, then requires the checkpoint's RustCrypto
  SHA-256 digest and frontiers to match it exactly; stale, newer, mixed, missing,
  truncated, or tampered pairs fail instead of replaying a lagging checkpoint;
- keep causal DAG spans and the rich-text import baseline only in `xm`; the
  checkpoint carries state, frontiers, and the exact `xm` digest, and the old
  `ci`/`ib` keys are not written;
- require callers to authenticate and atomically publish each store root with
  its matching checkpoint. Neither artifact is trusted alone. The
  authenticated root is what binds historical change blocks and the exact
  prior `xm`/checkpoint pair. Flush first re-reads and matches that predecessor.
  Causal spans contain compact source proofs which can be sliced at arbitrary
  interior IDs and recomposed only when prior nonlinear boundaries are
  preserved. Newly materialized source runs are validated exhaustively and
  anchored to the predecessor `xm` digest. Reopen verifies compact proofs
  without causal-block or peer-history reads;
- bind each plain-text tracker to its stable encoded `ContainerID` and commit
  the complete canonical tracker snapshot, including real IDs, origins,
  future/delete status, and delete cursors. Changed trackers are derived by
  incrementally applying new operations to the authenticated prior tracker and
  are anchored to the predecessor `xm` digest. Reopen validates tracker wires
  without materializing mutable tracker trees or reading historical text
  blocks. Untouched trackers retain their exact authenticated bytes; sealing
  encodes and hashes only changed/new trackers, retains the sealed baseline,
  and reuses cached IB03 bytes for a no-delta flush. A constant-time arena
  count invalidates the cache when a new empty text container needs coverage;
- construct a strict one-span tracker directly when an unseen plain-text
  container is born from one full insertion at position zero. The compact path
  emits byte-for-byte the same canonical tracker snapshot as the generic Fugue
  builder; later edits decode and promote it through the existing full tracker
  path, while multi-operation or styled births use the generic path immediately;
- match Tine's caller contract: `AuthenticatedLoroStore` is an immutable COW
  tree authenticated by `LoroStoreRoot`, and the matching history root plus
  checkpoint are published together under one candidate `ScratchRoots` LSM
  root only after flush succeeds. Replacing that authenticated caller
  record/root is outside the threat model; public SHA-256 is not a secret MAC;
- fail closed before external import for nested containers, lists, trees,
  rich-text styles, or incomplete text tracker coverage. Top-level scalar maps
  and plain text inserts/deletes use the bounded old-base path;
- add executable map/text convergence, multi-block interior causal splitting,
  short-versus-4096-history counting, both-delivery-order, reopen/eviction,
  exact-pair corruption, tracker commitment/tamper, fully recomputed
  predecessor forgery, post-journal physical-block rollback, unsupported-shape,
  physical-read accounting, metadata-size, no-delta zero-codec, one-touched
  tracker, and reopened two-text old-base tests.

The fork intentionally does not change Loro's encoded change-block or state
formats. Plain-text tracker metadata is
`O(live Fugue structural spans + retained deletion cursors/tombstones)`;
repeated front edits or edit/delete workloads can therefore grow with retained
CRDT structure. The test ceiling is only a smoke guard, not a sublinear-size
claim. Metadata is stored once rather than in `xm`, checkpoint, and separate
keys. Remove the fork when upstream provides an equivalent fallible external
change-store API.
