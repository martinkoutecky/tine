# Changelog

All notable changes to `tine-storage` are recorded here. The crate's semantic
version describes its Rust API; persistent byte formats are versioned
independently in `src/formats.rs` and summarized in
`FORMAT-COMPATIBILITY.md`.

## [Unreleased]

## [0.8.2] - 2026-08-14

### Added

- SQLite frontiers now support an immutable lazy-genesis baseline: baseline
  dependencies remain in the external pack while SQLite stores only later
  accepted-document overlays. Sequence-zero installation and the first edit of
  an existing baseline document therefore require neither copied baseline rows
  nor a fabricated accepted batch.

## [0.8.1] - 2026-08-14

### Added

- A fresh SQLite candidate can atomically install an authenticated
  sequence-zero genesis frontier and its document map without fabricating an
  accepted bootstrap batch. The primitive refuses nonempty history and is the
  physical foundation for Tine's operation-free managed-storage activation.

## [0.8.0] - 2026-08-14

### Changed

- SQLite schema 20 preserves every block that claims the same external Logseq
  UUID and exposes those claimants through one bounded, canonical multi-row
  read. Ambiguous source graphs are now application-visible input rather than
  a projection-construction failure or an arbitrary physical owner.
- The same schema retains append-only block-ID/home-document claims derived
  from accepted history, including their accepted batch and optional causal
  dot. Deleting a live block no longer erases the evidence needed to classify
  sequential versus concurrent identity reuse; all candidates are exposed
  through a bounded canonical read.
- The disposable projection now also retains application-owned causal
  page-name and portable-path ownership records behind bounded SQLite point
  reads, plus append-only external-UUID introductions with baseline or
  accepted-batch provenance. These tables are the physical replacement for
  Tine's custom Patricia identity indexes; storage does not interpret their
  domain semantics.
- The former singular `block_by_logseq_uuid` API is replaced by
  `blocks_by_logseq_uuid(logseq_uuid, limit)`. Callers must classify ambiguity
  explicitly.

## [0.7.0] - 2026-08-14

### Changed

- SQLite schema 18 makes reference postings and aliases ordinary
  parser-derived projection facts. The disposable database now has one
  frontier stamp and one regime-neutral apply path instead of retaining an
  unused second reference-catalog authority.

### Removed

- Removed the authenticated reference-catalog change, stamp, coverage, and
  terminal-catalog APIs, together with their three authority tables and five
  indexes. Direct Files and managed storage now feed the same physical
  reference projection surface.

## [0.6.5] - 2026-08-14

### Added

- Managed materialization changes can now publish parser-derived reference
  postings, aliases, and portable-path claims directly in the accepted-frontier
  transaction. These are disposable current-state facts and no longer require
  clients to manufacture a second authenticated reference-catalog authority.

### Fixed

- Ordinary parser-derived page replacements now refresh the affected alias
  candidate bindings in the same transaction as their declarations. Alias
  navigation no longer waits for a terminal rebuild after an edit.

## [0.6.4] - 2026-08-14

### Added

- The certified managed-storage layout now names the content-addressed lazy
  genesis archive and its manifest, commit, catalog-update, and segment files.
  This is the physical publication vocabulary for Tine's new activation
  format; authority and semantic validation remain owned by Tine core.
- The disposable SQLite graph projection now retains caller-derived portable
  path keys as a bounded, non-unique page-candidate index. Direct Files and
  managed storage can share path-identity lookup while preserving every
  case/Unicode-equivalent conflict for application-level diagnosis.
- Page home-document ownership is now a bounded, non-unique candidate lookup,
  so duplicate CRDT-home claims remain diagnosable without a handwritten
  application index.

### Changed

- The disposable SQLite projection schema is now 17. Older projections rebuild
  from authoritative graph/history input rather than being reinterpreted.

## [0.6.3] - 2026-08-14

### Changed

- Fresh terminal bootstrap now builds ordinary SQLite secondary indexes once
  after inserting the complete row set. The exact normal schema is restored
  inside the unpublished candidate transaction before its authenticated stamp
  advances; Direct Files, ordinary managed updates, and reopen behavior are
  unchanged.

## [0.6.2] - 2026-08-14

### Fixed

- Private Patricia construction now divides a sorted update range larger than
  one resident bulk sink into consecutive canonical bulk publications. Dense
  reference indices no longer fall back to hundreds of thousands of loose
  per-key immutable files merely because the complete range exceeds one
  bounded construction buffer.
- Packed Patricia catalogs admit up to 128 MiB of exact immutable pack data.
  This remains a hard decode and construction bound, while covering the
  derived reverse-reference index of a representative 130,000-block graph
  without forcing the otherwise valid tail into loose per-node files.

