# Changelog

All notable changes to `tine-storage` are recorded here. The crate's semantic
version describes its Rust API; persistent byte formats are versioned
independently in `src/formats.rs` and summarized in
`FORMAT-COMPATIBILITY.md`.

## [Unreleased]

## [0.3.0] - 2026-08-10

### Added

- Bounded SQLite reads for exact-marker task-candidate blocks and block-only
  structure. Candidate pagination seeks by `(page_id, block_id)` and returns
  only the raw block/page fields needed for application-owned parsing;
  structural point reads exclude content, search text, and public UUIDs.

## [0.2.0] - 2026-08-10

### Added

- Local-journal protocol v2 with a checksummed segment identity and an ordered,
  separately durable frontier. A returned append is selected exactly once;
  an unreturned physical suffix is discarded on reopen without weakening a
  previously committed frontier.
- A typed durable-directory publication API for exact create, replacement, and
  authority retirement. Windows proves the capability in an owned namespace
  and uses `MoveFileExW` write-through publication with exact byte and file
  identity verification.

### Changed

- Legacy v1 journal rollover now inspects ambiguous suffixes without mutating
  them before a migration decision.
- The ordinary Patricia certification suite separates a 4,096-record semantic
  differential from a 96-record physical publish/reopen proof; the complete
  4,096-record physical journey remains a required release burn-in.
- The Rust API intentionally adds variants to exhaustive storage and journal
  error enums. This requires the `0.2.0` compatibility boundary.

## [0.1.1] - 2026-08-10

### Fixed

- Local-journal recovery now preserves the segment and fails closed when a
  fully sized final frame fails validation or a damaged length field makes its
  extent beyond EOF ambiguous. Only a byte tail too short to contain any
  complete frame is truncated, preventing corruption from silently discarding
  a previously durable commit.

## [0.1.0] - 2026-08-10

### Added

- Exact immutable filesystem publication with no-follow and durability checks.
- Durable batch codecs, local journal recovery, scratch storage, and packed
  authenticated Patricia indices.
- Disposable SQLite frontier and graph materialization behind a typed facade.
- Generated public-API inventory and a production/test-support boundary gate.
- Machine-readable persistent-format manifest.

[Unreleased]: https://github.com/martinkoutecky/tine-storage/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/martinkoutecky/tine-storage/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/martinkoutecky/tine-storage/compare/v0.1.1...v0.2.0
[0.1.1]: https://github.com/martinkoutecky/tine-storage/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/martinkoutecky/tine-storage/releases/tag/v0.1.0
