# tine-storage

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
