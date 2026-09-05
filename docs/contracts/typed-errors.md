# Typed backend errors

Every Tauri command and helper under `src-tauri/src` now rejects with
`CommandError`; the phase-B migration removed the remaining `String` error
boundaries. `CommandError` deliberately serializes as the same JSON string
value each command emitted before the migration. A tagged
error is therefore still a string whose contents are a fixed-shape JSON object:

```json
{"kind":"managed-actor-refusal","reason_code":"trusted_local.append_outcome_unknown"}
```

`kind` is a bounded code. `reason_code` is present for managed-actor refusals,
Direct save failures, and Direct save conflicts. Payloads never carry note text
or wording intended for display. Three kinds carry typed `detail` objects.
`shared-frontier-mismatch` includes the mismatch counts and at most 32
relative note paths (each `local-only`, `shared-only`, or `changed` with its
categories) plus an `omitted` count. `direct-save-failure` carries only the
`io_error_kind`; `save-conflict` carries that kind plus `epoch` (a non-negative
integer or `null` when no override authority exists). This is because
`docs/storage-sync-contract.md`
promises the joining user exactly that list to reconcile a refused join. The
funnel validates the detail field by field and degrades a malformed detail to
`null`; the frontend still owns every displayed word.

`TauriBackend.call` is the only frontend classification point. It converts a
recognized payload into one of the 10 BackendError subclasses (including the
existing `SaveConflictError`). Components branch with `instanceof`; the
frontend message table owns user-visible wording. Unknown and malformed
rejections keep their pre-existing generic error behavior.

## Direct save failures

The Direct producer retains `DirectSaveError` inside the public `io::Error`
surface. `DirectSaveFailureCode` and the optional conflict epoch are typed
fields; the source error is display-only. `direct_save_failure_code` and
`direct_save_conflict_epoch` downcast that inner value and never inspect
`io::Error::to_string()`.

The old whole-string `conflict` / `conflict:<epoch>` wire is retired. Ordinary
failures now use:

```json
{"kind":"direct-save-failure","reason_code":"precheck.symlink","detail":{"io_error_kind":"InvalidInput"}}
```

Banner-class conflicts use the existing tagged kind:

```json
{"kind":"save-conflict","reason_code":"conflict.pinned_owner","detail":{"io_error_kind":"AlreadyExists","epoch":17}}
```

