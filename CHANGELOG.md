# Changelog

All notable changes to `tine-storage` are recorded here. The crate's semantic
version describes its Rust API; persistent byte formats are versioned
independently in `src/formats.rs` and summarized in
`FORMAT-COMPATIBILITY.md`.

## [0.1.0] - 2026-08-10

### Added

- Exact immutable filesystem publication with no-follow and durability checks.
- Durable batch codecs, local journal recovery, scratch storage, and packed
  authenticated Patricia indices.
- Disposable SQLite frontier and graph materialization behind a typed facade.
- Generated public-API inventory and a production/test-support boundary gate.
- Machine-readable persistent-format manifest.

[0.1.0]: https://github.com/martinkoutecky/tine-storage/releases/tag/v0.1.0
