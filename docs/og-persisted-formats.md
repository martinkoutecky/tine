# og persisted-format census (batch 5b)

The pinned count is **30 durable layouts** in `scripts/lib/og-enforcement.mjs`.
Several rows share a low-level writer. A format means a byte layout or durable
directory convention, not each JSON key or filename. Temporary files used for
atomic publication have the same payload as their final name.

| Format | Where it is written | Writer evidence |
|---|---|---|
| Page Markdown | configured pages and journals directories, `.md` | `crates/tine-store/src/transaction.rs:403`, `:739`; `crates/tine-store/src/atomic_file.rs:34` |
| Page Org | configured pages and journals directories, `.org` | `crates/tine-store/src/transaction.rs:403`, `:739`; `crates/tine-core/src/org.rs:210` |
| Graph configuration EDN | `logseq/config.edn` | `crates/tine-store/src/store.rs:837`, `:844`; `crates/tine-graph-features/src/config.rs:35`, `:193` |
| Graph link identity | `logseq/tine-graph-id`, one UUID + LF (37 ASCII bytes), created only on explicit Copy link | `crates/tine-store/src/link_identity.rs::ensure_link_identity`, through the existing guarded `Transaction::create` (ADR 0071; Martin 2026-10-04) |
| Graph stylesheet | `logseq/custom.css` | `crates/tine-store/src/store.rs:878` (graph seed); `crates/tine-graph-features/src/config.rs:51` reads it |
| Graph assets | configured assets directory, original binary bytes | `crates/tine-graph-features/src/assets.rs:133`, `:137`, `:150`; `crates/tine-store/src/model.rs:4771` |
| Asset sidecar EDN | assets `*.edn`, including PDF metadata | `crates/tine-graph-features/src/pdf.rs:299`, `:340` |
| Asset trash | `logseq/.tine-trash/assets/` | `crates/tine-graph-features/src/assets.rs:205`; `crates/tine-store/src/store.rs:1635` |
| Graph trash | `logseq/.tine-trash/`, retired page/config bytes; entries are `<stamp>__[<reason>__]<name>`, a long name's stem cut to fit 255 bytes (`atomic_file.rs::prefixed_name`, C3Y) | `crates/tine-store/src/transaction.rs::trash_id`, `transaction/move_file.rs::publish_move` (case-only temporary recovery); `crates/tine-store/src/model.rs:5163` |
| Device settings JSON | app data `tine-settings.json` | `src-tauri/src/settings.rs:18`, `:84`; `src-tauri/src/device_io.rs:149` |
| Graph session JSON | app data `sessions/<graph-id>.json` (legacy `tine-session.json`) | `src-tauri/src/settings.rs:329`, `:347`, `:526`, `:544` |
| Workspace registry JSON | app data `sessions/<graph-id>-workspaces.json` | `src-tauri/src/settings.rs:361`, `:425`, `:442`, `:479` |
| Backup page copy | schema-2 snapshot `journals/`, `pages/` (configured roots only), original Markdown/Org bytes; no longer written since og-B, still listed and restored | `src-tauri/src/backup/restore.rs` `open_verified_restore_files` (read) |
| Backup configuration copy | snapshot `logseq/config.edn` | `src-tauri/src/backup.rs` `write_snapshot` → `write_payload` |
| Backup asset copy | snapshot assets `*.edn` sidecars, original bytes | `src-tauri/src/backup.rs` `copy_store_area(Area::Assets)` → `write_payload` |
| Backup manifest JSON | snapshot `snapshot.json`: schema 3 with `graph_text_policy` {version 2, `hidden`, `hidden_parse_failed_closed`} and `writer: "og"` (master's wire format; schema 2 still read) (ADR 0062) | `src-tauri/src/backup.rs` `write_manifest`, `read_manifest` |
| Backup graph-text copy | schema-3 snapshot `graph/<graph-relative path>`: every file in the graph-text scope (`Area::Graph`), original Markdown/Org bytes (ADR 0062) | `src-tauri/src/backup.rs` `copy_store_area(Area::Graph)` → `write_payload` |
| PDF highlights EDN | PDF `*.edn` sidecar and generated `hls__` notes page | `crates/tine-graph-features/src/pdf.rs:299`, `:340`, `:378`, `:398`; `src-tauri/src/commands.rs:2498` |
| Published site | export HTML/CSS/assets under publish destination | `crates/tine-store/src/publish.rs:329`, `:337`; `crates/tine-graph-features/src/publish.rs:38` |
| Restore recovery | retired files under `logseq/.tine-trash/<id>` (schema-3 whole-graph text under `<id>/graph/<graph-relative path>`) and `assets/.tine-restore-recovery/<id>` | `crates/tine-store/src/restore.rs:97`, `:565` (`reserve`), `:672` (retirement rename), `:691` (copy fallback), `:752` (`retire_extras`) |
| Plugin package | app data package `manifest.json` and `plugin.wasm` | `src-tauri/src/plugins.rs:420`, `:422` |
| Desktop launcher | Linux icon and `.desktop` entry | `src-tauri/src/linux_window_identity.rs:80`, `:138`, `:147` |
| Debug log | optional `tine-debug.log` or `TINE_DEBUG_LOG` | `src-tauri/src/debug.rs:27`, `:36`, `:66` |
| Diagnostic history JSONL | app data `diagnostics/history.jsonl`, fixed-shape events, ≤ 1 MiB (ADR 0058) | `src-tauri/src/flight_store.rs` `write_history` |
| Diagnostic session marker | app data `diagnostics/session-active` and `diagnostics/process.lock`, empty files (ADR 0058) | `src-tauri/src/flight_store.rs` `set_session_active`, `open` |
| Diagnostic report JSON | a user-chosen file from Settings → Help & diagnostics → Save report (ADR 0058) | `src-tauri/src/flight_store.rs` `save_report` |
| Concord base ledger | app data `concord-ledger-og/<graph-id>/` (never master's `concord-ledger/`, whose layout differs; og never reads, prunes or writes it): per page `pages/<sha(path)>/index.json` + ≤ 2 text blobs, per sync copy `pins/<sha(path)>.{json,blob}`; disposable, never under the graph root (ADR 0056) | `src-tauri/src/concord_ledger.rs` `LedgerFiles::write` (via `device_io::atomic_write`) |
| Draft store JSON | app data `drafts/<graph-id>.v1.json`, unsaved drafts of pages that could not be saved, ≤ 64 records and 8 MiB, enforced on read (a file past either bound is set aside) and on write (ADR 0061) | `src-tauri/src/drafts.rs` `write_unlocked`, `bounded_records` |
| Launch checkpoint | app data `launch-checkpoints/<graph-id>.bin`: magic `TINECKPT`, format version, a postcard header (lsdoc tag, graph root, config key of the build-read settings, lengths, SHA-256) and a zstd postcard dump of the whole published generation with per-file stamps; disposable, never under the graph root, any mismatch or damage means a full build (ADR 0070) | `crates/tine-store/src/store/checkpoint.rs` `Publisher::write_once` (via `atomic_file::atomic_write_with_check`) |
| Share inbox | app-owned `share-inbox/` (iOS App Group container, Android `filesDir`), never under a graph root: per item `<id>/item.json` + resource files (native producers, published by one directory rename from `.tmp-<id>/`) and `<id>/prepared.json` (bound graph, frozen day, shaped block, imported assets, arm state); removed after ingest, unreadable items kept as `.rejected-<id>/`; Android producer also keeps `filesDir/share-state/<fingerprint>` publication records (pending/published/abandoned, pruned 30 days after settling) (ADR 0073; Martin 2026-10-10 (Rule 8 new persisted format: share-inbox)) | `src-tauri/src/share_inbox.rs` `prepare` (via `device_io::atomic_write`); producers `src-tauri/ios-app/Shared/ShareInbox.swift`, Android `ShareIntake.kt` (`InboxWriter`, `ShareState`) via `NativeIntegrationsPlugin.kt` |

Opening a graph PDF reads its existing primary or active legacy sidecar without
creating, rewriting or moving graph files. The first highlight or annotation
creates the necessary sidecar and notes page through the existing guarded writer.
Reader page and zoom changes use the existing pane routes in the graph session
JSON, rather than writing the PDF sidecar; existing sidecar view state still loads.

The graph session JSON may carry `workspaceId`, the ID of the workspace that
produced it. On startup, a matching live session is fresher than the registry's
parked snapshot. If the session is missing or its `workspaceId` differs from the
registry's `activeId`, the registry's active workspace snapshot wins only when
no live route or session intent changed since the session read. An intervening
live edit wins instead, and the skipped recovery is reported to the user. This
resolves a crash after the registry switch was published but before its
scheduled session save without replacing newer live work.

The draft store holds a page's editor draft only while that page's edits cannot
be saved (a conflict or a failed save); an ordinary save never writes it. Its
`live-conflict` record kind is the Concord live-draft capsule (og 21a): the
same record plus `base_rev` / `observed_rev`, a record in this store rather
than a second store, with the envelope still `version: 1` (no new format; ADR
0061 amendment).
Restore recovery contains the original file bytes, not a new syntax.

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

Graph link identity costs one tiny file per graph written once, zero bytes/files per ordinary 1- or 60-block edit and zero transport. Opening and browsing only read it; a copied graph shares it, with a device-local choice remembered in the existing settings JSON. Existing malformed identity is preserved and reported.