| Variant | Stable string | Disposition | Producing stage |
| --- | --- | --- | --- |
| `PrecheckSymlink` | `precheck.symlink` | no retry | no-follow inventory |
| `PrecheckInterrupted` | `precheck.interrupted` | retry | coherent capture |
| `PrecheckPortableCollision` | `precheck.portable_collision` | no retry | portable-name admission |
| `PrecheckResourceAlias` | `precheck.resource_alias` | no retry | physical-resource admission |
| `PrecheckNotPortable` | `precheck.not_portable` | no retry | managed-path admission |
| `PrecheckNofollow` | `precheck.nofollow` | no retry | retained-directory admission |
| `PrecheckLimit` | `precheck.limit` | no retry | bounded inventory |
| `IdentityChangedSinceLoad` | `identity.changed_since_load` | retry | retained loaded identity |
| `IdentityOwnedElsewhere` | `identity.owned_elsewhere` | no retry | semantic owner check |
| `IdentityNameTaken` | `identity.name_taken` | no retry | rename/create identity |
| `ConflictRetrySaveBaselinePresent` | `conflict_retry.save_baseline_present` | retry | tokenless present baseline |
| `ConflictRetrySaveBaselineAbsent` | `conflict_retry.save_baseline_absent` | retry | tokenless absent baseline |
| `ConflictRetryCommitRecheck` | `conflict_retry.commit_recheck` | retry | tokenless commit recheck |
| `ConflictRetryReplacePreRetirement` | `conflict_retry.replace_pre_retirement` | retry | tokenless replace pre-retire |
| `ConflictRetryReplaceRetiredMismatch` | `conflict_retry.replace_retired_mismatch` | retry | tokenless retired recheck |
| `ConflictRetryReplacePublicationCollision` | `conflict_retry.replace_publication_collision` | retry | tokenless replace publish |
| `ConflictRetryCreatePublicationCollision` | `conflict_retry.create_publication_collision` | retry | tokenless create publish |
| `ConflictRetryFinalRereadAbsent` | `conflict_retry.final_reread_absent` | retry | tokenless final absent read |
| `ConflictRetryFinalRereadPresent` | `conflict_retry.final_reread_present` | retry | tokenless final present read |
| `ConflictRetryReplacePostPublication` | `conflict_retry.replace_post_publication` | retry | tokenless post-publish validation |
| `ConflictAuthoritySuperseded` | `conflict_authority.superseded` | re-observe | override epoch check |
| `ConflictAuthorityOtherEpisode` | `conflict_authority.other_episode` | re-observe | editor-episode check |
| `ConflictAuthoritySpent` | `conflict_authority.spent` | re-observe | one-shot authority check |
| `ConflictSaveBaselinePresent` | `conflict.save_baseline_present` | banner | present baseline observation |
| `ConflictSaveBaselineAbsent` | `conflict.save_baseline_absent` | banner | absent baseline observation |
| `ConflictCommitRecheck` | `conflict.commit_recheck` | banner | commit recheck |
| `ConflictReplacePreRetirement` | `conflict.replace_pre_retirement` | banner | replace pre-retire |
| `ConflictReplaceRetiredMismatch` | `conflict.replace_retired_mismatch` | banner | retired recheck |
| `ConflictReplacePublicationCollision` | `conflict.replace_publication_collision` | banner | replace publication |
| `ConflictCreatePublicationCollision` | `conflict.create_publication_collision` | banner | create publication |
| `ConflictFinalRereadAbsent` | `conflict.final_reread_absent` | banner | final absent read |
| `ConflictFinalRereadPresent` | `conflict.final_reread_present` | banner | final present read |
| `ConflictReplacePostPublication` | `conflict.replace_post_publication` | banner | post-publish validation |
| `ConflictPinnedOwner` | `conflict.pinned_owner` | banner | exact pinned owner |
| `ConflictBaseRev` | `conflict.base_rev` | banner | base revision |
| `Unknown` | `unknown` | retry | unclassified source failure |

The frontend save-policy vocabulary is pinned to the union of these 36 strings
and the 22 `SyncEditorRefusalCode` strings. The pre-existing
`managed.conflict` tagged prefix is the single documented exception: it remains
owned by the Managed producer and is cut to E2/E2b rather than being folded into
the Direct enum.

The managed half of that generated union is:

```text
trusted_local.missing_base_revision
trusted_local.preparation.bindings
trusted_local.preparation.planning
trusted_local.preparation.draft
trusted_local.preparation.capture
trusted_local.preparation.finalize
trusted_local.preparation.publication
trusted_local.preparation.archive_stage
trusted_local.preparation.sqlite_drain
trusted_local.preparation.projection_drain
trusted_local.engine_authority
trusted_local.commit.invalid_prepared_input
trusted_local.commit.managed_record
trusted_local.commit.precommit_graph
trusted_local.commit.append_refused
trusted_local.append_outcome_unknown
fallback.readmission
post_commit.current_page_lookup
trusted_outcome.declined
managed_queue.sequence_overflow
managed_queue.monotonicity
managed_record.decode
```

Scenario I/O errors retain `std::io::ErrorKind` as
`ScenarioError::Io(ErrorKind)`, never a formatted source error. Panic flight
records contain only source location, thread name, and a content-free payload
type class (`message_kind`).

The plugin-visible boundary is separate and unchanged. Plugin workers still
reply with `{ id, ok: false, error: string }`, and the host still exposes that
as `PluginRuntimeError`; JSON-tagged application errors are not sent to guests.

## `CommandError` boundary

