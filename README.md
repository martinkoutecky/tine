# tine-storage

`tine-storage` owns physical persistence mechanisms. The dependency direction
is `src-tauri -> tine-core -> tine-storage`: core supplies policy, authority,
validation, and domain meaning, while this crate supplies storage operations.
It has no dependency on `tine-core`, `lsdoc`, Tauri, or UI crates.

SQLite is a disposable local projection. The oplog/archive is authoritative,
so recovery can rebuild SQLite rather than treating it as a source of truth.
Consumers use the curated `tine_storage::sqlite` facade; it exposes typed
physical operations without raw connections or DDL construction details.

Package-local test ownership is intentionally divided as follows:

- Persistent-format invariants: `durable_batch::tests` and `digest_sealed::tests`.
- Durability and filesystem publication invariants: `filesystem::tests`.
- Authenticated-index invariants: `authenticated_patricia::tests`.
- Scratch lifecycle and retained-run invariants: `scratch::tests`.
- SQLite transaction and schema invariants: `sqlite_frontier::tests` and
  `sqlite_materialization::tests`.
- SQLite facade, connection ownership, and test-support-gate invariants:
  `sqlite_database::tests`.