## [0.6.1] - 2026-08-13

### Added

- Regime-neutral graph-projection changes can transactionally replace the
  parser-derived aliases owned by each replaced page. Replacement, deletion,
  and reopen now leave no stale alias declarations in the disposable SQLite
  projection.
- The complete managed-storage path vocabulary is now exported through the
  certified format manifest and pinned to the exact pre-migration Tine values.
  This changes ownership and reviewability only; no persisted path changes.

## [0.6.0] - 2026-08-13

### Added

- Regime-neutral graph-projection changes can transactionally replace the raw
  page-reference spellings owned by each replaced page. Direct Files can now
  populate the same disposable `reference_postings` facts and bounded
  navigation-name read used by managed storage, without a second application
  cache or a whole-graph referenced-name scan.
- Disposable projections now maintain a SQLite trigram index and expose
  bounded page candidates for exact normalized literal-substring matching,
  including matches in the middle of tokens. One- and two-character needles
  use the bounded page inventory; the application parser remains the final
  semantic matcher for every query.
- Disposable projections expose bounded page candidates for Unicode-normalized
  ordered-subsequence fuzzy matching. This is a candidate superset: the
  application parser remains the final owner of fuzzy semantics and ordering.

### Changed

- The disposable SQLite projection schema is now 16. Older projections rebuild
  from authoritative graph/history input rather than being reinterpreted.

## [0.5.0] - 2026-08-13

### Added

- Disposable SQLite projections can return bounded task-candidate structural
  locators without copying block content or public UUIDs. Applications that
  already retain the exact parser document can recover and identity-check the
  corresponding parser block while managed storage keeps the existing full-row
  physical API.
- Standalone disposable projections can transactionally retain each page's
  caller-owned source revision and report only changed, missing, or deleted
  pages on reopen. Direct Files can therefore reuse exact SQLite facts after a
  clean restart and lower only externally changed parser documents; these
  revisions are adapter metadata and do not alter managed-storage formats.

### Fixed

- A refused legacy-journal recovery scan now explicitly releases its retained
  writer lock before returning, so an immediate retry reports the original
  corruption instead of an intermittent false “already open” refusal.

## [0.4.0] - 2026-08-13

### Added

- A regime-neutral disposable SQLite graph projection with one typed page
  replace/delete transaction and the existing bounded page, block, task,
  property, reference, and search reads. Direct Files and managed storage can
  now feed the same physical graph-fact tables without exposing an oplog
  frontier or raw SQLite connection to Direct Files.

### Changed

- Managed materialization delegates its graph-row work to the same extracted
  projection engine while retaining accepted-frontier stamps and reference
  authentication in its managed-only adapter. The on-disk managed format is
  unchanged.

## [0.3.1] - 2026-08-13

### Fixed

- Android immutable bootstrap batches now fall back from unavailable
  filesystem-wide synchronization to exact per-file synchronization, while
  retaining strict failure for ordinary I/O errors.

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

[Unreleased]: https://github.com/martinkoutecky/tine-storage/compare/v0.8.2...HEAD
[0.8.2]: https://github.com/martinkoutecky/tine-storage/compare/v0.8.1...v0.8.2
[0.8.1]: https://github.com/martinkoutecky/tine-storage/compare/v0.8.0...v0.8.1
[0.8.0]: https://github.com/martinkoutecky/tine-storage/compare/v0.7.0...v0.8.0
[0.7.0]: https://github.com/martinkoutecky/tine-storage/compare/v0.6.5...v0.7.0
[0.6.5]: https://github.com/martinkoutecky/tine-storage/compare/v0.6.4...v0.6.5
[0.6.4]: https://github.com/martinkoutecky/tine-storage/compare/v0.6.3...v0.6.4
[0.6.3]: https://github.com/martinkoutecky/tine-storage/compare/v0.6.2...v0.6.3
[0.6.2]: https://github.com/martinkoutecky/tine-storage/compare/v0.6.1...v0.6.2
[0.6.1]: https://github.com/martinkoutecky/tine-storage/compare/v0.6.0...v0.6.1
[0.6.0]: https://github.com/martinkoutecky/tine-storage/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/martinkoutecky/tine-storage/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/martinkoutecky/tine-storage/compare/v0.3.1...v0.4.0
[0.3.1]: https://github.com/martinkoutecky/tine-storage/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/martinkoutecky/tine-storage/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/martinkoutecky/tine-storage/compare/v0.1.1...v0.2.0
[0.1.1]: https://github.com/martinkoutecky/tine-storage/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/martinkoutecky/tine-storage/releases/tag/v0.1.0