The variants are `Tagged`, `Coded`, `Io`, `Worker`, `Tauri`, `Json`, `Plugin`,
`Clipboard`, `Platform`, `GraphVerification`, `Graph`, `SyncRuntime`,
`Settings`, `Diagnostic`, `Backup`, `Core`, and `Prose`. Custom `Serialize`
always calls `serialize_str`; an object-valued IPC
wire is a future product decision, not part of wave 4. `From<String>` and
`From<&str>` are intentionally absent. `Worker` is constructed only at
`spawn_blocking(...).await` joins; other `tauri::Error` values use `Tauri`.
`DirectSaveError` never reaches the generic `Io` conversion because doing so
would discard its closed reason code and conflict epoch.

### Conversion table

| Source type | Production mapper | Variant | Format family | Owned producer symbols |
| --- | --- | --- | --- | --- |
| `DirectSaveError` | `direct_save_error_message` | `Tagged` | tagged reason + detail | `direct_save_error_message` conflict and failure branches |
| clean shutdown refusal | `CommandError::tagged` | `Tagged` | tagged kind | `close_graph_window` |
| `std::io::Error` | `CommandError::from` | `Io` | source display | filesystem/`Graph` calls in `commands.rs` and `state.rs` |
| `tauri::Error` from blocking join | `CommandError::worker` | `Worker` | source display | every `spawn_blocking(...).await` site in `commands.rs` |
| other `tauri::Error` | `CommandError::from` | `Tauri` | source display | path/window/platform calls in `commands.rs` |
| `SyncApplicationPageRequestError`, `SyncEditorRequestError`, `SyncLocalMutationRequestError`, `SyncRuntimeRequestError`, `FastCommitError`, `MergeRefused` | `CommandError::from` | `Core` | typed display; page request retains its tagged backend wire | managed request and merge helpers in `commands.rs` |
| bounded code + detail | `CommandError::coded` | `Coded` | `{code}: {detail}` | query/result budgets, base64 decoding, retained-page conversion, canonical path context |
| `serde_json::Error` | `CommandError::from` / `CommandError::json` when legacy context is retained | `Json` | source display or unchanged contextual display | JSON producers in `conflict_capsule.rs`, `graph_verification.rs`, `plugins.rs`, `settings.rs`, and `sync_runtime.rs` |
| plugin package/registry source | `CommandError::plugin` | `Plugin` | unchanged source/context display | plugin validation, recovery, publication, and retirement helpers |
| clipboard provider source | `CommandError::clipboard` | `Clipboard` | unchanged source display | `read_clipboard_files`, `copy_image_to_clipboard` |
| platform/host source | `CommandError::platform` | `Platform` | unchanged source/context display | platform opener and capture-host helpers |
| graph-verification source | `CommandError::graph_verification` | `GraphVerification` | unchanged source/context display | verification task, dialog, and report helpers (non-JSON) |
| graph lifecycle source | `CommandError::graph` | `Graph` | unchanged source/context display | `graph.rs` and `watcher.rs` lifecycle helpers |
| managed lifecycle source | `CommandError::sync_runtime` | `SyncRuntime` | unchanged source/context display | `storage_mode_supervisor.rs` and `sync_runtime.rs` helpers |
| settings source | `CommandError::settings` | `Settings` | unchanged source/context display | settings validation/load helpers |
| diagnostic source | `CommandError::diagnostic` | `Diagnostic` | unchanged source/context display | diagnostic recorder and dialog helpers |
| backup source | `CommandError::backup` | `Backup` | unchanged source/context display | backup validation and restore helpers |
| literal/context-only remainder | `CommandError::prose` | `Prose` | unchanged prose | symbols in the census below |

The source manifest in `command_error::tests` records `targets`, `file`,
`enclosingSymbol`, `sourceErrorType`, `requiredVariant`, `productionMapper`,
`formatFamily`, `legacyWireTemplate`, and `goldenTest`. Every format family has
a producer-coupled golden in
`command_error::tests::phase_a_production_wire_matches_legacy` and
`command_error::tests::phase_b_production_wire_matches_legacy`; the structural
guard expands every `CommandError` mapper occurrence into a
file/enclosing-symbol/mapper site row, equality-pins the 498-row manifest and
its placement fingerprint, rejects stale registration/manifest rows, and
rejects a typed source routed to `Prose`. The family rows supply source type,
required variant, format family, legacy wire template, and producer-coupled
golden for those site rows.

