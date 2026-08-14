# Persistent-format compatibility

Crate semver and persistent-format identity are deliberately independent.
`tine-storage` 0.1.0 writes and reads the format family below. The authoritative
machine-readable values are `tine_storage::formats::FORMAT_MANIFEST`; this table
is a review aid, not a second source of truth.

| Artifact | Format identity in 0.1.0 | Compatibility rule |
| --- | --- | --- |
| Oplog manifest/object protocol | protocol 2; object envelope 2; manifest encoding 4 | Existing versions must remain readable or receive an explicit migration before a writer changes these values. |
| Local journal | frame schema 1; segment/frontier protocol 2; `TINEJNL2`/`TINEFRT2`; 136-byte header; 240-byte `.frontier-v2` | V1 inspection remains non-mutating. V2 recovery accepts only the exact selected header and frontier and treats bytes beyond a valid old frontier as uncommitted suffix. |
| Engine scratch | run schema 13; page schema 1; 32 LSM levels; `engine-scratch-v2` layout | Scratch is reconstructible, but an interrupted run must either resume safely or be rejected and rebuilt. |
| SQLite projection | application ID `0x54494e45`; schema 19 | SQLite is disposable. A mismatch must rebuild from authoritative history, never reinterpret rows under a new schema. Schema 19 preserves ambiguous external Logseq UUID claimants and append-only block-ID/home-document claims with accepted-batch and causal provenance for bounded application-level classification. |
| Checkpoint fingerprints | 64 KiB edges; 16 KiB interior ranges; 1 MiB interior sampling interval | Stored and freshly computed fingerprints are comparable only with identical geometry. |
| Managed-storage layout | Shared-provider, archive, lazy-genesis, projection, reconciliation, enrollment, bootstrap, migration, and source-capture path vocabulary frozen in `managed-layout-v1.txt` | The lazy-genesis names are additive before 0.7 and old crates simply ignore them. Future changes must preserve old readers/writers or carry an explicit migration/rebuild rule. |

Writer bounds are also part of compatibility because lowering them may strand
already-written data. The exact released bounds are pinned by
`formats::tests::format_identity_is_pinned` and included in every certification
receipt.

Any change to `FORMAT_MANIFEST` requires all of the following in the same
storage release:

1. a changelog entry explaining the old-data behavior;
2. old-version fixtures or an explicit rebuild/migration proof;
3. an updated pinned-format test;
4. a new certification receipt and a new Tine pin receipt.
