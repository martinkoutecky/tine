# tine-storage

[![certify](https://github.com/martinkoutecky/tine-storage/actions/workflows/certify.yml/badge.svg)](https://github.com/martinkoutecky/tine-storage/actions/workflows/certify.yml)

`tine-storage` owns physical persistence mechanisms. The dependency direction
is `src-tauri -> tine-core -> tine-storage`: core supplies policy, authority,
validation, and domain meaning, while this crate supplies storage operations.
It has no dependency on `tine-core`, `lsdoc`, Tauri, or UI crates.

SQLite is a disposable local projection. The oplog/archive is authoritative,
so recovery can rebuild SQLite rather than treating it as a source of truth.
Consumers use the curated `tine_storage::sqlite` facade; it exposes typed
physical operations without raw connections or DDL construction details.

## Persistent-format identity

`tine_storage::formats` collects every constant that describes bytes already on
disk — envelope/schema versions, on-disk names and layout, the bounds a writer
may legally have produced, and checkpoint fingerprint geometry — and exposes
them as `FORMAT_MANIFEST`.

**On-disk format versions are independent of this crate's semver.** The crate
version tracks the Rust API; these constants track the bytes. An API-breaking
refactor that reads and writes identical bytes changes nothing in the manifest,
and a one-field change to a stored envelope changes it even in a patch release.

A storage release receipt and Tine's storage pin receipt should be generated
from `FORMAT_MANIFEST` rather than transcribing values by hand, because a
hand-copied receipt drifts silently and the drift is invisible exactly when it
matters. `formats::tests::format_identity_is_pinned` asserts the exact current
values, so changing an on-disk format cannot pass CI without a deliberate edit
a reviewer sees; when that test fails, update it together with the migration
story for existing graphs, not on its own.

In-memory budgets and read-path limits are deliberately excluded from the
manifest: they bound one process's work, not the bytes it leaves behind.

Package-local test ownership is intentionally divided as follows:

- Persistent-format invariants: `durable_batch::tests` and `digest_sealed::tests`.
- Durability and filesystem publication invariants: `filesystem::tests`.
- Authenticated-index invariants: `authenticated_patricia::tests`.
- Scratch lifecycle and retained-run invariants: `scratch::tests`.
- SQLite transaction and schema invariants: `sqlite_frontier::tests` and
  `sqlite_materialization::tests`.
- SQLite facade, connection ownership, and test-support-gate invariants:
  `sqlite_database::tests`.

## Public API surface

`api.txt` records every publicly reachable name, its export path, and whether it
is gated behind `test-support`. It is generated, not hand-maintained:

```
TINE_STORAGE_BLESS_API=1 cargo test -p tine-storage api_surface
```

A change to the surface fails `api_surface::tests::api_surface_matches_the_recorded_golden`
until `api.txt` is regenerated in the same commit, so a version can be cut
against a surface someone actually reviewed. It records names, not signatures —
rustdoc JSON would give signatures but is nightly-only, and this crate builds on
the pinned stable toolchain.

Two rules the tests enforce, both of which exist because this crate is becoming
an independently versioned package with an exact Tine pin:

- **A persistent-format constant has exactly one export path**,
  `tine_storage::formats::NAME`. A receipt generated from `FORMAT_MANIFEST`
  claims to state the format surface a build commits to; a second path would let
  a consumer bind to a constant the receipt never mentions.
- **`test-support` never reaches a release build.** It holds today only because
  the feature is dev-dependency-only *and* the workspace uses resolver 2;
  `scripts/check-storage-test-support.mjs` resolves the app's real feature graph
  rather than trusting either fact to stay true.

`tests/public_boundary.rs` is compiled as a separate crate against the built
library, so it can only reach `pub` paths with default features. Its compiling
is the assertion: the production API is self-sufficient for someone outside this
crate.