### Absolute phase-B rule

Every fallible command registered for desktop, Android, or iOS returns
`Result<_, CommandError>`. Every helper under `src-tauri/src` is subject to the
same error boundary: `Result<_, String>` and the temporary
`map_err(|error| error.to_string())` bridge are both strict zero, with no
third-party exception rows. `Worker` is used only immediately after
`spawn_blocking(...).await`; non-worker Tauri errors remain `Tauri`.

### Conversions inside a platform `cfg` body

`CommandError` implements `From` for `std::io::Error`, `tauri::Error`,
`serde_json::Error` and the typed `Sync*` request errors, and for nothing else.
Two native error types you will meet only inside a platform `cfg` body are
therefore NOT convertible with `CommandError::from`:

| source | where | mapper |
| --- | --- | --- |
| `PluginInvokeError` (`run_mobile_plugin`) | Android, iOS | `CommandError::platform` |
| `tauri_plugin_opener::Error` (`open_url`) | Windows, Android, iOS | `CommandError::platform` |

`platform` is the family for an OS capability — a folder picker, media capture,
system bars, opening a URL. `plugin` means the **Tine** plugin system; its only
producer is `plugins.rs`. Prefer the family constructor over adding a `From`
impl: `From` widens what inference will silently accept, which is the surface
this contract exists to keep narrow.

**Nothing on a Linux host compiles any of these bodies.** W4-E2b wrote
`CommandError::from` at nine such sites, passed every local gate, and hosted CI
then failed Android with nine errors and Windows with one. The two iOS sites
were not caught by CI at all — no job compiles iOS — only by enumerating all
five shipped targets by hand, the same gap that left `rename_noreplace` without
an iOS arm through v0.10.0. When you touch a `cfg`-gated conversion, enumerate
Linux, Windows, macOS, iOS and Android deliberately, and expect CI to be the
first thing that compiles what you wrote.

Guard: `backend_command_parity::tests::native_platform_calls_convert_through_a_family_constructor`.
Exemplar to imitate: `android_media::call`.

### `Prose` census

The phase-A syntactic census is 116 production sites (95 in `commands.rs`,
21 in `state.rs`; test fixtures excluded). `CommandError::prose` is an identity
adapter when a phase-B helper already returns `CommandError`, so those retained
E2 call sites do not erase the typed variant. Phase B adds the placement rows
below. Other rows have no typed source: they are literal state assertions,
bounded domain wording, or wrong-reply/deferred outcomes.

**The census only shrinks for a fixed managed-command surface.** It is a
retirement ratchet on untyped *legacy* wording, not a cap on how many managed
commands may exist. Every command routed through `sparse_navigation` must match
its own reply variant and reject the others, so each new managed command adds
exactly one wrong-reply arm — the category this table already records as having
no typed source. Typing those arms would need a new error family, which
CLOSURE §4 of the 2026-09 query campaign explicitly rejects; the twenty-plus
existing arms are the idiom they follow.

113 → 116 (2026-09-05, P0-rust Wave B, `08da2595`): `query_registry`,
`query_run` and `query_explain_empty` each gained one
`"managed navigation returned the wrong reply"` arm. No legacy prose site was
retained, reintroduced, or converted back from a typed variant; the retirement
owners below are unchanged. A future growth entry belongs in this paragraph with
the same three facts: which commands, which arms, and what did *not* regress.

