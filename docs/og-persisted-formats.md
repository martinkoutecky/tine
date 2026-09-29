# og persisted-format census (batch 5b)

The pinned count is **25 durable layouts** in `scripts/lib/og-enforcement.mjs`.
Several rows share a low-level writer. A format means a byte layout or durable
directory convention, not each JSON key or filename. Temporary files used for
atomic publication have the same payload as their final name.

| Format | Where it is written | Writer evidence |
|---|---|---|
| Page Markdown | configured pages and journals directories, `.md` | `crates/tine-store/src/transaction.rs:403`, `:739`; `crates/tine-store/src/atomic_file.rs:34` |
| Page Org | configured pages and journals directories, `.org` | `crates/tine-store/src/transaction.rs:403`, `:739`; `crates/tine-core/src/org.rs:210` |
| Graph configuration EDN | `logseq/config.edn` | `crates/tine-store/src/store.rs:837`, `:844`; `crates/tine-graph-features/src/config.rs:35`, `:193` |
| Graph stylesheet | `logseq/custom.css` | `crates/tine-store/src/store.rs:878` (graph seed); `crates/tine-graph-features/src/config.rs:51` reads it |
| Graph assets | configured assets directory, original binary bytes | `crates/tine-graph-features/src/assets.rs:133`, `:137`, `:150`; `crates/tine-store/src/model.rs:4771` |
| Asset sidecar EDN | assets `*.edn`, including PDF metadata | `crates/tine-graph-features/src/pdf.rs:299`, `:340` |
| Asset trash | `logseq/.tine-trash/assets/` | `crates/tine-graph-features/src/assets.rs:205`; `crates/tine-store/src/store.rs:1635` |
| Graph trash | `logseq/.tine-trash/`, retired page/config bytes | `crates/tine-store/src/transaction.rs:545`; `crates/tine-store/src/model.rs:5163` |
| Device settings JSON | app data `tine-settings.json` | `src-tauri/src/settings.rs:18`, `:84`; `src-tauri/src/device_io.rs:149` |
| Graph session JSON | app data `sessions/<graph-id>.json` (legacy `tine-session.json`) | `src-tauri/src/settings.rs:329`, `:347`, `:526`, `:544` |
| Workspace registry JSON | app data `sessions/<graph-id>-workspaces.json` | `src-tauri/src/settings.rs:361`, `:425`, `:442`, `:479` |
| Backup page copy | app data backup snapshot, original Markdown/Org bytes | `src-tauri/src/backup.rs:234`, `:434` |
| Backup configuration copy | snapshot `logseq/config.edn` | `src-tauri/src/backup.rs:621` |
| Backup asset copy | snapshot assets and sidecars, original bytes | `src-tauri/src/backup.rs:362`, `:434` |
| Backup manifest JSON | snapshot `snapshot.json` | `src-tauri/src/backup.rs:147`, `:191`, `:268` |
| PDF highlights EDN | PDF `*.edn` sidecar and generated `hls__` notes page | `crates/tine-graph-features/src/pdf.rs:299`, `:340`, `:378`, `:398`; `src-tauri/src/commands.rs:2498` |
| Published site | export HTML/CSS/assets under publish destination | `crates/tine-store/src/publish.rs:329`, `:337`; `crates/tine-graph-features/src/publish.rs:38` |
| Restore recovery | retired files under `logseq/.tine-trash/<id>` and `assets/.tine-restore-recovery/<id>` | `crates/tine-store/src/restore.rs:97`, `:260`, `:265`, `:518`, `:589` |
| Plugin package | app data package `manifest.json` and `plugin.wasm` | `src-tauri/src/plugins.rs:420`, `:422` |
| Desktop launcher | Linux icon and `.desktop` entry | `src-tauri/src/linux_window_identity.rs:80`, `:138`, `:147` |
| Debug log | optional `tine-debug.log` or `TINE_DEBUG_LOG` | `src-tauri/src/debug.rs:27`, `:36`, `:66` |
| Diagnostic history JSONL | app data `diagnostics/history.jsonl`, fixed-shape events, ≤ 1 MiB (ADR 0058) | `src-tauri/src/flight_store.rs` `write_history` |
| Diagnostic session marker | app data `diagnostics/session-active` and `diagnostics/process.lock`, empty files (ADR 0058) | `src-tauri/src/flight_store.rs` `set_session_active`, `open` |
| Diagnostic report JSON | a user-chosen file from Settings → Help & diagnostics → Save report (ADR 0058) | `src-tauri/src/flight_store.rs` `save_report` |
| Concord base ledger | app data `concord-ledger/<graph-id>/`: per page `pages/<sha(path)>/index.json` + ≤ 2 text blobs, per sync copy `pins/<sha(path)>.{json,blob}`; disposable, never under the graph root (ADR 0056) | `src-tauri/src/concord_ledger.rs` `LedgerFiles::write` (via `device_io::atomic_write`) |

The graph session JSON may carry `workspaceId`, the ID of the workspace that
produced it. On startup, a matching live session is fresher than the registry's
parked snapshot. If the session is missing or its `workspaceId` differs from the
registry's `activeId`, the registry's active workspace snapshot wins only when
no live route or session intent changed since the session read. An intervening
live edit wins instead, and the skipped recovery is reported to the user. This
resolves a crash after the registry switch was published but before its
scheduled session save without replacing newer live work.

There is **no separate retained-draft format** on this og tree. Unsaved editor
state is not a durable draft capsule; that is inventory family 9, status todo.
Likewise, restore recovery contains the original file bytes, not a new syntax.

The count test pins the vocabulary and compares low-level writer-site counts
against `2d0349368` to catch uncensused new writes. A caller may still route a
new name through an existing generic writer, so review of store entry points
remains necessary. New formats require an ADR and Martin's approval under
OG-RULES Rule 8; their writer sites are listed in `APPROVED_WRITER_SITES`.

The experiment build's one-time config seed (`src-tauri/src/experiment_config_seed.rs`,
temporary, `docs/app-identity.md`) adds **no format**. It copies census files
byte for byte (device settings, graph sessions, workspace registry, plugin packages, the
webview's own store) from the released Tine's app-data dir into the experiment's. Its
writer sites are approved in `APPROVED_WRITER_SITES`. The same document classifies each
app-data entry the released Tine writes as read as-is or master-only.