| File | Enclosing symbols | Legacy template | Why no typed source exists | Retirement owner |
| --- | --- | --- | --- | --- |
| `state.rs` | `require_legacy_authority`, `legacy_graph`, `wait_for_legacy_drain`, `bind`, `replace_if_current` | authority/lease/binding literal or bounded contextual message | local state predicate, not a source error | typed state domain follow-up |
| `state.rs` | `owned_graph_context`, `canonical_graph_root`, `slot_for_window`, `slot_for_bound_window`, `capture_quick_switch_slot`, `refresh_graph_for_label` | missing/stale/bound-window/canonical-path literal | local state predicate or E2b bridge | W4-E2b |
| `commands.rs` | `load_workspaces`, `save_workspaces`, `sparse_application_handle`, `prepare_tine_quit_all_slots`, `open_page_file` | unchanged helper display | E2 compatibility adapter; typed phase-B errors pass through unchanged | phase-A adapter cleanup |
| `commands.rs` | `map_managed_graph_mutation`, `map_managed_sync_conflict_resolution`, `sparse_navigation`, `map_sparse_page_inventory`, `map_sparse_page_load`, `map_sparse_page_save` | conflict/deferred/refusal wording | enum outcome has no error value in that arm | managed outcome taxonomy follow-up |
| `commands.rs` | `referenced_page_names`, `journal_feed_page`, `move_managed_application_subtrees`, `acknowledge_managed_application_move`, `recover_managed_application_subtrees`, `copy_guide_into_bound_graph` | deferred/unsupported literal | semantic outcome or local precondition | managed outcome taxonomy follow-up |
| `commands.rs` | `get_backlinks`, `get_backlink_filter_context`, `get_unlinked_refs`, `block_ref_counts`, `block_referrers`, `run_query`, `export_query_subtrees`, `run_graph_search`, `run_advanced_query`, `query_facets`, `page_aliases`, `page_icons`, `existing_page_names`, `search`, `quick_switch`, `list_templates`, `resolve_block`, `resolve_blocks`, `preview_block`, `list_orphan_assets` | bounded-size or wrong-reply wording | bounded/local semantic refusal | query outcome taxonomy follow-up |
| `commands.rs` | `publish_html`, `page_print_html`, `open_pdf`, `write_highlights`, `write_pdf_view_state` | deferred or `no-page` literal | semantic outcome without a source error | managed outcome taxonomy follow-up |
| `commands.rs` | `decode_asset_b64`, `tine_quit`, `read_local_image`, `import_native_capture`, `delimited_ext`, `read_text_file`, `open_asset`, `edit_asset_external`, `build_editor_argv` | local validation/platform literal | validation branch has no source error | validation taxonomy follow-up |
| `commands.rs` | `managed_capsule_current`, `conflict_capsule_diff`, `resolve_conflict_capsule`, `resolve_sync_conflict`, `merge_pages` | authority missing/changed literal | local state predicate | conflict taxonomy follow-up |
| `backup.rs` | `collect`, `collect_scoped_restore_graph_text`, `restore_backup`, `restore_from_backup_source` | backup path/schema/safety literal | local validation branch, not a source error | backup outcome taxonomy follow-up |
| `conflict_capsule.rs` | `capsule_page_name`, `decode_envelope`, `quarantine_unreadable`, `reclaim_torn_temps`, `retire_at`, `write_unlocked` | capsule validation literal | local validation branch, not a source error | capsule outcome taxonomy follow-up |
| `debug.rs` | `clear_diagnostics`, `save_diagnostic_report` | recorder/destination literal | local availability or validation branch | diagnostic outcome taxonomy follow-up |
| `graph.rs` | `approve_external_assets`, `capture_graph_binding`, `capture_target_for_state`, `create_graph`, `inspect_graph_access`, `load_graph`, `load_graph_for_label`, `open_graph_window`, `refuse_unclaimed_sparse_archive_with` | graph state/selection literal | local predicate, not a source error | graph outcome taxonomy follow-up |
| `graph_verification.rs` | `cancel_graph_verification`, `create_graph_verification`, `save_graph_verification_report` | operation/registry/destination literal | local predicate, not a source error | verification outcome taxonomy follow-up |
| `lib.rs`, `android_folder_picker.rs`, `android_managed_storage_smoke.rs` | `capture_frontend_ready`, `pick_graph_folder`, `Java_page_tine_app_ManagedStorageSmoke_runManagedActivationSmoke` | platform availability/JNI validation literal | local platform predicate | platform outcome taxonomy follow-up |
| `android_media.rs` | `$name` in the non-Android `android_media_command` template | unsupported-platform literal | cfg-split macro command has no source error | platform outcome taxonomy follow-up |
| `platform.rs` | `external_open_plan`, `open_external`, `reveal_page_source` | unsupported URL/platform/path literal | local validation branch | platform outcome taxonomy follow-up |
| `plugins.rs` | `install_plugin`, `install_plugin_package_at`, `manifest_identity`, `package_dir`, `plugins_dir`, `read_plugin_entry`, `set_plugin_enabled`, `set_plugin_enabled_at`, `store_plugin_registry_cache`, `store_plugin_registry_cache_at`, `uninstall_package`, `validate_uninstall_target`, `verify_plugin_registry` | plugin identity/bounds/availability literal | local validation branch, not a source error | plugin outcome taxonomy follow-up |
| `settings.rs` | `atomic_write_workspaces`, `load_session`, `load_workspaces`, `managed_sync_device_id`, `managed_sync_device_id_at`, `migrate_legacy_session_at`, `reveal_known_graph`, `save_session`, `save_session_at`, `save_workspaces`, `update_settings`, `validate_workspaces_json` | settings shape/availability literal | local validation branch, not a source error | settings outcome taxonomy follow-up |
| `storage_mode_supervisor.rs` | `commit_if_current` | superseded-transition literal | local state predicate | transition outcome taxonomy follow-up |
| `sync_runtime.rs` | `activate_sparse_v2`, `activate_sparse_v2_blocking`, `adopt_sparse_v2_shared_blocking`, `archive_graph_provider_namespace_with`, `archive_private_root`, `blank_slate_recovery_key`, `cancel_sparse_v2_at_paths_with_archive_and_publish`, `cancel_sparse_v2_blocking`, `cancel_sparse_v2_cold`, `join_runtime_failure`, `join_sparse_v2_shared_blocking`, `move_recovery_result`, `prepare_sparse_v2_activation`, `prepare_sparse_v2_share_blocking`, `prove_managed_application_ready`, `recover_managed_application_subtrees_with`, `replace_failed_blank_slate_candidate`, `run_android_managed_return_to_direct_files`, `set_aside_managed_history_for_adoption`, `validate_for` | managed lifecycle/state literal | local predicate or enum outcome without a source error | managed outcome taxonomy follow-up |

## Core-only clean-open boundary

`CleanOpenError` is the one core taxonomy for the 16 error classes reachable
from clean managed construction and recovery. It is `pub(crate)`: several
source types are crate-private, and wave 4 deliberately keeps
`SyncRuntimeOpenStatus::OpenRefused { detail: String }` as the Tauri-facing
boundary. The sole projection into that string is tagged JSON with
`kind: "clean-open"` and the stable `reason_code` below. Source display text,
paths, and note names do not enter the payload. The frontend DTO bridge and its
current parsing of open-status detail are follow-up work; this packet adds no
frontend subclass.

| Variant | Source class | `reason_code` | Refusal scenario |
| --- | --- | --- | --- |
| `BootstrapStreamingImport` | `BootstrapStreamingImportError` | `clean_open.bootstrap_streaming_import` | malformed or over-bound inbound source (`MS-REF-MALFORMED-IMPORT`, `MS-REF-BOUNDS`) |
| `Engine` | `EngineError` | `clean_open.engine` | invalid retained causal/archive state (`MS-REF-DISK-CORRUPT`) |
| `Enrollment` | `EnrollmentError` | `clean_open.enrollment` | damaged, incompatible, contended, or unsafe enrollment authority (`MS-REF-DISK-CORRUPT`, `MS-REF-PROTOCOL-INCOMPATIBLE`, `MS-REF-CONCURRENT-WRITER`, `MS-REF-UNSAFE-FS-KIND`) |
| `ManagedLocalRecord` | `ManagedLocalRecordError` | `clean_open.managed_local_record` | damaged foreground journal record (`MS-REF-DISK-CORRUPT`) |
| `ProjectionStore` | `ProjectionStoreError` | `clean_open.projection_store` | damaged, incompatible, or unsafe receipt state (`MS-REF-DISK-CORRUPT`, `MS-REF-PROTOCOL-INCOMPATIBLE`, `MS-REF-UNSAFE-FS-KIND`) |
| `ProjectionTurnJournal` | `ProjectionTurnJournalError` | `clean_open.projection_turn_journal` | damaged, contended, or unsafe turn journal (`MS-REF-DISK-CORRUPT`, `MS-REF-CONCURRENT-WRITER`, `MS-REF-UNSAFE-FS-KIND`) |
| `Receipt` | `ReceiptError` | `clean_open.receipt` | malformed or incompatible retained receipt (`MS-REF-DISK-CORRUPT`, `MS-REF-PROTOCOL-INCOMPATIBLE`) |
| `RuntimePromotion` | `RuntimePromotionError` | `clean_open.runtime_promotion` | stale or unavailable runtime authority (`MS-REF-STALE-GENERATION`, `MS-REF-CONCURRENT-WRITER`) |
| `Scenario` | `ScenarioError` | `clean_open.scenario` | malformed, conflicting, or unsafe provider input (`MS-REF-MALFORMED-IMPORT`, `MS-REF-SYNC-CONFLICT`, `MS-REF-UNSAFE-FS-KIND`) |
| `Store` | `StoreError` | `clean_open.store` | damaged, incompatible, or unsafe operation archive (`MS-REF-DISK-CORRUPT`, `MS-REF-PROTOCOL-INCOMPATIBLE`, `MS-REF-UNSAFE-FS-KIND`) |
| `Sweep` | `SweepError` | `clean_open.sweep` | damaged retained disposition state (`MS-REF-DISK-CORRUPT`) |
| `WorkspaceAuthority` | `WorkspaceAuthorityRefusal` | `clean_open.workspace_authority` | another instance or a stale lease identity owns the workspace (`MS-REF-CONCURRENT-WRITER`, `MS-REF-STALE-GENERATION`) |
| `SqliteProjection` | `oplog::sqlite::ProjectionError` | `clean_open.sqlite_projection` | contended, stale, or damaged disposable projection open (`MS-REF-CONCURRENT-WRITER`, `MS-REF-STALE-GENERATION`, with rebuild for cache-only damage) |
| `Projection` | `oplog::projection::ProjectionError` | `clean_open.projection` | malformed source or stale projection authority (`MS-REF-MALFORMED-IMPORT`, `MS-REF-STALE-GENERATION`) |
| `Io` | `std::io::Error` (retained as `ErrorKind`) | `clean_open.io` | transient storage unavailability is retryable; unsafe or damaged authority uses the corresponding §3.1 scenario |
| `Batch` | `tine_storage::BatchError<CoreDurableBatchContract>` | `clean_open.batch` | malformed or damaged durable batch (`MS-REF-MALFORMED-IMPORT`, `MS-REF-DISK-CORRUPT`) |

Reachability is a source-class × boundary property, not a Cartesian promise.
`yes` means that boundary calls a producer of the class; `—` means it does not.
The three shared-inspection entry points are grouped because they all call the
same `ScenarioError` provider primitives.

| Source class | shared provider/descriptor inspection | local activation | existing managed reopen | retained actor adoption/reopen |
| --- | --- | --- | --- | --- |
| `BootstrapStreamingImportError` | — | yes | — | — |
| `EngineError` | — | yes | yes | yes |
| `EnrollmentError` | — | yes | yes | yes |
| `ManagedLocalRecordError` | — | yes | yes | — |
| `ProjectionStoreError` | — | yes | yes | — |
| `ProjectionTurnJournalError` | — | yes | yes | yes |
| `ReceiptError` | — | yes | yes | — |
| `RuntimePromotionError` | — | yes | yes | yes |
| `ScenarioError` | yes | yes | yes | yes |
| `StoreError` | — | yes | yes | yes |
| `SweepError` | — | yes | yes | yes |
| `WorkspaceAuthorityRefusal` | — | yes | yes | — |
| `oplog::sqlite::ProjectionError` | — | yes | yes | yes |
| `oplog::projection::ProjectionError` | — | yes | yes | — |
| `std::io::Error` | — | yes | yes | yes |
| `tine_storage::BatchError<CoreDurableBatchContract>` | — | yes | yes | — |
