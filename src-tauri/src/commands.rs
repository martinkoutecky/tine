use crate::command_error::CommandError;
#[cfg(desktop)]
use crate::debug::diag;
#[cfg(desktop)]
use crate::platform::{open_page_source, opener_command, reveal_page_source};
use crate::state::{
    capture_quick_switch_slot, owned_graph_context, refresh_graph, slot_for_bound_window,
    slot_for_context, with_config_graph, with_filesystem_graph, with_trash_graph, AppState,
    ApplicationPageAdmissionAuthority, GraphContext,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Instant;
use tauri::{Emitter, Manager, State, WebviewWindow};
use tine_core::date::JournalDate;
use tine_core::journal_feed::{
    collect_journal_feed_page, journal_feed_candidate_in_window, journal_feed_inventory,
};
use tine_core::model::{
    BacklinkFilterContext, BacklinkFilterTarget, BlockDto, PageDto, PageEntry, PageKind, RefGroup,
};
use tine_core::sync_runtime::{
    SyncApplicationGraphMutationRequest, SyncApplicationGuideCopyOutcome,
    SyncApplicationJournalFeedOutcome, SyncApplicationJournalFeedRequest,
    SyncApplicationMoveSubtreesOutcome, SyncApplicationMoveSubtreesRequest,
    SyncApplicationNavigationOutcome, SyncApplicationNavigationReply,
    SyncApplicationNavigationRequest, SyncApplicationPageInventoryOutcome,
    SyncApplicationPageLoadOutcome, SyncApplicationPageLoadRequest, SyncApplicationPageSaveOutcome,
    SyncApplicationPageSaveRequest, SyncApplicationPageSaveTarget, SyncApplicationPageSelector,
    SyncApplicationPdfOpenOutcome, SyncApplicationPublishOutcome, SyncApplicationUnitOutcome,
    SyncRuntimeHandle,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct ManagedApplicationMoveSubtreesResult {
    pub(crate) binding_generation: u64,
    pub(crate) application_page_admission: crate::state::ApplicationPageAdmission,
    pub(crate) outcome: SyncApplicationMoveSubtreesOutcome,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct ManagedApplicationMoveSubtreesRecoveryResult {
    pub(crate) previous_binding_generation: u64,
    pub(crate) binding_generation: u64,
    pub(crate) status: crate::sync_runtime::SparseV2StatusDto,
    pub(crate) application_page_admission: crate::state::ApplicationPageAdmission,
    pub(crate) episode_id: String,
    pub(crate) outcome: SyncApplicationMoveSubtreesOutcome,
}

#[cfg(test)]
mod managed_application_move_wire_tests {
    use super::{
        ManagedApplicationMoveSubtreesRecoveryResult, ManagedApplicationMoveSubtreesResult,
    };
    use crate::state::{ApplicationPageAdmission, ApplicationPageAdmissionAuthority};
    use tine_core::model::{Format, PageDto, PageKind};
    use tine_core::sync_runtime::{
        SyncApplicationMoveConflict, SyncApplicationMoveSubtreesOutcome, SyncApplicationMovedPage,
        SyncEditorDeferred, SyncLocalMutationPhase,
    };

    #[test]
    fn bounded_tauri_move_result_json_round_trips() {
        let page = |name: &str| PageDto {
            activation: None,
            name: name.into(),
            kind: PageKind::Page,
            title: name.into(),
            pre_block: None,
            blocks: Vec::new(),
            rev: None,
            format: Format::Md,
            read_only: false,
            path: format!("pages/{name}.md"),
            guide: false,
        };
        let source = page("Source");
        let destination = page("Destination");
        let episode_id = "019d2e53-3cf0-7a31-a19b-1bdf47b7d3a1";
        let admission = ApplicationPageAdmission {
            binding_generation: 17,
            authority: ApplicationPageAdmissionAuthority::ManagedWritable {
                application_save_page_blocks: 511,
                application_page_request_text_bytes: 1_048_576,
                application_page_max_depth: 128,
            },
        };
        let outcomes = vec![
            SyncApplicationMoveSubtreesOutcome::Committed {
                episode_id: episode_id.into(),
                batch_id: "019d2e53-3cf0-7a31-a19b-1bdf47b7d3a2".into(),
                recovered: true,
                source: SyncApplicationMovedPage {
                    page: source,
                    revision: "source-revision".into(),
                },
                destination: SyncApplicationMovedPage {
                    page: destination,
                    revision: "destination-revision".into(),
                },
            },
            SyncApplicationMoveSubtreesOutcome::NoCommit {
                episode_id: episode_id.into(),
                reason: SyncApplicationMoveConflict::EpisodeNotCommitted,
            },
            SyncApplicationMoveSubtreesOutcome::Deferred {
                episode_id: episode_id.into(),
                state: SyncEditorDeferred::RetryableExternalWork,
            },
            SyncApplicationMoveSubtreesOutcome::Deferred {
                episode_id: episode_id.into(),
                state: SyncEditorDeferred::RetryableRetainedPublication {
                    batch_id: "019d2e53-3cf0-7a31-a19b-1bdf47b7d3a3".into(),
                    phase: SyncLocalMutationPhase::ArchiveStage,
                },
            },
            SyncApplicationMoveSubtreesOutcome::Deferred {
                episode_id: episode_id.into(),
                state: SyncEditorDeferred::BlockedRecovery {
                    batch_id: None,
                    phase: SyncLocalMutationPhase::ProjectionDrain,
                    retained_publication: true,
                },
            },
            SyncApplicationMoveSubtreesOutcome::Deferred {
                episode_id: episode_id.into(),
                state: SyncEditorDeferred::Revoked {
                    batch_id: None,
                    phase: SyncLocalMutationPhase::Bindings,
                },
            },
        ];
        let status: crate::sync_runtime::SparseV2StatusDto =
            serde_json::from_value(serde_json::json!({
                "state": "active",
                "runtime": {
                    "lifecycle": "active",
                    "recovery": "adopted_safe_handoff",
                    "watcher": {
                        "latest_enqueue": 0,
                        "acknowledged": 0,
                        "drain_in_flight": false,
                        "pending": false,
                        "pending_requires_full_scan": false,
                        "deferred": false,
                        "quiescing": false,
                        "sequence_exhausted": false
                    },
                    "last_tick": null,
                    "detail": null,
                    "shared_role": null,
                    "shared_phase": null,
                    "provider_pending": 0,
                    "provider_runnable": false,
                    "search_index_building": false,
                    "managed_local_pending": 0,
                    "managed_local_checkpointed_sequence": 0,
                    "managed_local_next_sequence": 0,
                    "managed_local_stage": null
                },
                "can_activate": false,
                "can_retry": false,
                "can_cancel": true,
                "cancel_reason": null,
                "binding_generation": 17,
                "application_page_admission": serde_json::to_value(&admission).unwrap()
            }))
            .unwrap();

        for outcome in outcomes {
            let result = ManagedApplicationMoveSubtreesResult {
                binding_generation: 17,
                application_page_admission: admission.clone(),
                outcome: outcome.clone(),
            };
            let bytes = serde_json::to_vec(&result).unwrap();
            assert!(bytes.len() < 16 * 1024);
            let decoded: ManagedApplicationMoveSubtreesResult =
                serde_json::from_slice(&bytes).unwrap();
            assert_eq!(
                serde_json::to_value(decoded).unwrap(),
                serde_json::to_value(result).unwrap()
            );

            let recovery = ManagedApplicationMoveSubtreesRecoveryResult {
                previous_binding_generation: 16,
                binding_generation: 17,
                status: status.clone(),
                application_page_admission: admission.clone(),
                episode_id: episode_id.into(),
                outcome,
            };
            let bytes = serde_json::to_vec(&recovery).unwrap();
            let decoded: ManagedApplicationMoveSubtreesRecoveryResult =
                serde_json::from_slice(&bytes).unwrap();
            assert_eq!(
                serde_json::to_value(decoded).unwrap(),
                serde_json::to_value(recovery).unwrap()
            );
        }
    }
}

#[tauri::command]
pub(crate) fn load_workspaces(
    app: tauri::AppHandle,
    state: GraphContext<'_>,
) -> Result<String, CommandError> {
    crate::settings::load_workspaces(app, state).map_err(CommandError::prose)
}

#[tauri::command]
pub(crate) fn save_workspaces(
    data: String,
    app: tauri::AppHandle,
    state: GraphContext<'_>,
) -> Result<(), CommandError> {
    crate::settings::save_workspaces(data, app, state).map_err(CommandError::prose)
}

const RESULT_BRIDGE_MAX_ROWS: usize = 20_000;
const RESULT_BRIDGE_MAX_BYTES: usize = 32 * 1024 * 1024;
const AUTOCOMPLETE_FACET_MAX_ITEMS: usize = 2_000;
const AUTOCOMPLETE_FACET_MAX_BYTES: usize = 2 * 1024 * 1024;
const QUERY_EXPORT_MAX_QUERIES: usize = 64;
const QUERY_EXPORT_REQUEST_MAX_QUERIES: usize = 1_024;
const QUERY_EXPORT_MAX_QUERY_BYTES: usize = 64 * 1024;
const QUERY_EXPORT_MAX_ROOTS: usize = 50;
const QUERY_EXPORT_MAX_NODES: usize = 2_000;
const QUERY_EXPORT_MAX_BYTES: usize = 8 * 1024 * 1024;

fn validate_query_source(query: &str) -> Result<(), CommandError> {
    if !tine_core::query::query_source_within_limit(query) {
        return Err(CommandError::coded(
            "query-too-large",
            format!(
                "query source is {} bytes (limit: {} bytes)",
                query.len(),
                tine_core::query::QUERY_SOURCE_MAX_BYTES
            ),
        ));
    }
    if !tine_core::query::query_nesting_within_limit(query) {
        return Err(CommandError::coded(
            "query-nesting-too-deep",
            "simplify nested boolean clauses",
        ));
    }
    Ok(())
}

fn enforce_result_bridge_budget(groups: &[RefGroup]) -> Result<(), CommandError> {
    let rows = groups.iter().map(|group| group.blocks.len()).sum::<usize>();
    let bytes = tine_core::model::ref_groups_estimated_bytes(groups);
    if rows > RESULT_BRIDGE_MAX_ROWS || bytes > RESULT_BRIDGE_MAX_BYTES {
        return Err(CommandError::coded(
            "result-too-large",
            format!("{rows} matching blocks (~{bytes} bytes); narrow the query or add (sample N) (limits: {RESULT_BRIDGE_MAX_ROWS} blocks / {RESULT_BRIDGE_MAX_BYTES} bytes)"),
        ));
    }
    Ok(())
}

fn enforce_optional_result_bridge_budget(groups: &[Option<RefGroup>]) -> Result<(), CommandError> {
    let rows = groups
        .iter()
        .flatten()
        .map(|group| group.blocks.len())
        .sum::<usize>();
    let bytes = groups
        .iter()
        .flatten()
        .map(|group| tine_core::model::ref_groups_estimated_bytes(std::slice::from_ref(group)))
        .sum::<usize>();
    if rows > RESULT_BRIDGE_MAX_ROWS || bytes > RESULT_BRIDGE_MAX_BYTES {
        return Err(CommandError::coded(
            "result-too-large",
            format!("{rows} matching blocks (~{bytes} bytes); narrow the query or add (sample N) (limits: {RESULT_BRIDGE_MAX_ROWS} blocks / {RESULT_BRIDGE_MAX_BYTES} bytes)"),
        ));
    }
    Ok(())
}

fn bounded_groups_or_error(
    result: tine_core::model::BoundedRefGroups,
) -> Result<Arc<Vec<RefGroup>>, CommandError> {
    if result.exceeded {
        return Err(CommandError::coded(
            "result-too-large",
            format!("{} matching blocks; narrow the query or add (sample N) (construction limits: {RESULT_BRIDGE_MAX_ROWS} blocks / {RESULT_BRIDGE_MAX_BYTES} bytes)", result.total),
        ));
    }
    Ok(result.groups)
}

fn enforce_query_execution_budget(
    execution: &tine_core::query_plan::QueryExecution,
) -> Result<(), CommandError> {
    use tine_core::query_plan::QueryHit;
    let bytes = execution.hits.iter().fold(0usize, |total, hit| {
        total.saturating_add(match hit {
            QueryHit::Page {
                page,
                display_text,
                evidence,
                matched_alias,
                ..
            } => {
                page.name.len()
                    + page.rel_path.len()
                    + display_text.len()
                    + matched_alias.as_ref().map_or(0, String::len)
                    + evidence.len() * 128
                    + 256
            }
            QueryHit::Block {
                page,
                block,
                display_text,
                evidence,
                ..
            } => {
                page.len()
                    + tine_core::model::block_dto_estimated_bytes(block)
                    + display_text.len()
                    + evidence.len() * 128
                    + 256
            }
        })
    });
    if execution.hits.len() > RESULT_BRIDGE_MAX_ROWS || bytes > RESULT_BRIDGE_MAX_BYTES {
        return Err(CommandError::coded(
            "result-too-large",
            format!("{} search hits (~{bytes} bytes); narrow the search (limits: {RESULT_BRIDGE_MAX_ROWS} hits / {RESULT_BRIDGE_MAX_BYTES} bytes)", execution.hits.len()),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod result_bridge_budget_tests {
    use super::{
        enforce_optional_result_bridge_budget, enforce_result_bridge_budget, validate_query_source,
        RESULT_BRIDGE_MAX_BYTES, RESULT_BRIDGE_MAX_ROWS,
    };
    use tine_core::{BlockDto, PageKind, RefGroup};

    fn group(blocks: Vec<BlockDto>) -> RefGroup {
        RefGroup {
            page: "Budget".into(),
            kind: PageKind::Page,
            blocks,
            evidence: Vec::new(),
        }
    }

    #[test]
    fn rejects_oversized_result_count_before_ipc() {
        let groups = [group(vec![BlockDto::default(); RESULT_BRIDGE_MAX_ROWS + 1])];
        assert!(enforce_result_bridge_budget(&groups)
            .unwrap_err()
            .to_string()
            .starts_with("result-too-large:"));
    }

    #[test]
    fn optional_results_are_budgeted_without_cloning_present_groups() {
        let groups = [
            None,
            Some(group(vec![BlockDto::default(); RESULT_BRIDGE_MAX_ROWS + 1])),
        ];
        assert!(enforce_optional_result_bridge_budget(&groups)
            .unwrap_err()
            .to_string()
            .starts_with("result-too-large:"));
    }

    #[test]
    fn rejects_oversized_result_bytes_before_ipc() {
        let mut block = BlockDto::default();
        block.raw = "x".repeat(RESULT_BRIDGE_MAX_BYTES + 1);
        assert!(enforce_result_bridge_budget(&[group(vec![block])])
            .unwrap_err()
            .to_string()
            .starts_with("result-too-large:"));
    }

    #[test]
    fn rejects_oversized_query_source_before_cache_or_parser() {
        let source = "x".repeat(tine_core::query::QUERY_SOURCE_MAX_BYTES + 1);
        assert!(validate_query_source(&source)
            .unwrap_err()
            .to_string()
            .starts_with("query-too-large:"));

        let nested = format!("{}(task TODO){}", "(and ".repeat(65), ")".repeat(65));
        assert!(validate_query_source(&nested)
            .unwrap_err()
            .to_string()
            .starts_with("query-nesting-too-deep:"));
    }
}

/// Write a PNG image to the OS clipboard. The lightbox encodes the shown image to
/// PNG and sends the bytes. On Linux we prefer `wl-copy`/`xclip` (see above) and
/// fall back to the Tauri clipboard plugin; elsewhere the plugin is reliable.
/// Decode a base64 asset payload. The frontend sends bytes as one base64 string
/// rather than a JSON number[] (which inflated the IPC payload ~4-5x and forced a
/// per-element parse + a giant throwaway array on the webview thread).
const ASSET_INGRESS_MAX_BYTES: usize = 64 * 1024 * 1024;

fn decoded_base64_len(input: &str) -> Option<usize> {
    if input.len() % 4 != 0 {
        return None;
    }
    let padding = input
        .as_bytes()
        .iter()
        .rev()
        .take_while(|byte| **byte == b'=')
        .count()
        .min(2);
    input
        .len()
        .checked_div(4)?
        .checked_mul(3)?
        .checked_sub(padding)
}

pub(crate) fn decode_asset_b64(b64: &str) -> Result<Vec<u8>, CommandError> {
    use base64::Engine;
    let max_encoded = ASSET_INGRESS_MAX_BYTES.div_ceil(3) * 4;
    if b64.len() > max_encoded
        || decoded_base64_len(b64).is_some_and(|len| len > ASSET_INGRESS_MAX_BYTES)
    {
        return Err(CommandError::prose(
            "asset payload exceeds 64 MiB ingress limit",
        ));
    }
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|error| CommandError::coded("bad base64 asset payload", error.to_string()))?;
    if decoded.len() > ASSET_INGRESS_MAX_BYTES {
        return Err(CommandError::prose(
            "asset payload exceeds 64 MiB ingress limit",
        ));
    }
    Ok(decoded)
}

#[cfg(test)]
mod asset_ingress_tests {
    use super::{decoded_base64_len, ASSET_INGRESS_MAX_BYTES};

    #[test]
    fn base64_size_gate_accounts_for_padding_before_decode() {
        let encoded = ASSET_INGRESS_MAX_BYTES.div_ceil(3) * 4;
        assert!(encoded / 4 * 3 > ASSET_INGRESS_MAX_BYTES);
        assert_eq!(decoded_base64_len("AAAA"), Some(3));
        assert_eq!(decoded_base64_len("AA=="), Some(1));
        assert_eq!(decoded_base64_len("AAA="), Some(2));
    }
}

fn sparse_application_handle(
    slot: &crate::state::GraphSlot,
) -> Result<Option<&SyncRuntimeHandle>, CommandError> {
    if slot.is_sparse_v2() {
        crate::sync_runtime::active_handle(slot)
            .map(Some)
            .map_err(CommandError::prose)
    } else {
        Ok(None)
    }
}

fn map_managed_graph_mutation(
    outcome: Result<
        SyncApplicationUnitOutcome,
        tine_core::sync_runtime::SyncApplicationPageRequestError,
    >,
) -> Result<(), CommandError> {
    match outcome? {
        SyncApplicationUnitOutcome::Applied => Ok(()),
        SyncApplicationUnitOutcome::Deferred { .. } => Err(CommandError::prose(
            "Tine-managed storage is updating pages. Try the operation again when it finishes.",
        )),
    }
}

fn map_managed_sync_conflict_resolution(
    outcome: Result<
        SyncApplicationUnitOutcome,
        tine_core::sync_runtime::SyncApplicationPageRequestError,
    >,
) -> Result<(), CommandError> {
    match outcome {
        Err(tine_core::sync_runtime::SyncApplicationPageRequestError::ActorRefusedAt(
            "sync_conflict_changed",
        )) => Err(CommandError::prose("conflict")),
        other => map_managed_graph_mutation(other),
    }
}

fn sparse_page_inventory(handle: &SyncRuntimeHandle) -> Result<Vec<PageEntry>, CommandError> {
    let outcome = handle.application_page_inventory()?;
    map_sparse_page_inventory(outcome)
}

fn sparse_navigation(
    handle: &SyncRuntimeHandle,
    request: SyncApplicationNavigationRequest,
) -> Result<SyncApplicationNavigationReply, CommandError> {
    match handle.application_navigation(request)? {
        SyncApplicationNavigationOutcome::Loaded { reply } => Ok(reply),
        SyncApplicationNavigationOutcome::Deferred { state: _ } => Err(CommandError::prose(
            "Tine-managed storage is updating page navigation. Try again when it finishes.",
        )),
    }
}

fn map_sparse_page_inventory(
    outcome: SyncApplicationPageInventoryOutcome,
) -> Result<Vec<PageEntry>, CommandError> {
    match outcome {
        SyncApplicationPageInventoryOutcome::Loaded { pages } => Ok(pages),
        SyncApplicationPageInventoryOutcome::Deferred { state: _ } => Err(CommandError::prose(
            "Tine-managed storage is updating the page list. Try again when it finishes.",
        )),
    }
}

fn map_sparse_page_load(
    outcome: SyncApplicationPageLoadOutcome,
) -> Result<Option<PageDto>, CommandError> {
    match outcome {
        SyncApplicationPageLoadOutcome::Loaded {
            mut page,
            revision,
        } => {
            page.rev = Some(revision);
            Ok(Some(page))
        }
        SyncApplicationPageLoadOutcome::Missing { .. } => Ok(None),
        SyncApplicationPageLoadOutcome::Ambiguous => Err(CommandError::prose(
            "Tine-managed storage could not identify this page. Reload it and resolve any conflicts.",
        )),
        SyncApplicationPageLoadOutcome::Deferred { state: _ } => Err(CommandError::prose(
            "Tine-managed storage is updating this page. Try again when it finishes.",
        )),
    }
}

fn load_sparse_page(
    handle: &SyncRuntimeHandle,
    selector: SyncApplicationPageSelector,
) -> Result<Option<PageDto>, CommandError> {
    let outcome =
        handle.load_application_page(SyncApplicationPageLoadRequest { page: selector })?;
    map_sparse_page_load(outcome)
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ManagedConflictObservation {
    path: String,
    revision: String,
}

/// Save transport result. Direct Files includes the activation at its resolved
/// target when an absent editor successfully becomes present. Managed storage
/// preserves its existing revision semantics and returns no activation.
#[derive(Serialize)]
pub(crate) struct SavePageResult {
    revision: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    activation: Option<tine_core::EditorActivationHandle>,
}

fn sparse_save_request(
    page: PageDto,
    base_rev: Option<String>,
    force: bool,
    managed_conflict_observation: Option<ManagedConflictObservation>,
) -> Result<SyncApplicationPageSaveRequest, CommandError> {
    let target = match (force, base_rev) {
        (true, _) => {
            let observation = managed_conflict_observation.ok_or_else(|| {
                CommandError::coded(
                    "managed.conflict_unobserved",
                    "Keep mine needs an identifiable current managed page. Use current or wait for the page to become identifiable.",
                )
            })?;
            SyncApplicationPageSaveTarget::ResolveConflict {
                path: observation.path,
                observed_revision: observation.revision,
            }
        }
        (false, Some(revision)) => SyncApplicationPageSaveTarget::Existing {
            path: page.path.clone(),
            revision,
        },
        (false, None) => SyncApplicationPageSaveTarget::New {
            name: page.name.clone(),
            page_kind: page.kind.into(),
        },
    };
    Ok(SyncApplicationPageSaveRequest { target, page })
}

fn map_sparse_page_save(outcome: SyncApplicationPageSaveOutcome) -> Result<String, CommandError> {
    match outcome {
        SyncApplicationPageSaveOutcome::Prepared => Err(CommandError::prose(
            "managed preflight result escaped the preflight command",
        )),
        SyncApplicationPageSaveOutcome::Saved { revision, .. }
        | SyncApplicationPageSaveOutcome::Unchanged { revision, .. } => Ok(revision),
        // This bounded family tells the frontend to retain the draft, observe
        // the exact current managed page through the actor, and raise the same
        // explicit resolution surface as Direct Files. No revision is embedded
        // in this error: the follow-up exact-path load is the observation, and
        // the actor re-proves its revision in the replacement turn.
        SyncApplicationPageSaveOutcome::Conflict { reason } => Err(CommandError::coded(
            "managed.conflict",
            format!("this page changed in Tine-managed storage ({reason:?})"),
        )),
        SyncApplicationPageSaveOutcome::Deferred { state: _ } => Err(CommandError::prose(
            "Tine-managed storage is updating this page. Try saving again when it finishes.",
        )),
    }
}

fn save_sparse_page_with(
    page: PageDto,
    base_rev: Option<String>,
    force: bool,
    managed_conflict_observation: Option<ManagedConflictObservation>,
    save: impl FnOnce(
        SyncApplicationPageSaveRequest,
    ) -> Result<
        SyncApplicationPageSaveOutcome,
        tine_core::sync_runtime::SyncApplicationPageRequestError,
    >,
) -> Result<String, CommandError> {
    let request = sparse_save_request(page, base_rev, force, managed_conflict_observation)?;
    let outcome = save(request)?;
    map_sparse_page_save(outcome)
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum ManagedPageMutationPreflightResult {
    Accepted {
        binding_generation: u64,
        page_name: String,
        page_path: String,
        base_revision: Option<String>,
    },
    Refused,
    Deferred,
}

fn managed_preflight_binding_admitted(
    owned_binding: u64,
    requested_binding: u64,
    authority: &ApplicationPageAdmissionAuthority,
) -> bool {
    owned_binding == requested_binding
        && matches!(
            authority,
            ApplicationPageAdmissionAuthority::ManagedWritable { .. }
        )
}

/// Prepare the exact managed application-page transaction and discard it. This
/// command never falls back to Direct Files and never settles actor work.
#[tauri::command]
pub(crate) async fn preflight_managed_page_mutation(
    page: PageDto,
    base_revision: Option<String>,
    binding_generation: u64,
    state: GraphContext<'_>,
) -> Result<ManagedPageMutationPreflightResult, CommandError> {
    let (app, label, owned_binding) = owned_graph_context(state)?;
    if owned_binding != binding_generation {
        return Ok(ManagedPageMutationPreflightResult::Refused);
    }
    let page_name = page.name.clone();
    let page_path = page.path.clone();
    let echoed_base = base_revision.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        if !managed_preflight_binding_admitted(
            slot.binding_generation,
            binding_generation,
            &slot.application_page_admission().authority,
        ) {
            return Ok(ManagedPageMutationPreflightResult::Refused);
        }
        let Some(handle) = sparse_application_handle(&slot)? else {
            return Ok(ManagedPageMutationPreflightResult::Refused);
        };
        let request = match sparse_save_request(page, base_revision, false, None) {
            Ok(request) => request,
            Err(_) => return Ok(ManagedPageMutationPreflightResult::Refused),
        };
        match handle.preflight_application_page(request) {
            Ok(tine_core::sync_runtime::SyncApplicationPagePreflightOutcome::Accepted) => {
                Ok(ManagedPageMutationPreflightResult::Accepted {
                    binding_generation,
                    page_name,
                    page_path,
                    base_revision: echoed_base,
                })
            }
            Ok(tine_core::sync_runtime::SyncApplicationPagePreflightOutcome::Deferred {
                ..
            }) => Ok(ManagedPageMutationPreflightResult::Deferred),
            Ok(tine_core::sync_runtime::SyncApplicationPagePreflightOutcome::Conflict {
                ..
            })
            | Err(_) => Ok(ManagedPageMutationPreflightResult::Refused),
        }
    })
    .await
    .map_err(CommandError::worker)?
}

#[cfg(test)]
mod managed_page_mutation_preflight_tests {
    use super::{managed_preflight_binding_admitted, ManagedPageMutationPreflightResult};
    use crate::state::ApplicationPageAdmissionAuthority;

    #[test]
    fn binding_and_authority_gate_refuses_mismatch_direct_and_unavailable() {
        let writable = ApplicationPageAdmissionAuthority::ManagedWritable {
            application_save_page_blocks: 511,
            application_page_request_text_bytes: 1024 * 1024,
            application_page_max_depth: 128,
        };
        assert!(managed_preflight_binding_admitted(7, 7, &writable));
        assert!(!managed_preflight_binding_admitted(7, 8, &writable));
        assert!(!managed_preflight_binding_admitted(
            7,
            7,
            &ApplicationPageAdmissionAuthority::Direct
        ));
        assert!(!managed_preflight_binding_admitted(
            7,
            7,
            &ApplicationPageAdmissionAuthority::ManagedUnavailable
        ));
    }

    #[test]
    fn every_preflight_result_round_trips_exact_json() {
        for result in [
            ManagedPageMutationPreflightResult::Accepted {
                binding_generation: 7,
                page_name: "Dense".into(),
                page_path: "pages/Dense.md".into(),
                base_revision: Some("base".into()),
            },
            ManagedPageMutationPreflightResult::Refused,
            ManagedPageMutationPreflightResult::Deferred,
        ] {
            let json = serde_json::to_string(&result).unwrap();
            assert_eq!(
                serde_json::from_str::<ManagedPageMutationPreflightResult>(&json).unwrap(),
                result
            );
        }
    }
}

/// Keep the user-facing save error bounded, while allowing an opt-in local
/// diagnostic trace to carry the exact core refusal that led to it.  The core
/// only constructs this detail under TINE_DEBUG/--debug; this helper is a
/// second gate before the text reaches the debug log.
fn managed_save_debug_detail_line(
    error: &tine_core::sync_runtime::SyncApplicationPageRequestError,
) -> Option<String> {
    error
        .debug_detail()
        .map(|detail| format!("managed storage save refusal detail: {detail}"))
}

#[tauri::command]
pub(crate) async fn list_pages(state: GraphContext<'_>) -> Result<Vec<PageEntry>, CommandError> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        match sparse_application_handle(&slot)? {
            Some(handle) => sparse_page_inventory(handle),
            None => Ok(slot.legacy_graph()?.list_pages()),
        }
    })
    .await
    .map_err(CommandError::worker)?
}

#[tauri::command]
pub(crate) async fn referenced_page_names(
    known_digest: Option<u64>,
    state: GraphContext<'_>,
) -> Result<tine_core::ReferencedPageNames, CommandError> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        match sparse_application_handle(&slot)? {
            Some(handle) => match sparse_navigation(
                handle,
                SyncApplicationNavigationRequest::ReferencedPageNames { known_digest },
            )? {
                SyncApplicationNavigationReply::ReferencedPageNames(answer) => Ok(answer),
                _ => Err(CommandError::prose(
                    "managed navigation returned the wrong reply",
                )),
            },
            None => Ok(slot
                .legacy_graph()?
                .referenced_page_names_versioned(known_digest)),
        }
    })
    .await
    .map_err(CommandError::worker)?
}

#[derive(Serialize)]
pub(crate) struct JournalFeedPage {
    pages: Vec<PageDto>,
    next_before_day: Option<i64>,
    done: bool,
    as_of_day: i64,
}

/// The Journals feed.
///
/// Managed storage answers the whole request in ONE actor turn: the runtime
/// keeps the graph's journal days indexed per accepted frontier, so a feed
/// open costs a window lookup plus `limit` page loads instead of the complete
/// page inventory it used to walk on every open and every scroll step
/// (managed-storage cost-model audit 2026-08-26, D6). Direct Files selects the
/// same window from its warmed page cache through the same
/// `tine_core::journal_feed` rules.
#[tauri::command]
pub(crate) async fn journal_feed_page(
    limit: usize,
    before_day: Option<i64>,
    state: GraphContext<'_>,
) -> Result<JournalFeedPage, CommandError> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        let as_of_day = JournalDate::today().ordinal_key();
        match sparse_application_handle(&slot)? {
            Some(handle) => {
                match handle.journal_feed_page(SyncApplicationJournalFeedRequest {
                    limit,
                    before_day,
                    as_of_day,
                })? {
                    SyncApplicationJournalFeedOutcome::Loaded {
                        pages,
                        next_before_day,
                        done,
                    } => Ok(JournalFeedPage {
                        pages,
                        next_before_day,
                        done,
                        as_of_day,
                    }),
                    SyncApplicationJournalFeedOutcome::Ambiguous => Err(CommandError::prose(
                        "Tine-managed storage could not identify this page. Reload it and resolve any conflicts.",
                    )),
                    SyncApplicationJournalFeedOutcome::Deferred { state: _ } => {
                        Err(CommandError::prose(
                            "Tine-managed storage is updating the page list. Try again when it finishes.",
                        ))
                    }
                }
            }
            None => {
                let graph = slot.legacy_graph()?;
                let entries =
                    graph.feed_journals_desc_through(JournalDate::from_ordinal(as_of_day));
                let selection = collect_journal_feed_page(
                    entries.into_iter().filter(|entry| {
                        journal_feed_candidate_in_window(entry, as_of_day, before_day)
                    }),
                    limit,
                    // A journal deleted from disk between selection and load is
                    // skipped, but its day still advances the cursor.
                    |entry| graph.load_page(entry),
                )
                .map_err(CommandError::prose)?;
                Ok(JournalFeedPage {
                    pages: selection.pages,
                    next_before_day: selection.next_before_day,
                    done: selection.done,
                    as_of_day,
                })
            }
        }
    })
    .await
    .map_err(CommandError::worker)?
}

#[tauri::command]
pub(crate) async fn get_page(
    name: String,
    kind: PageKind,
    state: GraphContext<'_>,
) -> Result<Option<PageDto>, CommandError> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        match sparse_application_handle(&slot)? {
            Some(handle) => load_sparse_page(
                handle,
                SyncApplicationPageSelector::Logical {
                    name,
                    page_kind: kind.into(),
                },
            ),
            None => slot
                .legacy_graph()?
                .load_named(&name, kind)
                .map_err(CommandError::from),
        }
    })
    .await
    .map_err(CommandError::worker)?
}

/// One raw source file of the open graph, for the in-app lsdoc↔mldoc diff panel.
#[derive(serde::Serialize)]
pub(crate) struct GraphSourceFile {
    /// graph-root-relative, forward-slashed path (stable id shown in the report)
    rel: String,
    /// the file's raw UTF-8 text (fed to both parsers exactly as on disk)
    text: String,
    /// "md" | "org" — selects the parser grammar
    format: String,
    bytes: u64,
}

/// Raw text of every Markdown/Org file in the open graph (`pages/`, plus
/// `journals/` when `include_journals`), for the "Help improve Tine" diff panel.
/// Mirrors `lsdoc/tools/graph-check.mjs`'s file scan: skips files over 8 MB, tags
/// format by extension, returns graph-root-relative paths sorted for stable
/// output. Read-only and local — the panel makes no network calls.
#[tauri::command]
pub(crate) fn graph_source_files(
    include_journals: bool,
    state: GraphContext<'_>,
) -> Result<Vec<GraphSourceFile>, CommandError> {
    const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;
    with_filesystem_graph(&state, |g| {
        let mut out: Vec<GraphSourceFile> = Vec::new();
        let mut roots = vec![g.pages_path()];
        if include_journals {
            roots.push(g.journals_path());
        }
        for root in roots {
            collect_graph_text(g, &root, MAX_FILE_BYTES, &mut out);
        }
        out.sort_by(|a, b| a.rel.cmp(&b.rel));
        Ok(out)
    })
}

fn collect_graph_text(
    g: &tine_core::model::Graph,
    dir: &std::path::Path,
    max_bytes: u64,
    out: &mut Vec<GraphSourceFile>,
) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect_graph_text(g, &p, max_bytes, out);
            continue;
        }
        let format = match p.extension().and_then(|x| x.to_str()) {
            Some("md") => "md",
            Some("org") => "org",
            _ => continue,
        };
        let Ok(meta) = std::fs::metadata(&p) else {
            continue;
        };
        if meta.len() > max_bytes {
            continue; // oversized files skipped, like graph-check
        }
        let Ok(text) = std::fs::read_to_string(&p) else {
            continue; // non-UTF-8 / unreadable file skipped
        };
        out.push(GraphSourceFile {
            rel: g.rel_path(&p),
            text,
            format: format.to_string(),
            bytes: meta.len(),
        });
    }
}

/// A Direct-Markdown save is slow enough to be worth a line above this. Chosen
/// so ordinary saves stay silent (0.6.5 saved in single-digit milliseconds) while
/// anything a user would notice as a hitch is on the record.
const DIRECT_SAVE_DIAGNOSTIC_THRESHOLD_MS: u128 = 150;

/// Turn a failed Direct save into a message the frontend can act on.
///
/// The old mapping collapsed EVERY `AlreadyExists` to the literal string
/// "conflict", and the frontend recognised a conflict by testing whether the
/// message contained that substring. So a portable-filename collision, a
/// physical-resource alias, or "another document owns this page identity" all
/// raised the content-conflict prompt — whose two buttons are "Keep mine
/// (overwrite)" and "Use disk version", neither of which can resolve any of
/// them. Choosing either left the page marked conflicted, and a conflicted page
/// silently refuses to save from then on.
///
/// Only a real base-revision conflict gets the "conflict" contract now. Every
/// other failure carries its bounded code as a stable prefix, so the frontend
/// can classify it without sniffing prose and the user gets an error they can
/// read instead of a prompt that cannot help.
pub(crate) fn direct_save_error_message(error: std::io::Error) -> CommandError {
    let error = tine_core::model::DirectSaveError::ensure_io(error);
    let code = tine_core::model::direct_save_failure_code(&error);
    let io_error_kind = format!("{:?}", error.kind());
    if code.starts_with("conflict.") {
        return CommandError::tagged(
            "save-conflict",
            Some(code),
            Some(serde_json::json!({
                "io_error_kind": io_error_kind,
                "epoch": tine_core::model::direct_save_conflict_epoch(&error),
            })),
        );
    }
    CommandError::tagged(
        "direct-save-failure",
        Some(code),
        Some(serde_json::json!({ "io_error_kind": io_error_kind })),
    )
}

/// Report what a slow or failed Direct-Markdown save actually did.
///
/// Always on. Every field is a duration, a count, or a bounded failure code --
/// no page names, no paths, no content -- so the line is safe to paste into a
/// public issue, which is the only way it helps the people reporting #266/#267
/// from Windows machines we cannot reproduce.
///
/// The counters are the load-bearing part. `builds` distinguishes "the save was
/// slow" from "the save rebuilt a whole-graph index to answer a filename
/// question", and those have opposite fixes.
fn report_direct_save_diagnostics(
    graph: &tine_core::model::Graph,
    elapsed: std::time::Duration,
    error: Option<&std::io::Error>,
) {
    if error.is_none() && elapsed.as_millis() < DIRECT_SAVE_DIAGNOSTIC_THRESHOLD_MS {
        return;
    }
    let report = graph.guarded_graph_text_identity_report();
    let outcome = match error {
        Some(error) => tine_core::model::direct_save_failure_code(error),
        None => "ok",
    };
    crate::debug::record_direct_save(
        outcome,
        u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX),
        u64::try_from(report.complete_builds).unwrap_or(u64::MAX),
        u64::try_from(report.exact_updates).unwrap_or(u64::MAX),
        report.invalidated,
        report
            .last_build
            .as_ref()
            .map(|build| u64::try_from(build.captured_entries).unwrap_or(u64::MAX)),
        report
            .last_build
            .as_ref()
            .map(|build| u64::try_from(build.captured_bytes).unwrap_or(u64::MAX)),
    );
    let build = report.last_build.map_or_else(
        || " last_build=none".to_string(),
        |build| {
            format!(
                " last_build_capture_ms={} last_build_index_ms={} last_build_parsed={} last_build_entries={} last_build_bytes={}",
                build.capture.as_millis(),
                build.index.as_millis(),
                build.decode_semantics,
                build.captured_entries,
                build.captured_bytes,
            )
        },
    );
    crate::debug::diag(format!(
        "direct save: outcome={outcome} total_ms={} guarded_index_builds={} guarded_index_exact_updates={} guarded_index_invalidated={}{build}",
        elapsed.as_millis(),
        report.complete_builds,
        report.exact_updates,
        report.invalidated,
    ));
}

#[tauri::command]
pub(crate) async fn save_page(
    page: PageDto,
    base_rev: Option<String>,
    force: Option<bool>,
    // Which conflict observation a forced save is answering. Required for a
    // force; a request that cannot name one is refused rather than allowed to
    // consume whatever authority happens to be current (GH #254 increment 2,
    // adversarial implementation verification, finding 1).
    conflict_epoch: Option<u64>,
    // Exact managed owner observed after a conflict refusal. Direct Files
    // ignores this; managed Keep mine cannot proceed without both path and rev.
    managed_conflict_observation: Option<ManagedConflictObservation>,
    state: GraphContext<'_>,
) -> Result<SavePageResult, CommandError> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let benchmark_started = std::env::var_os("TINE_ISSUE248_BENCH").map(|_| Instant::now());
        let result = {
            let state = app.state::<AppState>();
            let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
            match sparse_application_handle(&slot)? {
                Some(handle) => {
                    let result = save_sparse_page_with(
                        page,
                        base_rev,
                        force.unwrap_or(false),
                        managed_conflict_observation,
                        |request| {
                            let saved = handle.save_application_page(request);
                            if crate::debug::debug_enabled() {
                                if let Err(error) = &saved {
                                    if let Some(line) = managed_save_debug_detail_line(error) {
                                        crate::debug::diag(line);
                                    }
                                }
                            }
                            saved
                        },
                    );
                    // A successful managed save has already made its exact user
                    // projection durable, but archive/checkpoint derivatives are
                    // intentionally drained by actor ticks. Wake the watcher even
                    // when the OS coalesces Tine's own file event; otherwise that
                    // derivative queue can remain pending until unrelated I/O.
                    crate::state::poke_watcher(&state);
                    result.map(|revision| SavePageResult {
                        revision,
                        activation: None,
                    })
                }
                None => {
                    let graph = slot.legacy_graph()?;
                    let first_save_activation = base_rev
                        .is_none()
                        .then_some(page.activation)
                        .flatten()
                        .map(tine_core::EditorActivation::from_u64);
                    // Always timed, not just under the issue-248 benchmark env
                    // var. A save that takes minutes is the thing users report,
                    // and a measurement that only exists when someone thought to
                    // set an environment variable beforehand is not available at
                    // the moment it is needed.
                    let started = Instant::now();
                    let result = if force.unwrap_or(false) {
                        match conflict_epoch {
                            Some(observation_epoch) => graph.force_save_page_at_revision(
                                &page,
                                base_rev.as_deref(),
                                tine_core::ConflictOverride { observation_epoch },
                            ),
                            None => Err(tine_core::model::DirectSaveError::into_io(
                                tine_core::model::DirectSaveFailureCode::ConflictAuthoritySpent,
                                std::io::Error::new(
                                    std::io::ErrorKind::PermissionDenied,
                                    "conflict override authority is missing or already consumed",
                                ),
                            )),
                        }
                    } else {
                        graph.save_page(&page, base_rev.as_deref())
                    };
                    let elapsed = started.elapsed();
                    if benchmark_started.is_some() {
                        let _ = app.emit_to(
                            &label,
                            "issue-248-legacy-save-page-ms",
                            elapsed.as_secs_f64() * 1_000.0,
                        );
                    }
                    report_direct_save_diagnostics(&graph, elapsed, result.as_ref().err());
                    result.map_err(direct_save_error_message).map(|revision| {
                        let activation = first_save_activation.and_then(|activation| {
                            graph.finish_saved_editor_activation(activation)
                        });
                        SavePageResult {
                            revision,
                            activation,
                        }
                    })
                }
            }
        };
        if let Some(started) = benchmark_started {
            let _ = app.emit_to(
                &label,
                "issue-248-backend-save-ms",
                started.elapsed().as_secs_f64() * 1_000.0,
            );
        }
        result
    })
    .await
    .map_err(CommandError::worker)?
}

/// X1-only native bridge for one actor-owned managed cross-page move. No
/// production gesture calls this command until X2 installs the queue, busy
/// lease, save hold, authoritative DTO publication, and semantic history.
#[tauri::command]
pub(crate) async fn move_managed_application_subtrees(
    binding_generation: u64,
    request: SyncApplicationMoveSubtreesRequest,
    state: GraphContext<'_>,
) -> Result<ManagedApplicationMoveSubtreesResult, CommandError> {
    let (app, label, context_generation) = owned_graph_context(state)?;
    if context_generation != binding_generation {
        return Err(CommandError::prose(
            "managed cross-page move belongs to a stale graph binding",
        ));
    }
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        let handle = sparse_application_handle(&slot)?.ok_or_else(|| {
            CommandError::prose("managed cross-page move requires managed storage")
        })?;
        let outcome = handle.move_application_subtrees(request)?;
        let result = ManagedApplicationMoveSubtreesResult {
            binding_generation,
            application_page_admission: slot.application_page_admission(),
            outcome,
        };
        crate::state::poke_watcher(&state);
        Ok(result)
    })
    .await
    .map_err(CommandError::worker)?
}

/// Retire one committed move's private response-replay evidence after the
/// frontend has installed the authoritative source and destination DTOs.
#[tauri::command]
pub(crate) async fn acknowledge_managed_application_move(
    binding_generation: u64,
    episode_id: String,
    batch_id: String,
    state: GraphContext<'_>,
) -> Result<(), CommandError> {
    let (app, label, context_generation) = owned_graph_context(state)?;
    if context_generation != binding_generation {
        return Err(CommandError::prose(
            "managed cross-page move acknowledgement belongs to a stale graph binding",
        ));
    }
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        let handle = sparse_application_handle(&slot)?.ok_or_else(|| {
            CommandError::prose("managed cross-page move acknowledgement requires managed storage")
        })?;
        let result = handle
            .acknowledge_application_move(&episode_id, &batch_id)
            .map_err(CommandError::from);
        // Acknowledgement schedules actor-owned bounded cleanup. Wake the
        // watcher on both success and retryable failure so the cleanup lane is
        // not stranded on an otherwise quiet graph.
        crate::state::poke_watcher(&state);
        result
    })
    .await
    .map_err(CommandError::worker)?
}

/// X1.5 recovery handoff for one exact, already-issued managed move episode.
/// The helper owns graph lifecycle serialization and may replace only an
/// already-stopped retained actor with one recovered successor.
#[tauri::command]
pub(crate) async fn recover_managed_application_subtrees(
    binding_generation: u64,
    request: SyncApplicationMoveSubtreesRequest,
    state: GraphContext<'_>,
) -> Result<ManagedApplicationMoveSubtreesRecoveryResult, CommandError> {
    let (app, label, context_generation) = owned_graph_context(state)?;
    if context_generation != binding_generation {
        return Err(CommandError::prose(
            "managed cross-page move recovery belongs to a stale graph binding",
        ));
    }
    tauri::async_runtime::spawn_blocking(move || {
        crate::sync_runtime::recover_managed_application_subtrees_blocking(
            &app,
            &label,
            binding_generation,
            request,
        )
    })
    .await
    .map_err(CommandError::worker)?
    .map_err(CommandError::prose)
}

#[tauri::command]
pub(crate) fn guide_pages() -> Result<Vec<tine_core::onboarding::GuidePage>, CommandError> {
    tine_core::onboarding::bundled_guide_pages().map_err(CommandError::from)
}

pub(crate) fn copy_guide_into_bound_graph(
    app: &tauri::AppHandle,
    label: &str,
    binding_generation: u64,
    title: String,
) -> Result<tine_core::onboarding::GuideCopyResult, CommandError> {
    let state = app.state::<AppState>();
    let slot = slot_for_bound_window(&state, label, Some(binding_generation))?;
    match sparse_application_handle(&slot)? {
        Some(handle) => match handle.copy_application_guide(title)? {
            SyncApplicationGuideCopyOutcome::Copied { result } => Ok(result),
            SyncApplicationGuideCopyOutcome::Deferred { .. } => Err(CommandError::prose(
                "Tine-managed storage is updating pages. Try copying the guide again when it finishes.",
            )),
        },
        None => {
            let graph = slot.legacy_graph()?;
            tine_core::onboarding::copy_guide_into_graph(&graph, &title)
                .map_err(CommandError::from)
        }
    }
}

#[tauri::command]
pub(crate) async fn copy_guide_into_graph(
    title: String,
    state: GraphContext<'_>,
) -> Result<tine_core::onboarding::GuideCopyResult, CommandError> {
    // managed-command-routing: managed
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        copy_guide_into_bound_graph(&app, &label, binding_generation, title)
    })
    .await
    .map_err(CommandError::worker)?
}

#[tauri::command]
pub(crate) async fn get_backlinks(
    name: String,
    state: GraphContext<'_>,
) -> Result<Arc<Vec<RefGroup>>, CommandError> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        match sparse_application_handle(&slot)? {
            Some(handle) => match sparse_navigation(
                handle,
                SyncApplicationNavigationRequest::Backlinks {
                    name,
                    max_rows: RESULT_BRIDGE_MAX_ROWS,
                    max_bytes: RESULT_BRIDGE_MAX_BYTES,
                },
            )? {
                SyncApplicationNavigationReply::Backlinks(result) => {
                    if result.exceeded {
                        Err(CommandError::prose(format!(
                            "result-too-large: {} matching blocks; narrow the query or add (sample N) (construction limits: {RESULT_BRIDGE_MAX_ROWS} blocks / {RESULT_BRIDGE_MAX_BYTES} bytes)",
                            result.total
                        )))
                    } else {
                        Ok(Arc::new(result.groups))
                    }
                }
                _ => Err(CommandError::prose("managed backlinks returned the wrong reply kind")),
            },
            None => bounded_groups_or_error(slot.legacy_graph()?.backlinks_bounded(
                &name,
                RESULT_BRIDGE_MAX_ROWS,
                RESULT_BRIDGE_MAX_BYTES,
            )),
        }
    })
    .await
    .map_err(CommandError::worker)?
}

#[tauri::command]
pub(crate) async fn get_backlink_filter_context(
    name: String,
    targets: Vec<BacklinkFilterTarget>,
    state: GraphContext<'_>,
) -> Result<BacklinkFilterContext, CommandError> {
    if targets.len() > RESULT_BRIDGE_MAX_ROWS {
        return Err(CommandError::prose(format!(
            "too many backlink filter roots: {} (limit: {RESULT_BRIDGE_MAX_ROWS})",
            targets.len()
        )));
    }
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        match sparse_application_handle(&slot)? {
            Some(handle) => match sparse_navigation(
                handle,
                SyncApplicationNavigationRequest::BacklinkFilterContext { name, targets },
            )? {
                SyncApplicationNavigationReply::BacklinkFilterContext(context) => Ok(context),
                _ => Err(CommandError::prose(
                    "managed backlink filter context returned the wrong reply kind",
                )),
            },
            None => {
                let graph = slot.legacy_graph()?;
                Ok(tine_core::query::backlink_filter_context(
                    &graph, &name, &targets,
                ))
            }
        }
    })
    .await
    .map_err(CommandError::worker)?
}

#[tauri::command]
pub(crate) async fn get_unlinked_refs(
    name: String,
    state: GraphContext<'_>,
) -> Result<Arc<Vec<RefGroup>>, CommandError> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        match sparse_application_handle(&slot)? {
            Some(handle) => match sparse_navigation(
                handle,
                SyncApplicationNavigationRequest::UnlinkedReferences {
                    name,
                    max_rows: RESULT_BRIDGE_MAX_ROWS,
                    max_bytes: RESULT_BRIDGE_MAX_BYTES,
                },
            )? {
                SyncApplicationNavigationReply::UnlinkedReferences(result) => {
                    if result.exceeded {
                        Err(CommandError::prose(format!(
                            "result-too-large: {} matching blocks; narrow the query or add (sample N) (construction limits: {RESULT_BRIDGE_MAX_ROWS} blocks / {RESULT_BRIDGE_MAX_BYTES} bytes)",
                            result.total
                        )))
                    } else {
                        Ok(Arc::new(result.groups))
                    }
                }
                _ => Err(CommandError::prose("managed unlinked references returned the wrong reply kind")),
            },
            None => bounded_groups_or_error(slot.legacy_graph()?.unlinked_refs_bounded(
                &name,
                RESULT_BRIDGE_MAX_ROWS,
                RESULT_BRIDGE_MAX_BYTES,
            )),
        }
    })
    .await
    .map_err(CommandError::worker)?
}

/// `block uuid → # of referrers` over the whole graph (drives the per-block
/// reference-count badge). Small map (only referenced uuids); fetched once per
/// graph generation by the frontend.
#[tauri::command]
pub(crate) async fn block_ref_counts(
    state: GraphContext<'_>,
) -> Result<Arc<std::collections::HashMap<String, usize>>, CommandError> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        match sparse_application_handle(&slot)? {
            Some(handle) => match sparse_navigation(
                handle,
                SyncApplicationNavigationRequest::BlockReferenceCounts,
            )? {
                SyncApplicationNavigationReply::BlockReferenceCounts(counts) => {
                    Ok(Arc::new(counts))
                }
                _ => Err(CommandError::prose(
                    "managed block-reference counts returned the wrong reply kind",
                )),
            },
            None => slot
                .legacy_graph()?
                .block_ref_counts()
                .map_err(CommandError::from),
        }
    })
    .await
    .map_err(CommandError::worker)?
}

/// The blocks that reference block `uuid`, grouped by page (the badge's referrers
/// panel). Lazy: called only when a badge is clicked open.
#[tauri::command]
pub(crate) async fn block_referrers(
    uuid: String,
    state: GraphContext<'_>,
) -> Result<Arc<Vec<RefGroup>>, CommandError> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        match sparse_application_handle(&slot)? {
            Some(handle) => match sparse_navigation(
                handle,
                SyncApplicationNavigationRequest::BlockReferrers {
                    uuid,
                    max_rows: RESULT_BRIDGE_MAX_ROWS,
                    max_bytes: RESULT_BRIDGE_MAX_BYTES,
                },
            )? {
                SyncApplicationNavigationReply::BlockReferrers(result) => {
                    if result.exceeded {
                        Err(CommandError::prose(format!(
                            "result-too-large: {} matching blocks; narrow the query or add (sample N) (construction limits: {RESULT_BRIDGE_MAX_ROWS} blocks / {RESULT_BRIDGE_MAX_BYTES} bytes)",
                            result.total
                        )))
                    } else {
                        Ok(Arc::new(result.groups))
                    }
                }
                _ => Err(CommandError::prose("managed block referrers returned the wrong reply kind")),
            },
            None => bounded_groups_or_error(slot.legacy_graph()?.block_referrers_bounded(
                &uuid,
                RESULT_BRIDGE_MAX_ROWS,
                RESULT_BRIDGE_MAX_BYTES,
            )),
        }
    })
    .await
    .map_err(CommandError::worker)?
}

/// Deleting one page is graph-wide work: it re-derives the page inventory and
/// rebuilds three O(pages) indexes. Measured at ~95 µs/file, linear — 757 ms at
/// 8,006 files. This was the only such command still running on the command
/// thread; its neighbours `get_page`, `save_page` and `rename_page` already
/// cross the blocking pool, and the guard test below simply did not list it.
/// (Direct Files perf audit, 2026-08-09, F3.)
#[tauri::command]
pub(crate) async fn delete_page(
    name: String,
    kind: PageKind,
    expected_path: Option<String>,
    state: GraphContext<'_>,
) -> Result<(), CommandError> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        match sparse_application_handle(&slot)? {
            Some(handle) => map_managed_graph_mutation(handle.mutate_application_graph(
                SyncApplicationGraphMutationRequest::DeletePage {
                    name,
                    page_kind: kind.into(),
                    expected_path,
                },
            )),
            None => slot
                .legacy_graph()?
                .delete_page_expected(&name, kind, expected_path.as_deref())
                .map_err(CommandError::from),
        }
    })
    .await
    .map_err(CommandError::worker)?
}

#[tauri::command]
pub(crate) async fn rename_page(
    old: String,
    new: String,
    expected_path: Option<String>,
    state: GraphContext<'_>,
) -> Result<tine_core::model::RenameOutcome, CommandError> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        match sparse_application_handle(&slot)? {
            // Managed storage has no VCS-marker quarantine (markers are an
            // on-disk Direct Files concept), so it never skips a referrer and
            // the empty report is accurate rather than a stand-in.
            Some(handle) => map_managed_graph_mutation(handle.mutate_application_graph(
                SyncApplicationGraphMutationRequest::RenamePage {
                    old,
                    new,
                    expected_path,
                },
            ))
            .map(|()| tine_core::model::RenameOutcome::default()),
            None => slot
                .legacy_graph()?
                .rename_page_reporting(&old, &new, expected_path.as_deref())
                .map_err(CommandError::from),
        }
    })
    .await
    .map_err(CommandError::worker)?
}

#[cfg(test)]
mod graph_wide_command_boundary_tests {
    #[test]
    fn expensive_reference_and_rename_commands_cross_the_blocking_pool() {
        let source = include_str!("commands.rs");
        // `delete_page` was omitted here until the 2026-08-09 perf audit (F3)
        // measured it at 757 ms on an 8,006-file graph, on the command thread.
        for name in [
            "get_backlinks",
            "get_unlinked_refs",
            "block_ref_counts",
            "block_referrers",
            "get_backlink_filter_context",
            "list_templates",
            "query_facets",
            "run_query",
            "run_advanced_query",
            "export_query_subtrees",
            "list_orphan_assets",
            "open_pdf",
            "page_print_html",
            "run_graph_search",
            "search",
            "write_pdf_view_state",
            "rename_page",
            "delete_page",
            "merge_pages",
            "rename_file_to_page",
            "trash_journal_file",
            "resolve_sync_conflict",
        ] {
            let signature = format!("pub(crate) async fn {name}(");
            let start = source.find(&signature).expect("command stays async");
            let tail = &source[start..];
            let end = tail.find("\n#[tauri::command]").unwrap_or(tail.len());
            assert!(
                tail[..end].contains("tauri::async_runtime::spawn_blocking"),
                "{name} must not run graph-wide work on the command/UI thread"
            );
        }
    }
}

#[cfg(test)]
mod managed_actor_command_boundary_tests {
    #[test]
    fn move_acknowledgement_wakes_actor_cleanup_after_every_attempt() {
        let source = include_str!("commands.rs");
        let signature = "pub(crate) async fn acknowledge_managed_application_move(";
        let start = source
            .find(signature)
            .expect("acknowledgement command remains");
        let tail = &source[start..];
        let end = tail.find("\n#[tauri::command]").unwrap_or(tail.len());
        let command = &tail[..end];
        assert!(command.contains("tauri::async_runtime::spawn_blocking(move ||"));
        assert!(command.contains("slot_for_bound_window"));
        assert!(command.contains("Some(binding_generation)"));
        assert!(command.contains("crate::state::poke_watcher(&state);"));
        assert!(
            command.find("let result = handle").unwrap()
                < command.find("crate::state::poke_watcher(&state);").unwrap(),
            "the watcher wake must happen after the actor attempt and before its result returns"
        );
    }

    #[test]
    fn every_ordinary_managed_actor_command_re_resolves_off_the_async_command_thread() {
        let source = include_str!("commands.rs");
        for name in [
            "list_pages",
            "referenced_page_names",
            "journal_feed_page",
            "get_page",
            "save_page",
            "journal_content_days",
            "get_page_by_path",
            "page_aliases",
            "page_icons",
            "existing_page_names",
            "quick_switch",
            "resolve_block",
            "resolve_blocks",
            "preview_block",
            "block_ref_counts",
            "block_referrers",
            "get_backlinks",
            "get_backlink_filter_context",
            "get_unlinked_refs",
            "list_templates",
            "query_facets",
            "run_query",
            "run_advanced_query",
            "export_query_subtrees",
            "list_orphan_assets",
            "open_pdf",
            "open_page_file",
            "page_print_html",
            "run_graph_search",
            "search",
            "write_pdf_view_state",
        ] {
            let signature = format!("pub(crate) async fn {name}(");
            let start = source
                .find(&signature)
                .expect("managed command stays async");
            let tail = &source[start..];
            let end = tail.find("\n#[tauri::command]").unwrap_or(tail.len());
            let command = &tail[..end];
            assert!(
                command.contains("owned_graph_context(state)?"),
                "{name} must own the exact window binding before await"
            );
            assert!(
                command.contains("tauri::async_runtime::spawn_blocking(move ||"),
                "{name} must move every possible managed actor wait to the blocking pool"
            );
            assert!(
                command.contains("slot_for_bound_window")
                    && command.contains("Some(binding_generation)"),
                "{name} must re-resolve the captured generation inside the blocking operation"
            );
        }
    }

    #[test]
    fn managed_semantic_read_commands_never_fall_back_to_the_broad_parsed_cache() {
        let source = include_str!("commands.rs");
        for name in [
            "referenced_page_names",
            "page_aliases",
            "page_icons",
            "existing_page_names",
            "quick_switch",
            "resolve_block",
            "resolve_blocks",
            "preview_block",
            "block_ref_counts",
            "block_referrers",
            "get_backlinks",
            "get_backlink_filter_context",
            "get_unlinked_refs",
            "list_templates",
            "query_facets",
            "run_query",
            "run_advanced_query",
            "export_query_subtrees",
            "list_orphan_assets",
            "open_pdf",
            "open_page_file",
            "page_print_html",
            "run_graph_search",
            "search",
            "write_pdf_view_state",
        ] {
            let signature = format!("pub(crate) async fn {name}(");
            let start = source.find(&signature).expect("navigation command remains");
            let tail = &source[start..];
            let end = tail.find("\n#[tauri::command]").unwrap_or(tail.len());
            let command = &tail[..end];
            assert!(
                command.contains("sparse_application_handle("),
                "{name} must dispatch through an exact managed actor boundary"
            );
            assert!(
                !command.contains("with_read_graph(") && !command.contains("read_graph_cloned("),
                "{name} must not touch the managed broad parsed cache"
            );
        }
    }
}

#[tauri::command]
pub(crate) async fn publish_html(state: GraphContext<'_>) -> Result<(String, usize), CommandError> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        match sparse_application_handle(&slot)? {
            Some(handle) => match handle
                .publish_application_html()
                .map_err(CommandError::from)?
            {
                SyncApplicationPublishOutcome::Published { path, pages } => Ok((path, pages)),
                SyncApplicationPublishOutcome::Deferred { .. } => Err(CommandError::prose(
                    "Tine-managed storage is updating pages. Try publishing again when it finishes.",
                )),
            },
            None => slot
                .legacy_graph()?
                .publish_html()
                .map_err(CommandError::from),
        }
    })
    .await
    .map_err(CommandError::worker)?
}

/// Render one page to a self-contained HTML document (assets inlined, no sidebar)
/// for the print-to-PDF export, with the dialog's options. `Err("no-page")` if the
/// page doesn't exist.
#[tauri::command]
pub(crate) async fn page_print_html(
    name: String,
    opts: tine_core::publish::PrintOpts,
    state: GraphContext<'_>,
) -> Result<String, CommandError> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        match sparse_application_handle(&slot)? {
            Some(handle) => {
                let entry = sparse_page_inventory(handle)?
                    .into_iter()
                    .find(|entry| entry.name == name)
                    .ok_or_else(|| CommandError::prose("no-page"))?;
                let page = load_sparse_page(
                    handle,
                    SyncApplicationPageSelector::ExactPath {
                        path: entry.rel_path,
                    },
                )?
                .ok_or_else(|| CommandError::prose("no-page"))?;
                slot.with_filesystem_graph(|graph| {
                    graph
                        .page_print_html_page(&page, opts)
                        .map_err(CommandError::from)
                })
            }
            None => slot
                .legacy_graph()?
                .page_print_html(&name, opts)
                .map_err(CommandError::from)?
                .ok_or_else(|| CommandError::prose("no-page")),
        }
    })
    .await
    .map_err(CommandError::worker)?
}

#[tauri::command]
pub(crate) async fn run_query(
    query: String,
    state: GraphContext<'_>,
) -> Result<Arc<Vec<RefGroup>>, CommandError> {
    validate_query_source(&query)?;
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        match sparse_application_handle(&slot)? {
            Some(handle) => match sparse_navigation(
                handle,
                SyncApplicationNavigationRequest::SimpleQuery {
                    query: query.clone(),
                    max_rows: RESULT_BRIDGE_MAX_ROWS,
                    max_bytes: RESULT_BRIDGE_MAX_BYTES,
                },
            )? {
                SyncApplicationNavigationReply::SimpleQuery(result) => {
                    if result.exceeded {
                        Err(CommandError::prose(format!(
                            "result-too-large: {} matching blocks; narrow the query or add (sample N) (construction limits: {RESULT_BRIDGE_MAX_ROWS} blocks / {RESULT_BRIDGE_MAX_BYTES} bytes)",
                            result.total
                        )))
                    } else {
                        Ok(Arc::new(result.groups))
                    }
                }
                _ => Err(CommandError::prose("managed navigation returned the wrong reply")),
            },
            None => bounded_groups_or_error(slot.legacy_graph()?.run_query_bounded(
                &query,
                RESULT_BRIDGE_MAX_ROWS,
                RESULT_BRIDGE_MAX_BYTES,
            )),
        }
    })
    .await
    .map_err(CommandError::worker)?
}

/// Resolve every query macro in one Copy / Export session under one cumulative
/// construction budget. Unlike `get_page`, this returns only selected subtrees;
/// unrelated page content is never cloned across IPC or retained by the WebView.
#[tauri::command]
pub(crate) async fn export_query_subtrees(
    specs: Vec<tine_core::query::QueryExportSpec>,
    state: GraphContext<'_>,
) -> Result<tine_core::query::QueryExportBatch, CommandError> {
    let query_bytes = specs.iter().fold(0usize, |total, spec| {
        total
            .saturating_add(spec.key.len())
            .saturating_add(spec.query.len())
    });
    if specs.len() > QUERY_EXPORT_REQUEST_MAX_QUERIES || query_bytes > QUERY_EXPORT_MAX_QUERY_BYTES
    {
        return Err(CommandError::prose(format!(
            "query-export-request-too-large: {} macros / {} bytes (request limits: {} macros / {} bytes; processing cap: {} macros)",
            specs.len(),
            query_bytes,
            QUERY_EXPORT_REQUEST_MAX_QUERIES,
            QUERY_EXPORT_MAX_QUERY_BYTES,
            QUERY_EXPORT_MAX_QUERIES,
        )));
    }
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        let batch = match sparse_application_handle(&slot)? {
            Some(handle) => match sparse_navigation(
                handle,
                SyncApplicationNavigationRequest::ExportQuerySubtrees {
                    specs,
                    max_queries: QUERY_EXPORT_MAX_QUERIES,
                    max_roots: QUERY_EXPORT_MAX_ROOTS,
                    max_nodes: QUERY_EXPORT_MAX_NODES,
                    max_bytes: QUERY_EXPORT_MAX_BYTES,
                },
            )? {
                SyncApplicationNavigationReply::ExportQuerySubtrees(batch) => batch,
                _ => return Err(CommandError::prose("managed navigation returned the wrong reply")),
            },
            None => {
                let graph = slot.legacy_graph()?;
                tine_core::query::export_query_subtrees(
                    &graph,
                    &specs,
                    QUERY_EXPORT_MAX_QUERIES,
                    QUERY_EXPORT_MAX_ROOTS,
                    QUERY_EXPORT_MAX_NODES,
                    QUERY_EXPORT_MAX_BYTES,
                )
            }
        };
        let bytes = batch
            .results
            .iter()
            .map(|result| {
                result.key.len()
                    + result
                        .groups
                        .iter()
                        .map(|group| {
                            tine_core::model::ref_groups_estimated_bytes(std::slice::from_ref(
                                group,
                            ))
                        })
                        .sum::<usize>()
                    + 128
            })
            .sum::<usize>();
        if bytes > QUERY_EXPORT_MAX_BYTES {
            return Err(CommandError::prose(format!(
                "query-export-result-too-large: ~{bytes} bytes (limit: {QUERY_EXPORT_MAX_BYTES} bytes)"
            )));
        }
        Ok(batch)
    })
    .await
    .map_err(CommandError::worker)?
}

#[tauri::command]
pub(crate) async fn run_graph_search(
    source: String,
    page_limit: usize,
    block_limit: usize,
    lane: Option<String>,
    explain: bool,
    scope: Option<tine_core::query_plan::QueryPageScope>,
    state: GraphContext<'_>,
) -> Result<tine_core::query_plan::QueryExecution, CommandError> {
    let page_limit = page_limit.min(RESULT_BRIDGE_MAX_ROWS);
    let block_limit = block_limit.min(RESULT_BRIDGE_MAX_ROWS - page_limit);
    let (app, label, binding_generation) = owned_graph_context(state)?;
    let execution = tauri::async_runtime::spawn_blocking(move || -> Result<_, CommandError> {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        match sparse_application_handle(&slot)? {
            Some(handle) => match sparse_navigation(
                handle,
                SyncApplicationNavigationRequest::GraphSearch {
                    source,
                    page_limit,
                    block_limit,
                    lane,
                    explain,
                    scope,
                },
            )? {
                SyncApplicationNavigationReply::GraphSearch(execution) => Ok(execution),
                _ => Err(CommandError::prose(
                    "managed navigation returned the wrong reply",
                )),
            },
            None => Ok(match lane.as_deref() {
                Some(lane) => slot.legacy_graph()?.run_graph_search_latest_scoped(
                    lane,
                    &source,
                    page_limit,
                    block_limit,
                    scope,
                    explain,
                ),
                None => slot.legacy_graph()?.run_graph_search_scoped(
                    &source,
                    page_limit,
                    block_limit,
                    scope,
                    explain,
                ),
            }),
        }
    })
    .await
    .map_err(CommandError::worker)??;
    enforce_query_execution_budget(&execution)?;
    Ok(execution)
}

#[tauri::command]
pub(crate) async fn run_advanced_query(
    query: String,
    current_page: Option<String>,
    state: GraphContext<'_>,
) -> Result<tine_core::query::AdvancedResult, CommandError> {
    validate_query_source(&query)?;
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        let bounded = match sparse_application_handle(&slot)? {
            Some(handle) => match sparse_navigation(
                handle,
                SyncApplicationNavigationRequest::AdvancedQuery {
                    query: query.clone(),
                    current_page: current_page.clone(),
                    max_rows: RESULT_BRIDGE_MAX_ROWS,
                    max_bytes: RESULT_BRIDGE_MAX_BYTES,
                },
            )? {
                SyncApplicationNavigationReply::AdvancedQuery(result) => result,
                _ => {
                    return Err(CommandError::prose(
                        "managed navigation returned the wrong reply",
                    ))
                }
            },
            None => {
                let (result, exceeded, total) =
                    slot.legacy_graph()?.run_advanced_query_bounded_cached(
                        &query,
                        current_page.as_deref(),
                        RESULT_BRIDGE_MAX_ROWS,
                        RESULT_BRIDGE_MAX_BYTES,
                    );
                tine_core::sync_runtime::SyncApplicationBoundedAdvancedResult {
                    result,
                    total,
                    exceeded,
                }
            }
        };
        if bounded.exceeded {
            Err(CommandError::prose(format!(
                "result-too-large: {} advanced-query matches; narrow the query",
                bounded.total
            )))
        } else {
            Ok(bounded.result)
        }
    })
    .await
    .map_err(CommandError::worker)?
}

// ---------------------------------------------------------------------------
// The query-language command surface (SPEC §7.1).
//
// Six commands over ONE engine. `query_parse`, `query_print` and
// `query_og_expressible` are pure functions of their arguments plus (for
// suggestions) the graph's property registry; `query_registry`, `query_run` and
// `query_explain_empty` read the graph. The old `run_query` /
// `run_advanced_query` / `query_facets` / `export_query_subtrees` commands stay
// and keep working: P0-ts moves the frontend, and their deletion is a P1 item.

/// The INPUT a `query_parse` caller has, on the wire (SPEC §7.1).
///
/// `og`, `tql` and `advanced` are explicit FORM inputs. `macro_query` and
/// `macro_tql` take the COMPLETE raw macro argument, without the outer
/// delimiters, and are the only inputs that split a trailing options map — one
/// splitter, in Rust, so the frontend's own splitters can be deleted in P0-ts
/// (X4, W2).
#[derive(Clone, Copy, Debug, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum QueryTextDialect {
    Og,
    Tql,
    /// `{{query #+BEGIN_QUERY …}}` — datalog, parsed as the advanced form.
    Advanced,
    /// The complete argument of a `{{query …}}` macro: OG or advanced, decided
    /// here by the one Rust discriminator rather than by a frontend regex.
    MacroQuery,
    /// The complete argument of a `{{tine-query …}}` macro: TQL.
    MacroTql,
}

/// The printed form a `query_print` caller wants (SPEC §4.3, §7.1).
#[derive(Clone, Copy, Debug, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum QueryPrintDialect {
    Og,
    /// The text pane's multi-line editing layout.
    Tql,
    /// The persisted single-line `{{tine-query …}}` form.
    TqlMacro,
    /// A `{{query [:find …]}}` advanced macro, printed from its authored source.
    AdvancedMacro,
}

/// The `{query, view}` pair every parse returns (SPEC §7.1).
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct ParsedQuery {
    pub(crate) query: tine_core::query::ir::Query,
    pub(crate) view: tine_core::query::ir::ViewSettings,
}

/// The core input a wire dialect parses as. `Advanced` is OG's
/// `{{query #+BEGIN_QUERY …}}` form, which the OG parser already detects and
/// reports (M5); it is not a third parser.
fn core_query_input(dialect: QueryTextDialect) -> tine_core::query::QueryInput {
    use tine_core::query::QueryInput;
    match dialect {
        QueryTextDialect::Og => QueryInput::Og,
        QueryTextDialect::Advanced => QueryInput::Advanced,
        QueryTextDialect::Tql => QueryInput::Tql,
        QueryTextDialect::MacroQuery => QueryInput::MacroQuery,
        QueryTextDialect::MacroTql => QueryInput::MacroTql,
    }
}

fn core_print_dialect(dialect: QueryPrintDialect) -> tine_core::query::print::PrintDialect {
    use tine_core::query::print::PrintDialect;
    match dialect {
        QueryPrintDialect::Og => PrintDialect::Og,
        QueryPrintDialect::Tql => PrintDialect::Tql,
        QueryPrintDialect::TqlMacro => PrintDialect::TqlMacro,
        QueryPrintDialect::AdvancedMacro => PrintDialect::AdvancedMacro,
    }
}

/// The whole of `query_parse` that is not slot plumbing: parse, then merge the
/// host block's `tine.*` properties over the lifted directives (§4.1).
fn parse_query_pair(
    text: &str,
    dialect: QueryTextDialect,
    block_properties: &[(String, String)],
    registry: &tine_core::query::registry::Registry,
) -> ParsedQuery {
    let (query, parsed_view) = tine_core::query::parse_query_input(
        text,
        core_query_input(dialect),
        tine_core::date::JournalDate::today(),
        registry,
    );
    ParsedQuery {
        query,
        view: tine_core::query::view::merge_block_property_view(&parsed_view, block_properties),
    }
}

/// The whole of `query_print` that is not `#[tauri::command]`: print, and turn
/// a printer refusal into the one `CommandError` carrying the diagnostic.
fn print_query_text(
    query: &tine_core::query::ir::Query,
    view: &tine_core::query::ir::ViewSettings,
    dialect: QueryPrintDialect,
    preserve_form: bool,
) -> Result<String, CommandError> {
    tine_core::query::print::query_print(query, view, core_print_dialect(dialect), preserve_form)
        .map_err(|diagnostic| {
            let reason_code = match diagnostic.kind {
                tine_core::query::ir::DiagnosticKind::NotApplicable => "not_applicable",
                _ => "syntax",
            };
            CommandError::tagged(
                "query-print-refused",
                Some(reason_code),
                Some(serde_json::to_value(&diagnostic).unwrap_or(serde_json::Value::Null)),
            )
        })
}

/// A result the WebView cannot be handed is a refusal, not a truncation: the
/// same rule `run_query` applies, over the §7.1 shape.
fn query_result_or_error(
    result: tine_core::query::ir::QueryResult,
) -> Result<tine_core::query::ir::QueryResult, CommandError> {
    if result.exceeded {
        return Err(CommandError::coded(
            "result-too-large",
            format!(
                "{} matching rows; narrow the query or add a sample (construction limits: {RESULT_BRIDGE_MAX_ROWS} rows / {RESULT_BRIDGE_MAX_BYTES} bytes)",
                result.total
            ),
        ));
    }
    Ok(result)
}

/// Fetch the registry snapshot the parse reads for its `UnknownIdent`
/// suggestions, through whichever storage mode this slot is bound to.
fn query_registry_snapshot(
    slot: &crate::state::GraphSlot,
) -> Result<tine_core::query::ir::RegistrySnapshot, CommandError> {
    match sparse_application_handle(slot)? {
        Some(handle) => {
            match sparse_navigation(handle, SyncApplicationNavigationRequest::PropertyRegistry)? {
                SyncApplicationNavigationReply::PropertyRegistry(snapshot) => Ok(snapshot),
                _ => Err(CommandError::prose(
                    "managed navigation returned the wrong reply",
                )),
            }
        }
        None => Ok(slot.legacy_graph()?.property_registry().snapshot()),
    }
}

/// SPEC §7.1 `query_parse`: text → `{query, view}`, with the §4.1 precedence
/// merge of the host block's `tine.*` properties applied here and nowhere else
/// (M14).
#[tauri::command]
pub(crate) async fn query_parse(
    text: String,
    dialect: QueryTextDialect,
    block_properties: Option<Vec<(String, String)>>,
    state: GraphContext<'_>,
) -> Result<ParsedQuery, CommandError> {
    validate_query_source(&text)?;
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        // managed-command-routing: managed -- the sparse/legacy dispatch lives
        // in `query_registry_snapshot`, shared with `query_registry`.
        // The registry only ever adds suggestions to a diagnostic. A storage
        // mode that cannot answer must not turn a parse into a failure, so a
        // refusal degrades to the empty registry rather than propagating.
        let snapshot =
            query_registry_snapshot(&slot).unwrap_or(tine_core::query::ir::RegistrySnapshot {
                rows: Vec::new(),
                generation: 0,
            });
        let registry = tine_core::query::registry::Registry::from_snapshot(&snapshot);
        Ok(parse_query_pair(
            &text,
            dialect,
            &block_properties.unwrap_or_default(),
            &registry,
        ))
    })
    .await
    .map_err(CommandError::worker)?
}

/// SPEC §7.1 `query_print` (A4). `Ok(text)` for a printable IR; the OG printer
/// is partial, so a non-OG-expressible IR **rejects**, carrying the whole
/// `NotApplicable` diagnostic — never an empty string and never a stringified
/// message. The one caller entitled to see it is the save path, which switches
/// dialect; every other caller asked the OG printer without first checking
/// `query_og_expressible`, and that is a bug.
///
/// **Reconciliation with §7.1, recorded:** the spec says the command rejects
/// with the serialized `Diagnostic`. I-9 says every fallible command returns
/// the ONE `CommandError`. Both hold here: the rejection is a `CommandError`
/// whose structured `detail` IS the serialized diagnostic, so the frontend
/// reads `kind`, `message` and `suggestions` as objects rather than parsing
/// prose.
#[tauri::command]
pub(crate) async fn query_print(
    query: tine_core::query::ir::Query,
    view: tine_core::query::ir::ViewSettings,
    dialect: QueryPrintDialect,
    preserve_form: Option<bool>,
) -> Result<String, CommandError> {
    print_query_text(&query, &view, dialect, preserve_form.unwrap_or(false))
}

/// SPEC §7.1 `query_og_expressible`: whether the OG DSL can say this query, so
/// the save path can choose the macro name (Q3) without provoking a rejection.
#[tauri::command]
pub(crate) async fn query_og_expressible(
    query: tine_core::query::ir::Query,
    view: tine_core::query::ir::ViewSettings,
) -> bool {
    tine_core::query::print::og_expressible(&query, &view)
}

/// SPEC §7.1 `query_registry`: the observed property registry (§6.1).
#[tauri::command]
pub(crate) async fn query_registry(
    state: GraphContext<'_>,
) -> Result<tine_core::query::ir::RegistrySnapshot, CommandError> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        // managed-command-routing: managed -- see `query_registry_snapshot`.
        query_registry_snapshot(&slot)
    })
    .await
    .map_err(CommandError::worker)?
}

/// SPEC §7.1 `query_run`: the IR, already parsed, evaluated by the one walk.
/// `@page` rows carry `{name, kind, journal_day?}` and need no document load
/// (K16).
#[tauri::command]
pub(crate) async fn query_run(
    query: tine_core::query::ir::Query,
    view: tine_core::query::ir::ViewSettings,
    context: Option<tine_core::query::ir::ExecutionContext>,
    state: GraphContext<'_>,
) -> Result<tine_core::query::ir::QueryResult, CommandError> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    let context = context.unwrap_or_default();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        let bounds = tine_core::query::ir::Bounds {
            max_rows: RESULT_BRIDGE_MAX_ROWS,
            max_bytes: RESULT_BRIDGE_MAX_BYTES,
        };
        let result = match sparse_application_handle(&slot)? {
            Some(handle) => match sparse_navigation(
                handle,
                SyncApplicationNavigationRequest::QueryRun {
                    query,
                    view,
                    context,
                    max_rows: bounds.max_rows,
                    max_bytes: bounds.max_bytes,
                },
            )? {
                SyncApplicationNavigationReply::QueryRun(result) => result,
                _ => {
                    return Err(CommandError::prose(
                        "managed navigation returned the wrong reply",
                    ))
                }
            },
            None => {
                let graph = slot.legacy_graph()?;
                tine_core::query::run_query_result_ir(&graph, &query, &view, bounds, &context)
            }
        };
        query_result_or_error(result)
    })
    .await
    .map_err(CommandError::worker)?
}

/// SPEC §7.1 `query_explain_empty` (Q14, N19): why the query returned nothing.
#[tauri::command]
pub(crate) async fn query_explain_empty(
    query: tine_core::query::ir::Query,
    view: tine_core::query::ir::ViewSettings,
    context: Option<tine_core::query::ir::ExecutionContext>,
    state: GraphContext<'_>,
) -> Result<tine_core::query::ir::ExplainEmptyResult, CommandError> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    let context = context.unwrap_or_default();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        let bounds = tine_core::query::ir::Bounds {
            max_rows: RESULT_BRIDGE_MAX_ROWS,
            max_bytes: RESULT_BRIDGE_MAX_BYTES,
        };
        match sparse_application_handle(&slot)? {
            Some(handle) => match sparse_navigation(
                handle,
                SyncApplicationNavigationRequest::QueryExplainEmpty {
                    query,
                    view,
                    context,
                    max_rows: bounds.max_rows,
                    max_bytes: bounds.max_bytes,
                },
            )? {
                SyncApplicationNavigationReply::QueryExplainEmpty(explained) => Ok(explained),
                _ => Err(CommandError::prose(
                    "managed navigation returned the wrong reply",
                )),
            },
            None => {
                let graph = slot.legacy_graph()?;
                Ok(tine_core::query::explain_empty_query(
                    &graph, &query, &view, bounds, &context,
                ))
            }
        }
    })
    .await
    .map_err(CommandError::worker)?
}

#[tauri::command]
pub(crate) async fn query_facets(
    state: GraphContext<'_>,
    autocomplete: Option<bool>,
) -> Result<Vec<(String, Vec<String>)>, CommandError> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        let autocomplete = autocomplete.unwrap_or(false);
        match sparse_application_handle(&slot)? {
            Some(handle) => {
                let meta = slot.graph_meta();
                let (max_items, max_bytes) = if autocomplete {
                    (AUTOCOMPLETE_FACET_MAX_ITEMS, AUTOCOMPLETE_FACET_MAX_BYTES)
                } else {
                    (RESULT_BRIDGE_MAX_ROWS, RESULT_BRIDGE_MAX_BYTES)
                };
                match sparse_navigation(
                    handle,
                    SyncApplicationNavigationRequest::PropertyFacets {
                        autocomplete,
                        hidden_properties: meta.block_hidden_properties,
                        max_items,
                        max_bytes,
                    },
                )? {
                    SyncApplicationNavigationReply::PropertyFacets { facets, exceeded } => {
                        if exceeded && !autocomplete {
                            Err(CommandError::coded(
                                "result-too-large",
                                "property facets exceed the construction budget",
                            ))
                        } else {
                            Ok(facets)
                        }
                    }
                    _ => Err(CommandError::prose(
                        "managed navigation returned the wrong reply",
                    )),
                }
            }
            None => {
                let graph = slot.legacy_graph()?;
                if autocomplete {
                    return Ok(graph
                        .autocomplete_property_facets_bounded(
                            AUTOCOMPLETE_FACET_MAX_ITEMS,
                            AUTOCOMPLETE_FACET_MAX_BYTES,
                        )
                        .0);
                }
                let (facets, exceeded) =
                    graph.property_facets_bounded(RESULT_BRIDGE_MAX_ROWS, RESULT_BRIDGE_MAX_BYTES);
                if exceeded {
                    Err(CommandError::coded(
                        "result-too-large",
                        "property facets exceed the construction budget",
                    ))
                } else {
                    Ok(facets)
                }
            }
        }
    })
    .await
    .map_err(CommandError::worker)?
}

#[tauri::command]
pub(crate) async fn page_aliases(
    state: GraphContext<'_>,
) -> Result<Vec<(String, String)>, CommandError> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        match sparse_application_handle(&slot)? {
            Some(handle) => {
                match sparse_navigation(handle, SyncApplicationNavigationRequest::PageAliases)? {
                    SyncApplicationNavigationReply::PageAliases(aliases) => Ok(aliases),
                    _ => Err(CommandError::prose(
                        "managed navigation returned the wrong reply",
                    )),
                }
            }
            None => Ok(slot.legacy_graph()?.page_aliases()),
        }
    })
    .await
    .map_err(CommandError::worker)?
}

#[tauri::command]
pub(crate) async fn page_icons(
    names: Vec<String>,
    state: GraphContext<'_>,
) -> Result<std::collections::HashMap<String, String>, CommandError> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        match sparse_application_handle(&slot)? {
            Some(handle) => match sparse_navigation(
                handle,
                SyncApplicationNavigationRequest::PageIcons { names },
            )? {
                SyncApplicationNavigationReply::PageIcons(icons) => Ok(icons),
                _ => Err(CommandError::prose(
                    "managed navigation returned the wrong reply",
                )),
            },
            None => Ok(slot.legacy_graph()?.page_icons(&names)),
        }
    })
    .await
    .map_err(CommandError::worker)?
}

#[tauri::command]
pub(crate) async fn existing_page_names(
    names: Vec<String>,
    state: GraphContext<'_>,
) -> Result<Vec<String>, CommandError> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        match sparse_application_handle(&slot)? {
            Some(handle) => match sparse_navigation(
                handle,
                SyncApplicationNavigationRequest::ExistingPageNames { names },
            )? {
                SyncApplicationNavigationReply::ExistingPageNames(names) => Ok(names),
                _ => Err(CommandError::prose(
                    "managed navigation returned the wrong reply",
                )),
            },
            None => Ok(slot.legacy_graph()?.existing_page_names(&names)),
        }
    })
    .await
    .map_err(CommandError::worker)?
}

#[tauri::command]
pub(crate) fn set_favorites(
    names: Vec<String>,
    state: GraphContext<'_>,
) -> Result<(), CommandError> {
    with_config_graph(&state, |g| {
        g.set_favorites(&names).map_err(CommandError::from)
    })
}

#[tauri::command]
pub(crate) fn set_favorites_page(
    name: String,
    state: GraphContext<'_>,
) -> Result<(), CommandError> {
    with_config_graph(&state, |g| {
        g.set_favorites_page(&name).map_err(CommandError::from)
    })
}

#[tauri::command]
pub(crate) fn set_default_home(
    name: Option<String>,
    state: GraphContext<'_>,
) -> Result<(), CommandError> {
    with_config_graph(&state, |graph| {
        graph
            .set_default_home_page(name.as_deref())
            .map_err(CommandError::from)
    })?;
    refresh_graph(&state)?;
    Ok(())
}

#[tauri::command]
pub(crate) fn set_preferred_workflow(
    workflow: String,
    state: GraphContext<'_>,
) -> Result<(), CommandError> {
    with_config_graph(&state, |g| {
        g.set_preferred_workflow(&workflow)
            .map_err(CommandError::from)
    })
}

#[tauri::command]
pub(crate) fn set_timetracking_enabled(
    enabled: bool,
    state: GraphContext<'_>,
) -> Result<(), CommandError> {
    with_config_graph(&state, |g| {
        g.set_timetracking_enabled(enabled)
            .map_err(CommandError::from)
    })?;
    refresh_graph(&state)?;
    Ok(())
}

#[tauri::command]
pub(crate) fn set_show_brackets(
    enabled: bool,
    state: GraphContext<'_>,
) -> Result<(), CommandError> {
    with_config_graph(&state, |g| {
        g.set_show_brackets(enabled).map_err(CommandError::from)
    })?;
    refresh_graph(&state)?;
    Ok(())
}

#[tauri::command]
pub(crate) fn set_doc_mode_enter_for_new_block(
    enabled: bool,
    state: GraphContext<'_>,
) -> Result<(), CommandError> {
    with_config_graph(&state, |g| {
        g.set_doc_mode_enter_for_new_block(enabled)
            .map_err(CommandError::from)
    })?;
    refresh_graph(&state)?;
    Ok(())
}

#[tauri::command]
pub(crate) fn set_logical_outdenting(
    enabled: bool,
    state: GraphContext<'_>,
) -> Result<(), CommandError> {
    with_config_graph(&state, |g| {
        g.set_logical_outdenting(enabled)
            .map_err(CommandError::from)
    })?;
    refresh_graph(&state)?;
    Ok(())
}

#[tauri::command]
pub(crate) fn set_guide_announced(
    announced: bool,
    state: GraphContext<'_>,
) -> Result<(), CommandError> {
    with_config_graph(&state, |g| {
        g.set_guide_announced(announced).map_err(CommandError::from)
    })?;
    refresh_graph(&state)?;
    Ok(())
}

#[tauri::command]
pub(crate) fn set_default_journal_template(
    name: Option<String>,
    state: GraphContext<'_>,
) -> Result<(), CommandError> {
    with_config_graph(&state, |g| {
        g.set_default_journal_template(name.as_deref())
            .map_err(CommandError::from)
    })
}

#[tauri::command]
pub(crate) fn set_start_of_week(n: u32, state: GraphContext<'_>) -> Result<(), CommandError> {
    with_config_graph(&state, |g| {
        g.set_start_of_week(n).map_err(CommandError::from)
    })
}

/// Set the graph's `:preferred-format` for new pages/journals ("md" or "org").
#[tauri::command]
pub(crate) fn set_preferred_format(
    format: String,
    state: GraphContext<'_>,
) -> Result<(), CommandError> {
    let fmt = if format.eq_ignore_ascii_case("org") {
        tine_core::model::Format::Org
    } else {
        tine_core::model::Format::Md
    };
    with_config_graph(&state, |g| {
        g.set_preferred_format(fmt).map_err(CommandError::from)
    })?;
    refresh_graph(&state)?; // so new pages/journals use the new extension immediately
    Ok(())
}

/// Set the graph's `:journal/page-title-format` (journal display-title format,
/// e.g. "MMM do, yyyy"). Display-only — does not rename journal files.
#[tauri::command]
pub(crate) fn set_journal_title_format(
    format: String,
    state: GraphContext<'_>,
) -> Result<(), CommandError> {
    with_config_graph(&state, |g| {
        g.set_journal_page_title_format(&format)
            .map_err(CommandError::from)
    })?;
    refresh_graph(&state)?; // pick up the new format + migrate any title-named journals
    Ok(())
}

#[tauri::command]
pub(crate) fn read_custom_css(state: GraphContext<'_>) -> Result<String, CommandError> {
    with_filesystem_graph(&state, |g| Ok(g.custom_css()))
}

#[tauri::command]
pub(crate) async fn search(
    query: String,
    limit: usize,
    lane: Option<String>,
    state: GraphContext<'_>,
) -> Result<Vec<RefGroup>, CommandError> {
    let limit = limit.min(RESULT_BRIDGE_MAX_ROWS);
    let (app, label, binding_generation) = owned_graph_context(state)?;
    let groups = tauri::async_runtime::spawn_blocking(move || -> Result<_, CommandError> {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        match sparse_application_handle(&slot)? {
            Some(handle) => match sparse_navigation(
                handle,
                SyncApplicationNavigationRequest::BlockSearch { query, limit, lane },
            )? {
                SyncApplicationNavigationReply::BlockSearch(groups) => Ok(groups),
                _ => Err(CommandError::prose(
                    "managed navigation returned the wrong reply",
                )),
            },
            None => Ok(match lane.as_deref() {
                Some(lane) => slot.legacy_graph()?.search_latest(lane, &query, limit),
                None => slot.legacy_graph()?.search(&query, limit),
            }),
        }
    })
    .await
    .map_err(CommandError::worker)??;
    enforce_result_bridge_budget(&groups)?;
    Ok(groups)
}

#[tauri::command]
pub(crate) async fn quick_switch(
    query: String,
    limit: usize,
    state: GraphContext<'_>,
) -> Result<Vec<PageEntry>, CommandError> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        match sparse_application_handle(&slot)? {
            Some(handle) => match sparse_navigation(
                handle,
                SyncApplicationNavigationRequest::QuickSwitch { query, limit },
            )? {
                SyncApplicationNavigationReply::QuickSwitch(pages) => Ok(pages),
                _ => Err(CommandError::prose(
                    "managed navigation returned the wrong reply",
                )),
            },
            None => Ok(slot.legacy_graph()?.quick_switch(&query, limit)),
        }
    })
    .await
    .map_err(CommandError::worker)?
}

fn capture_quick_switch_for(
    state: &AppState,
    caller: &str,
    binding_generation: Option<u64>,
    query: &str,
    limit: usize,
) -> Result<Vec<PageEntry>, CommandError> {
    let slot = capture_quick_switch_slot(state, caller, binding_generation)?;
    Ok(slot.legacy_graph()?.quick_switch(query, limit.min(8)))
}

/// The sole graph-backed capability exposed to Quick Capture. It is deliberately
/// not a `GraphContext` command: capture may ask for bounded page/tag candidates
/// but cannot save, delete, trash, or invoke any other graph command.
#[tauri::command]
pub(crate) fn capture_quick_switch(
    query: String,
    limit: usize,
    binding_generation: Option<u64>,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<Vec<PageEntry>, CommandError> {
    capture_quick_switch_for(&state, window.label(), binding_generation, &query, limit)
}

#[cfg(test)]
mod capture_quick_switch_tests {
    use super::*;
    use crate::state::{slot_for_bound_window, GraphRegistry, GraphSlot};
    use std::path::PathBuf;
    use std::sync::atomic::AtomicU64;
    use std::sync::{Mutex, RwLock};
    use tine_core::model::Graph;

    fn state_with_selected_graph() -> (AppState, PathBuf) {
        let base = std::env::temp_dir().join(format!(
            "tine-capture-quick-switch-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let selected = base.join("selected");
        let other = base.join("other");
        for (root, page) in [
            (&selected, "Selected Capture Target"),
            (&other, "Other Target"),
        ] {
            std::fs::create_dir_all(root.join("pages")).unwrap();
            std::fs::create_dir_all(root.join("journals")).unwrap();
            std::fs::write(root.join("pages").join(format!("{page}.md")), "- fixture\n").unwrap();
        }
        let state = AppState {
            graphs: RwLock::new(GraphRegistry::default()),
            storage_supervisor: crate::storage_mode_supervisor::StorageModeSupervisor::default(),
            watch_ctl: Mutex::new(None),
            last_focused: Mutex::new(Some("main".into())),
            capture_graph: Mutex::new(None),
            sync_runtime: crate::sync_runtime::SyncRuntimeFacade::default(),
            #[cfg(desktop)]
            next_window: AtomicU64::new(2),
        };
        let selected_slot = Arc::new(GraphSlot::new(Graph::open(&selected), selected.clone()));
        let generation = selected_slot.binding_generation;
        state
            .graphs
            .write()
            .unwrap()
            .bind("main".into(), selected_slot)
            .unwrap();
        state
            .graphs
            .write()
            .unwrap()
            .bind(
                "other".into(),
                Arc::new(GraphSlot::new(Graph::open(&other), other)),
            )
            .unwrap();
        state.bind_capture_graph("main".into(), generation);
        (state, base)
    }

    #[test]
    fn returns_candidates_from_the_selected_capture_graph() {
        let (state, base) = state_with_selected_graph();
        let generation = state.capture_graph_binding().unwrap().binding_generation;
        let result =
            capture_quick_switch_for(&state, "capture", Some(generation), "Selected Capture", 8)
                .unwrap();
        assert!(result
            .iter()
            .any(|page| page.name == "Selected Capture Target"));
        assert!(!result.iter().any(|page| page.name == "Other Target"));
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn rejects_a_stale_capture_binding_generation() {
        let (state, base) = state_with_selected_graph();
        let generation = state.capture_graph_binding().unwrap().binding_generation;
        assert_eq!(
            capture_quick_switch_for(&state, "capture", Some(generation + 1), "Selected", 8)
                .unwrap_err()
                .to_string(),
            "stale-graph-binding"
        );
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn rejects_non_capture_callers() {
        let (state, base) = state_with_selected_graph();
        let generation = state.capture_graph_binding().unwrap().binding_generation;
        assert_eq!(
            capture_quick_switch_for(&state, "main", Some(generation), "Selected", 8)
                .unwrap_err()
                .to_string(),
            "capture quick switch is only available to quick capture"
        );
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn capture_binding_never_grants_generic_graphcontext_mutation_access() {
        let (state, base) = state_with_selected_graph();
        let generation = state.capture_graph_binding().unwrap().binding_generation;
        // `save_page` and other mutations resolve through GraphContext, which
        // uses this normal window-slot path and therefore has no capture fallback.
        assert_eq!(
            slot_for_bound_window(&state, "capture", Some(generation))
                .err()
                .unwrap()
                .to_string(),
            "no graph loaded for window capture"
        );
        std::fs::remove_dir_all(base).unwrap();
    }
}

#[tauri::command]
pub(crate) async fn list_templates(
    state: GraphContext<'_>,
) -> Result<Vec<tine_core::model::TemplateDto>, CommandError> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        match sparse_application_handle(&slot)? {
            Some(handle) => {
                match sparse_navigation(handle, SyncApplicationNavigationRequest::ListTemplates)? {
                    SyncApplicationNavigationReply::Templates(templates) => Ok(templates),
                    _ => Err(CommandError::prose(
                        "managed navigation returned the wrong reply",
                    )),
                }
            }
            None => Ok(slot.legacy_graph()?.templates()),
        }
    })
    .await
    .map_err(CommandError::worker)?
}

/// Managed command loading recognizes `key:: value` lines through the one
/// shared recognizer (tine-core `doc::parse_property_line`, transcribed from
/// lsdoc) instead of a local copy — the two had drifted on leading whitespace,
/// Unicode keys and dotted keys (DUP-7).
fn application_property_line(line: &str) -> bool {
    tine_core::doc::parse_property_line(line).is_some()
}

fn application_blocks_have_content(blocks: &[BlockDto]) -> bool {
    blocks.iter().any(|block| {
        block
            .raw
            .lines()
            .any(|line| !line.trim().is_empty() && !application_property_line(line))
            || application_blocks_have_content(&block.children)
    })
}

fn sparse_journal_content_days(handle: &SyncRuntimeHandle) -> Result<Vec<i64>, CommandError> {
    let entries = sparse_page_inventory(handle)?;
    let mut days = Vec::new();
    for entry in entries {
        if entry.kind != PageKind::Journal {
            continue;
        }
        let Some(day) = entry.date_key else {
            continue;
        };
        let page = load_sparse_page(
            handle,
            SyncApplicationPageSelector::ExactPath {
                path: entry.rel_path,
            },
        )?;
        if page.is_some_and(|page| application_blocks_have_content(&page.blocks)) {
            days.push(day);
        }
    }
    Ok(days)
}

#[tauri::command]
pub(crate) async fn journal_content_days(
    state: GraphContext<'_>,
) -> Result<Vec<i64>, CommandError> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        match sparse_application_handle(&slot)? {
            Some(handle) => sparse_journal_content_days(handle),
            None => Ok(slot.legacy_graph()?.journal_content_days()),
        }
    })
    .await
    .map_err(CommandError::worker)?
}

#[tauri::command]
pub(crate) async fn resolve_block(
    uuid: String,
    state: GraphContext<'_>,
) -> Result<Option<RefGroup>, CommandError> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        let group = match sparse_application_handle(&slot)? {
            Some(handle) => match sparse_navigation(
                handle,
                SyncApplicationNavigationRequest::ResolveBlocks {
                    uuids: vec![uuid.clone()],
                },
            )? {
                SyncApplicationNavigationReply::ResolveBlocks(mut groups) => groups.pop().flatten(),
                _ => {
                    return Err(CommandError::prose(
                        "managed block resolution returned the wrong reply kind",
                    ))
                }
            },
            None => slot.legacy_graph()?.resolve_block(&uuid),
        };
        if let Some(group) = &group {
            enforce_result_bridge_budget(std::slice::from_ref(group))?;
        }
        Ok(group)
    })
    .await
    .map_err(CommandError::worker)?
}

#[tauri::command]
pub(crate) async fn resolve_blocks(
    uuids: Vec<String>,
    state: GraphContext<'_>,
) -> Result<Vec<Option<RefGroup>>, CommandError> {
    if uuids.len() > RESULT_BRIDGE_MAX_ROWS {
        return Err(CommandError::prose(format!(
            "result-too-large: {} requested block references (limit: {RESULT_BRIDGE_MAX_ROWS})",
            uuids.len()
        )));
    }
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        match sparse_application_handle(&slot)? {
            Some(handle) => match sparse_navigation(
                handle,
                SyncApplicationNavigationRequest::ResolveBlocks { uuids },
            )? {
                SyncApplicationNavigationReply::ResolveBlocks(groups) => {
                    enforce_optional_result_bridge_budget(&groups)?;
                    Ok(groups)
                }
                _ => Err(CommandError::prose(
                    "managed block resolution returned the wrong reply kind",
                )),
            },
            None => {
                let graph = slot.legacy_graph()?;
                let (groups, exceeded, total) = tine_core::query::resolve_blocks_bounded(
                    &graph,
                    &uuids,
                    RESULT_BRIDGE_MAX_ROWS,
                    RESULT_BRIDGE_MAX_BYTES,
                );
                if exceeded {
                    Err(CommandError::coded(
                        "result-too-large",
                        format!(
                            "{total} resolved block-reference rows exceed the construction budget"
                        ),
                    ))
                } else {
                    Ok(groups)
                }
            }
        }
    })
    .await
    .map_err(CommandError::worker)?
}

/// Explicit, bounded subtree resolution for hover previews. Ordinary
/// `resolve_block(s)` stays shallow so a page containing nested references
/// cannot multiply the same descendants across the IPC bridge.
#[tauri::command]
pub(crate) async fn preview_block(
    uuid: String,
    max_nodes: usize,
    state: GraphContext<'_>,
) -> Result<Option<tine_core::BlockPreview>, CommandError> {
    const MAX_PREVIEW_NODES: usize = 2_000;
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        let max_nodes = max_nodes.clamp(1, MAX_PREVIEW_NODES);
        let max_bytes = RESULT_BRIDGE_MAX_BYTES.saturating_sub(4 * 1024);
        let preview = match sparse_application_handle(&slot)? {
            Some(handle) => match sparse_navigation(
                handle,
                SyncApplicationNavigationRequest::PreviewBlock {
                    uuid,
                    max_nodes,
                    max_bytes,
                },
            )? {
                SyncApplicationNavigationReply::PreviewBlock(preview) => preview,
                _ => {
                    return Err(CommandError::prose(
                        "managed block preview returned the wrong reply kind",
                    ))
                }
            },
            None => slot
                .legacy_graph()?
                .preview_block_with_budget(&uuid, max_nodes, max_bytes),
        };
        if let Some(preview) = &preview {
            enforce_result_bridge_budget(std::slice::from_ref(&preview.group))?;
        }
        Ok(preview)
    })
    .await
    .map_err(CommandError::worker)?
}

#[tauri::command]
pub(crate) fn read_asset(
    name: String,
    max_bytes: Option<u64>,
    state: GraphContext<'_>,
) -> Result<tauri::ipc::Response, CommandError> {
    // Return RAW bytes (not a JSON number[]), so a multi-MB PDF/image isn't
    // serialized element-by-element and re-parsed on the JS side — the frontend
    // receives an ArrayBuffer directly.
    let window_label = state.window.label().to_string();
    let (bytes, path) = with_filesystem_graph(&state, |g| {
        let path = g.asset_file_for_read(&name).map_err(CommandError::from)?;
        let bytes = max_bytes
            .map_or_else(
                || g.read_asset(&name),
                |limit| g.read_asset_limited(&name, limit),
            )
            .map_err(CommandError::from)?;
        Ok((bytes, path))
    })?;
    crate::watcher::note_asset_read(&window_label, &path);
    crate::state::poke_watcher(&state.state);
    Ok(tauri::ipc::Response::new(bytes))
}

/// Validate one graph media file and return its top-level asset name for the
/// range-aware `tine-media:` protocol. The protocol revalidates against the
/// requesting window's current graph on every request.
#[tauri::command]
pub(crate) fn stream_asset_path(
    name: String,
    state: GraphContext<'_>,
) -> Result<String, CommandError> {
    let slot = slot_for_context(&state)?;
    slot.with_filesystem_graph(|graph| graph.stream_asset_path(&name).map_err(CommandError::from))?;
    Ok(format!("{}/{}", slot.binding_generation, name))
}

/// The process-wide managed-shutdown result. `Partial` is intentionally a
/// successful IPC response: native preparation has made durable progress, so
/// Android must keep its editor shielded and retry only preparation rather than
/// treating it as an ordinary refusal and replaying the frontend flush.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum TineQuitPreparation {
    Safe,
    Refused {
        detail: String,
    },
    Partial {
        safe_slots: Vec<String>,
        detail: String,
    },
}

/// Snapshot labels before invoking any actor, sort them so the observed partial
/// prefix is stable, then stop at the first refusal. The cleaner is injected so
/// the sequencing contract has a small deterministic test independent of the
/// real actor setup.
fn prepare_tine_quit_slots<S, F>(mut slots: Vec<(String, S)>, mut clean: F) -> TineQuitPreparation
where
    F: FnMut(&S) -> Result<crate::sync_runtime::CleanShutdownSlot, CommandError>,
{
    slots.sort_by(|(left, _), (right, _)| left.cmp(right));
    let mut safe_slots = Vec::new();
    for (label, slot) in slots {
        match clean(&slot) {
            // Direct Files needs no managed shutdown proof and must not be
            // represented as a managed safe slot.
            Ok(crate::sync_runtime::CleanShutdownSlot::Direct) => {}
            Ok(crate::sync_runtime::CleanShutdownSlot::Safe) => safe_slots.push(label),
            Err(error) => {
                crate::debug::diag(format!("managed shutdown refused: {error}"));
                let detail =
                    tine_core::sync_runtime::tagged_backend_error("sparse-shutdown-refused", None);
                return if safe_slots.is_empty() {
                    TineQuitPreparation::Refused { detail }
                } else {
                    TineQuitPreparation::Partial { safe_slots, detail }
                };
            }
        }
    }
    TineQuitPreparation::Safe
}

/// The native half of a process-wide clean quit. Kept separate from actually
/// exiting so Android can prove every managed slot is safe before handing the
/// final activity exit to its native SafeBack owner.
fn prepare_tine_quit_all_slots(state: &crate::state::AppState) -> TineQuitPreparation {
    let slots = state.graphs.read().unwrap().entries();
    prepare_tine_quit_slots(slots, |slot| {
        crate::sync_runtime::clean_shutdown_slot(slot).map_err(CommandError::prose)
    })
}

/// Verify that every managed runtime has stopped safely, without exiting the
/// process. Android follows this with its SafeBack-owned activity exit.
#[tauri::command]
pub(crate) fn prepare_tine_quit(
    state: tauri::State<'_, crate::state::AppState>,
) -> TineQuitPreparation {
    prepare_tine_quit_all_slots(&state)
}

/// Quit the app cleanly. After every managed slot has stopped safely, Linux
/// first SIGKILLs WebKitGTK's helper subprocesses so they do not run their
/// buggy GL-driver atexit teardown and dump a SIGABRT core on exit (GH #28).
/// The JS close handler calls this only after `flushAll()`/`flushSession()`
/// resolve, so tearing the web process down hard loses no edits.
#[tauri::command]
pub(crate) fn tine_quit(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::state::AppState>,
) -> Result<(), CommandError> {
    match prepare_tine_quit_all_slots(&state) {
        TineQuitPreparation::Safe => {}
        TineQuitPreparation::Refused { detail } | TineQuitPreparation::Partial { detail, .. } => {
            return Err(CommandError::prose(detail));
        }
    }
    #[cfg(target_os = "linux")]
    crate::platform::kill_webkit_children();
    app.exit(0);
    Ok(())
}

/// Close only the calling graph window. The final graph window still performs
/// the process-wide WebKit cleanup before exit; the hidden capture window never
/// keeps the process alive by itself.
#[tauri::command]
pub(crate) fn close_graph_window(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::state::AppState>,
) -> Result<(), CommandError> {
    let slot = crate::state::slot_for_window(&state, window.label())?;
    crate::sync_runtime::clean_shutdown_slot(&slot).map_err(|error| {
        crate::debug::diag(format!("managed window shutdown refused: {error}"));
        CommandError::tagged("sparse-shutdown-refused", None::<String>, None)
    })?;
    if state.graphs.read().unwrap().len() <= 1 {
        #[cfg(target_os = "linux")]
        crate::platform::kill_webkit_children();
        app.exit(0);
        return Ok(());
    }
    window.destroy().map_err(CommandError::from)
}

#[cfg(test)]
mod prepare_tine_quit_tests {
    use super::*;
    use crate::sync_runtime::CleanShutdownSlot;
    use std::cell::RefCell;

    #[test]
    fn partial_shutdown_is_sorted_stops_at_refusal_and_never_becomes_safe() {
        let calls = RefCell::new(Vec::new());
        let result = prepare_tine_quit_slots(
            vec![
                ("B".into(), "refuse"),
                ("direct".into(), "direct"),
                ("A".into(), "safe"),
            ],
            |outcome| {
                calls.borrow_mut().push(*outcome);
                match *outcome {
                    "safe" => Ok(CleanShutdownSlot::Safe),
                    "direct" => Ok(CleanShutdownSlot::Direct),
                    "refuse" => Err(CommandError::prose("retained local publication")),
                    other => panic!("unexpected fixture outcome {other}"),
                }
            },
        );

        assert_eq!(calls.into_inner(), vec!["safe", "refuse"]);
        assert_eq!(
            result,
            TineQuitPreparation::Partial {
                safe_slots: vec!["A".into()],
                detail: tine_core::sync_runtime::tagged_backend_error(
                    "sparse-shutdown-refused",
                    None,
                ),
            }
        );
    }

    #[test]
    fn retry_accepts_an_already_safe_slot_once_every_slot_is_safe() {
        let first =
            prepare_tine_quit_slots(vec![("A".into(), true), ("B".into(), false)], |safe| {
                safe.then_some(CleanShutdownSlot::Safe)
                    .ok_or_else(|| CommandError::prose("B refused"))
            });
        assert!(matches!(first, TineQuitPreparation::Partial { .. }));

        let retry = prepare_tine_quit_slots(vec![("B".into(), true), ("A".into(), true)], |safe| {
            safe.then_some(CleanShutdownSlot::Safe)
                .ok_or_else(|| CommandError::prose("B refused"))
        });
        assert_eq!(retry, TineQuitPreparation::Safe);
    }

    #[test]
    fn zero_progress_refusal_is_typed_and_serializable() {
        let result = prepare_tine_quit_slots(vec![("B".into(), false)], |safe| {
            safe.then_some(CleanShutdownSlot::Safe)
                .ok_or_else(|| CommandError::prose("terminal runtime"))
        });
        assert_eq!(
            result,
            TineQuitPreparation::Refused {
                detail: tine_core::sync_runtime::tagged_backend_error(
                    "sparse-shutdown-refused",
                    None,
                ),
            }
        );
        let encoded = serde_json::to_value(result).unwrap();
        assert_eq!(encoded["status"], "refused");
        assert_eq!(
            encoded["detail"],
            serde_json::Value::String(tine_core::sync_runtime::tagged_backend_error(
                "sparse-shutdown-refused",
                None,
            ))
        );
    }
}

/// Toggle the WebView developer tools (WebKit Web Inspector) for theme/CSS
/// debugging (GH #31). `open_devtools`/`close_devtools` are compiled in because
/// we enable tauri's `devtools` feature unconditionally (see Cargo.toml) — so
/// this works in shipped release builds, not just debug.
#[tauri::command]
pub(crate) fn tine_open_devtools(window: tauri::WebviewWindow) {
    if window.is_devtools_open() {
        window.close_devtools();
    } else {
        // #31 follow-up: on X11/XWayland, open the inspector as its OWN window
        // instead of docked into the app. Docked, WebKitGTK puts the window's resize
        // grip at the top of the inspector pane. Do not force this on native Wayland:
        // Fedora 44 / WebKitGTK 2.52 renders the detached inspector black, while its
        // docked inspector is correctly scaled and usable. Query the actual GDK
        // display rather than session environment variables because an AppImage in a
        // Wayland session deliberately runs GTK through XWayland.
        // WebKit creates/attaches the inspector asynchronously, so an immediate
        // is_attached()+detach() races and usually does nothing. Arm a one-shot hook
        // BEFORE opening instead. The attach signal is the event boundary; its idle
        // continuation runs after WebKit's default attach handler has finished, then
        // detaches. There is deliberately no guessed timeout. Disconnecting first
        // also lets the user attach the already-open inspector manually afterward.
        #[cfg(target_os = "linux")]
        {
            let _ = window.with_webview(|wv| {
                use gtk::{gdk::prelude::DisplayExtManual, prelude::WidgetExt};
                use std::{cell::RefCell, rc::Rc};
                use webkit2gtk::{glib, glib::prelude::ObjectExt, WebInspectorExt, WebViewExt};
                if wv.inner().display().backend().is_wayland() {
                    return;
                }
                if let Some(inspector) = wv.inner().inspector() {
                    let handler_slot = Rc::new(RefCell::new(None));
                    let callback_slot = Rc::clone(&handler_slot);
                    let handler_id = inspector.connect_attach(move |inspector| {
                        if let Some(handler_id) = callback_slot.borrow_mut().take() {
                            inspector.disconnect(handler_id);
                        }
                        let inspector = inspector.clone();
                        glib::idle_add_local_once(move || {
                            if inspector.is_attached() {
                                inspector.detach();
                            }
                        });
                        false
                    });
                    *handler_slot.borrow_mut() = Some(handler_id);
                }
            });
        }
        // Tauri queues UI-thread messages in order: with_webview installs the
        // hook above before this open request is dispatched.
        window.open_devtools();
    }
}

#[tauri::command]
pub(crate) fn read_local_image(
    path: String,
    app: tauri::AppHandle,
) -> Result<tauri::ipc::Response, CommandError> {
    // Read an image from an ABSOLUTE path OUTSIDE the graph, for raw-HTML `<img>`
    // srcs the user has explicitly opted into (Settings → "Load local-file images").
    // OFF by default; gated here too (defense in depth — the frontend also checks),
    // restricted to image extensions + a size cap so an allowed note can't slurp an
    // arbitrary file. Returns RAW bytes like `read_asset`. See ADR 0019.
    if !crate::settings::get_app_bool("allow_local_file_images".into(), false, app) {
        return Err(CommandError::prose("local-file images are disabled"));
    }
    let p = std::path::Path::new(&path);
    let ext_ok = matches!(
        p.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref(),
        Some("png" | "jpg" | "jpeg" | "gif" | "svg" | "webp" | "bmp" | "ico" | "avif" | "apng")
    );
    if !ext_ok {
        return Err(CommandError::prose("not an image file"));
    }
    let meta = std::fs::metadata(p).map_err(CommandError::from)?;
    if !meta.is_file() {
        return Err(CommandError::prose("not a file"));
    }
    const MAX_BYTES: u64 = 64 * 1024 * 1024;
    if meta.len() > MAX_BYTES {
        return Err(CommandError::prose("image too large"));
    }
    std::fs::read(p)
        .map(tauri::ipc::Response::new)
        .map_err(CommandError::from)
}

#[tauri::command]
pub(crate) fn import_asset(
    path: String,
    name: Option<String>,
    state: GraphContext<'_>,
) -> Result<String, CommandError> {
    let window_label = state.window.label().to_string();
    with_filesystem_graph(&state, |g| {
        let stored = g
            .import_asset(std::path::Path::new(&path), name.as_deref())
            .map_err(CommandError::from)?;
        crate::watcher::note_asset_self_write(&window_label, &g.assets_path().join(&stored));
        Ok(stored)
    })
}

/// Import a bounded Android photo or voice memo by native cache-file capability.
/// Media never crosses Kotlin/WebView/Rust as base64; Rust streams the open file
/// into the graph and removes the temp only after the durable asset commit.
#[tauri::command]
pub(crate) fn import_native_capture(
    path: String,
    name: String,
    app: tauri::AppHandle,
    state: GraphContext<'_>,
) -> Result<String, CommandError> {
    use cap_std::{ambient_authority, fs::Dir};
    use tauri::Manager;

    const MAX_PHOTO_BYTES: u64 = 64 * 1024 * 1024;
    const MAX_RECORDING_BYTES: u64 = 32 * 1024 * 1024;
    let source = std::path::Path::new(&path);
    let filename = source
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| CommandError::prose("invalid native capture token"))?;
    let (max_bytes, media_label) =
        if filename.starts_with("tine_memo_") && filename.ends_with(".m4a") {
            (MAX_RECORDING_BYTES, "recording")
        } else if filename.starts_with("tine_photo_") && filename.ends_with(".jpg") {
            (MAX_PHOTO_BYTES, "photo")
        } else {
            return Err(CommandError::prose("invalid native capture token"));
        };
    let cache_path = app.path().app_cache_dir().map_err(CommandError::from)?;
    let token_parent = source
        .parent()
        .ok_or_else(|| CommandError::prose("recording has no cache parent"))?;
    let cache_dir =
        Dir::open_ambient_dir(&cache_path, ambient_authority()).map_err(CommandError::from)?;
    let token_dir =
        Dir::open_ambient_dir(token_parent, ambient_authority()).map_err(CommandError::from)?;
    let cache_identity = same_file::Handle::from_file(
        cache_dir
            .try_clone()
            .map_err(CommandError::from)?
            .into_std_file(),
    )
    .map_err(CommandError::from)?;
    let token_identity = same_file::Handle::from_file(
        token_dir
            .try_clone()
            .map_err(CommandError::from)?
            .into_std_file(),
    )
    .map_err(CommandError::from)?;
    if token_identity != cache_identity {
        return Err(CommandError::prose(
            "capture is outside Tine's native cache",
        ));
    }

    let capture = token_dir.open(filename).map_err(CommandError::from)?;
    let metadata = capture.metadata().map_err(CommandError::from)?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > max_bytes {
        return Err(CommandError::prose(format!(
            "{media_label} is empty or exceeds the {} MiB limit",
            max_bytes / (1024 * 1024)
        )));
    }
    let mut capture = capture.into_std();
    let window_label = state.window.label().to_string();
    let stored = with_filesystem_graph(&state, |graph| {
        let stored = graph
            .import_asset_file(&mut capture, &name, max_bytes)
            .map_err(CommandError::from)?;
        crate::watcher::note_asset_self_write(&window_label, &graph.assets_path().join(&stored));
        Ok(stored)
    })?;
    // The graph asset is authoritative now. Cleanup failure is harmless cache
    // litter and must not make the frontend omit the already-durable reference.
    let _ = cache_dir.remove_file(filename);
    Ok(stored)
}

/// Read a dropped delimited-text file for the CSV/TSV → grid drop path.
/// Deliberately NARROW: this is the only webview-reachable read of a
/// caller-chosen path (everything else is gated to the graph/assets dirs),
/// so it refuses anything that isn't the drop feature's file types — it must
/// not grow into a general file-read primitive.
#[tauri::command]
pub(crate) fn read_text_file(path: String) -> Result<String, CommandError> {
    fn delimited_ext(p: &std::path::Path) -> bool {
        p.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("csv") || e.eq_ignore_ascii_case("tsv"))
            .unwrap_or(false)
    }
    let p = std::path::Path::new(&path);
    if !delimited_ext(p) {
        return Err(CommandError::prose("unsupported file type"));
    }
    // Re-check on the RESOLVED path too — a symlink named x.csv pointing at an
    // arbitrary file must not pass the extension gate (review finding).
    let resolved = std::fs::canonicalize(p).map_err(CommandError::from)?;
    if !delimited_ext(&resolved) {
        return Err(CommandError::prose("unsupported file type"));
    }
    let meta = std::fs::metadata(&resolved).map_err(CommandError::from)?;
    if !meta.is_file() {
        return Err(CommandError::prose("not a file"));
    }
    const MAX_BYTES: u64 = 10 * 1024 * 1024;
    if meta.len() > MAX_BYTES {
        return Err(CommandError::prose("text file too large"));
    }
    std::fs::read_to_string(&resolved).map_err(CommandError::from)
}

/// Open a graph asset (by its `assets/`-relative name) in the OS default app —
/// a file in its system viewer, a directory (or the empty name, i.e. the
/// assets root itself like OG's `[...](./assets/)`) in the file manager.
/// Path-gated to the assets dir (canonicalized) so a crafted name can't open
/// anything outside the graph.
#[tauri::command]
pub(crate) fn open_asset(name: String, state: GraphContext<'_>) -> Result<(), CommandError> {
    let target = with_filesystem_graph(&state, |g| {
        g.asset_path_for_open(&name).map_err(CommandError::from)
    })?;
    #[cfg(desktop)]
    {
        #[cfg(target_os = "linux")]
        let prog = "xdg-open";
        #[cfg(target_os = "macos")]
        let prog = "open";
        #[cfg(target_os = "windows")]
        let prog = "explorer";
        diag(format!(
            "open_asset: {name} -> {} ({prog})",
            target.display()
        ));
        opener_command(prog)
            .arg(&target)
            .spawn()
            .map_err(CommandError::from)?;
        Ok(())
    }
    // Mobile: opening an asset in an external app uses a platform intent; stub for now (M1).
    #[cfg(not(desktop))]
    {
        let _ = (&name, &target);
        Err(CommandError::prose(
            "open asset externally is not supported on this platform",
        ))
    }
}

/// Open or reveal the exact source file recorded on a loaded page. Rust resolves
/// and canonicalizes the graph-relative identity; the WebView never supplies an
/// arbitrary absolute path.
#[tauri::command]
pub(crate) async fn open_page_file(
    name: String,
    kind: PageKind,
    path: Option<String>,
    reveal: bool,
    state: GraphContext<'_>,
) -> Result<(), CommandError> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    let target = tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        match sparse_application_handle(&slot)? {
            Some(handle) => {
                let page = load_sparse_page(
                    handle,
                    SyncApplicationPageSelector::Logical {
                        name,
                        page_kind: kind.into(),
                    },
                )?
                .ok_or_else(|| CommandError::prose("no-page"))?;
                slot.with_filesystem_graph(|graph| {
                    graph
                        .page_source_file(&page.name, page.kind, Some(&page.path))
                        .map_err(CommandError::from)
                })
            }
            None => slot
                .legacy_graph()?
                .page_source_file(&name, kind, path.as_deref())
                .map_err(CommandError::from),
        }
    })
    .await
    .map_err(CommandError::worker)??;
    #[cfg(desktop)]
    {
        if reveal {
            reveal_page_source(&target).map_err(CommandError::prose)
        } else {
            open_page_source(&target).map_err(CommandError::prose)
        }
    }
    #[cfg(not(desktop))]
    {
        let _ = (target, reveal);
        Err(CommandError::prose(
            "page file actions are available on desktop only",
        ))
    }
}

/// Open a graph asset in a SPECIFIC external editor (drawio/Excalidraw/…) so a
/// diagram can be edited in place. `command` is the user-configured command
/// template for that editor (from Settings → Files); empty falls back to the OS
/// opener, exactly like `open_asset`. The template is tokenised on whitespace:
/// token[0] is the program, a `{}` inside any token is replaced by the asset
/// path, and if no argument contains `{}` the path is appended as the final arg.
/// Spawned as an argv (no shell → no injection) through `opener_command`, which
/// scrubs the WebKitGTK/AppImage env and detaches the child (so a Flatpak drawio
/// doesn't inherit Tine's bundled `LD_LIBRARY_PATH`). Path-gated to `assets/`.
/// Double quotes group a program/argument containing whitespace; backslashes are
/// literal so Windows paths such as `"C:\Program Files\draw.io\draw.io.exe" {}`
/// survive unchanged.
#[tauri::command]
pub(crate) fn edit_asset_external(
    name: String,
    command: String,
    state: GraphContext<'_>,
) -> Result<(), CommandError> {
    let target = with_filesystem_graph(&state, |g| {
        g.asset_file_for_read(&name).map_err(CommandError::from)
    })?;
    #[cfg(desktop)]
    {
        let target_str = target.to_string_lossy().to_string();
        let trimmed = command.trim();
        if trimmed.is_empty() {
            // No editor configured → same OS opener as open_asset.
            #[cfg(target_os = "linux")]
            let prog = "xdg-open";
            #[cfg(target_os = "macos")]
            let prog = "open";
            #[cfg(target_os = "windows")]
            let prog = "explorer";
            diag(format!(
                "edit_asset_external: {name} -> {target_str} (opener {prog})"
            ));
            opener_command(prog)
                .arg(&target)
                .spawn()
                .map_err(CommandError::from)?;
            return Ok(());
        }
        let (prog, args) = build_editor_argv(trimmed, &target_str)?;
        diag(format!("edit_asset_external: {name} -> {prog} {args:?}"));
        opener_command(&prog)
            .args(&args)
            .spawn()
            .map_err(CommandError::from)?;
        Ok(())
    }
    #[cfg(not(desktop))]
    {
        let _ = (&name, &command, &target);
        Err(CommandError::prose(
            "editing an asset externally is not supported on this platform",
        ))
    }
}

/// Best-effort autodetect of an installed external editor's launch command, by
/// PROBING known install locations on disk — never executing anything (so a
/// Flatpak wrapper can't leak its bundled env into the probe). Returns a command
/// template suitable for `edit_asset_external`, or an empty string if not found
/// (the caller then leaves the setting empty = OS opener). Currently knows
/// `drawio`; other ids return empty.
#[tauri::command]
pub(crate) fn detect_media_editor(id: String) -> Result<String, CommandError> {
    #[cfg(desktop)]
    {
        if id == "drawio" {
            return Ok(detect_drawio());
        }
        Ok(String::new())
    }
    #[cfg(not(desktop))]
    {
        let _ = id;
        Ok(String::new())
    }
}

/// Probe common drawio install sites without executing. Order: Flatpak exported
/// launcher (checked as a FILE, per the reporter's note — not via `flatpak run`,
/// which would inherit our env), then snap, then a `drawio` on PATH, then the
/// platform app bundle. Returns a command template or "".
#[cfg(desktop)]
fn detect_drawio() -> String {
    #[cfg(target_os = "linux")]
    {
        // Flatpak: the exported bin is a plain wrapper file we can stat.
        let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
        let flatpak_bins = [
            home.as_ref()
                .map(|h| h.join(".local/share/flatpak/exports/bin/com.jgraph.drawio.desktop")),
            Some(std::path::PathBuf::from(
                "/var/lib/flatpak/exports/bin/com.jgraph.drawio.desktop",
            )),
        ];
        for b in flatpak_bins.into_iter().flatten() {
            if b.exists() {
                return "flatpak run com.jgraph.drawio.desktop {}".to_string();
            }
        }
        if std::path::Path::new("/snap/bin/drawio").exists() {
            return "/snap/bin/drawio {}".to_string();
        }
        if let Some(p) = which_on_path("drawio") {
            return format!("{} {{}}", p.display());
        }
        String::new()
    }
    #[cfg(target_os = "macos")]
    {
        if std::path::Path::new("/Applications/draw.io.app").exists() {
            return "open -a draw.io {}".to_string();
        }
        String::new()
    }
    #[cfg(target_os = "windows")]
    {
        detect_drawio_windows()
    }
}

#[cfg(any(target_os = "windows", test))]
fn detect_drawio_windows() -> String {
    detect_drawio_windows_with(
        |name: &'static str| std::env::var_os(name),
        |path| path.is_file(),
    )
}

/// Windows installers can be per-user (`LOCALAPPDATA`) or per-machine
/// (`ProgramFiles`, including 32-bit installs). Keep the environment/filesystem
/// inputs injectable so this platform-specific discovery policy is covered by
/// host tests without mutating the process environment.
#[cfg(any(target_os = "windows", test))]
fn detect_drawio_windows_with<V, F>(mut var: V, mut is_file: F) -> String
where
    // Every probed environment name below is a string literal. Expressing that
    // lifetime avoids passing the generic `std::env::var_os` function item
    // through a higher-ranked `FnMut(&str)` bound, which MSVC rejects as "not
    // general enough" even though host builds accept it.
    V: FnMut(&'static str) -> Option<std::ffi::OsString>,
    F: FnMut(&std::path::Path) -> bool,
{
    let locations = [
        ("LOCALAPPDATA", Some("Programs")),
        ("ProgramFiles", None),
        ("ProgramFiles(x86)", None),
    ];
    for (variable, extra) in locations {
        let Some(root) = var(variable) else {
            continue;
        };
        let mut exe = std::path::PathBuf::from(root);
        if let Some(component) = extra {
            exe.push(component);
        }
        exe.push("draw.io");
        exe.push("draw.io.exe");
        if is_file(&exe) {
            // Windows executable paths commonly contain spaces. The tokenizer
            // below strips these grouping quotes before direct argv spawning.
            return format!("\"{}\" {{}}", exe.display());
        }
    }
    String::new()
}

/// Find an executable by name on `$PATH` (stat only, no exec). Linux/macOS.
#[cfg(all(desktop, unix))]
fn which_on_path(name: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|cand| cand.is_file())
}

/// Split a user command template into (program, args) for an editor launch.
/// Double quotes group whitespace but are not passed to the child; backslashes
/// are always literal, which is required for ordinary Windows paths. This is a
/// deliberately small argv tokenizer, not a shell: there is no expansion,
/// interpolation, or escape syntax. Unmatched quotes and an empty program are
/// rejected. `{}` is substituted in arguments; otherwise the target path is
/// appended as the final argument.
#[cfg(any(desktop, test))]
fn build_editor_argv(command: &str, target: &str) -> Result<(String, Vec<String>), CommandError> {
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut token_started = false;
    let mut quoted = false;
    for ch in command.chars() {
        match ch {
            '"' => {
                quoted = !quoted;
                token_started = true;
            }
            ch if ch.is_whitespace() && !quoted => {
                if token_started {
                    tokens.push(std::mem::take(&mut token));
                    token_started = false;
                }
            }
            _ => {
                token.push(ch);
                token_started = true;
            }
        }
    }
    if quoted {
        return Err(CommandError::prose(
            "unclosed double quote in editor command",
        ));
    }
    if token_started {
        tokens.push(token);
    }

    let (prog, rest) = tokens
        .split_first()
        .ok_or_else(|| CommandError::prose("empty editor command"))?;
    if prog.is_empty() {
        return Err(CommandError::prose("editor command program is empty"));
    }
    let mut args: Vec<String> = Vec::new();
    let mut substituted = false;
    for tok in rest {
        if tok.contains("{}") {
            args.push(tok.replace("{}", target));
            substituted = true;
        } else {
            args.push((*tok).to_string());
        }
    }
    if !substituted {
        args.push(target.to_string());
    }
    Ok((prog.clone(), args))
}

#[cfg(test)]
mod editor_argv_tests {
    use super::{build_editor_argv, detect_drawio_windows, detect_drawio_windows_with};
    use std::{ffi::OsString, path::PathBuf};

    #[test]
    fn appends_path_when_no_placeholder() {
        let (p, a) = build_editor_argv("drawio", "/g/assets/x.drawio.svg").unwrap();
        assert_eq!(p, "drawio");
        assert_eq!(a, vec!["/g/assets/x.drawio.svg"]);
    }

    #[test]
    fn substitutes_a_placeholder_token() {
        let (p, a) =
            build_editor_argv("flatpak run com.jgraph.drawio.desktop {}", "/g/x.svg").unwrap();
        assert_eq!(p, "flatpak");
        assert_eq!(a, vec!["run", "com.jgraph.drawio.desktop", "/g/x.svg"]);
    }

    #[test]
    fn substitutes_inside_a_token() {
        let (p, a) = build_editor_argv("app --file={}", "/g/x.svg").unwrap();
        assert_eq!(p, "app");
        assert_eq!(a, vec!["--file=/g/x.svg"]);
    }

    #[test]
    fn quoted_windows_program_path_is_one_argv_token() {
        let (p, a) = build_editor_argv(
            r#""C:\Program Files\draw.io\draw.io.exe" {}"#,
            r#"C:\graph\assets\x.drawio.svg"#,
        )
        .unwrap();
        assert_eq!(p, r#"C:\Program Files\draw.io\draw.io.exe"#);
        assert_eq!(a, vec![r#"C:\graph\assets\x.drawio.svg"#]);
    }

    #[test]
    fn quoted_argument_with_spaces_is_one_argv_token() {
        let (p, a) = build_editor_argv(
            r#"drawio --profile "C:\Users\Me\Drawio Profile" {}"#,
            r#"C:\graph\assets\x.drawio.svg"#,
        )
        .unwrap();
        assert_eq!(p, "drawio");
        assert_eq!(
            a,
            vec![
                r#"--profile"#,
                r#"C:\Users\Me\Drawio Profile"#,
                r#"C:\graph\assets\x.drawio.svg"#,
            ]
        );
    }

    #[test]
    fn malformed_or_empty_commands_are_rejected() {
        assert_eq!(
            build_editor_argv("   ", "/g/x.svg")
                .unwrap_err()
                .to_string(),
            "empty editor command"
        );
        assert_eq!(
            build_editor_argv(r#""C:\Program Files\draw.io\draw.io.exe {}"#, "/g/x.svg")
                .unwrap_err()
                .to_string(),
            "unclosed double quote in editor command"
        );
        assert_eq!(
            build_editor_argv(r#""" {}"#, "/g/x.svg")
                .unwrap_err()
                .to_string(),
            "editor command program is empty"
        );
    }

    #[test]
    fn windows_autodetect_checks_per_machine_install_locations() {
        for variable in ["ProgramFiles", "ProgramFiles(x86)"] {
            let root = PathBuf::from(format!("/{variable}"));
            let expected = root.join("draw.io").join("draw.io.exe");
            let command = detect_drawio_windows_with(
                |key| (key == variable).then(|| OsString::from(&root)),
                |path| path == expected,
            );
            assert_eq!(command, format!("\"{}\" {{}}", expected.display()));
        }
    }

    #[test]
    fn windows_autodetect_keeps_per_user_install_first() {
        let local = PathBuf::from("/Local App Data");
        let machine = PathBuf::from("/Program Files");
        let expected = local.join("Programs").join("draw.io").join("draw.io.exe");
        let command = detect_drawio_windows_with(
            |key| match key {
                "LOCALAPPDATA" => Some(OsString::from(&local)),
                "ProgramFiles" => Some(OsString::from(&machine)),
                _ => None,
            },
            |path| path == expected || path == machine.join("draw.io").join("draw.io.exe"),
        );
        assert_eq!(command, format!("\"{}\" {{}}", expected.display()));
    }

    #[test]
    fn windows_autodetect_returns_empty_when_no_candidate_is_a_file() {
        let command = detect_drawio_windows_with(|_| Some(OsString::from("/missing")), |_| false);
        assert!(command.is_empty());
    }

    #[test]
    fn windows_autodetect_real_callbacks_compile_and_run() {
        // This wrapper is the exact Windows call site. Keeping it compiled in
        // host tests catches callback lifetime regressions even before the
        // Windows CI runner builds the cfg(target_os = "windows") branch.
        let _ = detect_drawio_windows();
    }
}

/// Orphaned `assets/` files (no block references them) for the cleanup UI.
#[tauri::command]
pub(crate) async fn list_orphan_assets(
    state: GraphContext<'_>,
) -> Result<Vec<tine_core::model::AssetInfo>, CommandError> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        match sparse_application_handle(&slot)? {
            Some(handle) => {
                match sparse_navigation(handle, SyncApplicationNavigationRequest::OrphanAssets)? {
                    SyncApplicationNavigationReply::OrphanAssets(assets) => Ok(assets),
                    _ => Err(CommandError::prose(
                        "managed navigation returned the wrong reply",
                    )),
                }
            }
            None => Ok(slot.legacy_graph()?.orphan_assets()),
        }
    })
    .await
    .map_err(CommandError::worker)?
}

/// Move an orphaned asset to the recoverable trash.
#[tauri::command]
pub(crate) fn trash_asset(name: String, state: GraphContext<'_>) -> Result<(), CommandError> {
    let window_label = state.window.label().to_string();
    with_trash_graph(&state, |g| {
        let path = g.assets_path().join(&name);
        g.trash_asset(&name).map_err(CommandError::from)?;
        crate::watcher::note_asset_self_write(&window_label, &path);
        Ok(())
    })
}

/// Count + total bytes in the recoverable asset trash.
#[tauri::command]
pub(crate) fn asset_trash_stats(
    state: GraphContext<'_>,
) -> Result<tine_core::model::TrashStats, CommandError> {
    with_filesystem_graph(&state, |g| Ok(g.asset_trash_stats()))
}

/// Permanently delete everything in the asset trash; returns files removed.
#[tauri::command]
pub(crate) fn empty_asset_trash(state: GraphContext<'_>) -> Result<u64, CommandError> {
    with_trash_graph(&state, |g| {
        g.empty_asset_trash().map_err(CommandError::from)
    })
}

/// Journal days that resolve to more than one file (e.g. a date-stem file plus a
/// title-named one) — for the user to reconcile.
#[tauri::command]
pub(crate) fn list_journal_conflicts(
    state: GraphContext<'_>,
) -> Result<Vec<tine_core::model::JournalConflict>, CommandError> {
    with_filesystem_graph(&state, |g| Ok(g.journal_conflicts()))
}

/// Concord L0 reload-on-focus fallback: ask the watcher for ONE full stat-diff
/// pass right now. Whatever changed on disk is then emitted through the normal
/// `graph-changed` path, so the deferred-replay machinery decides what may be
/// applied — this command never touches a page itself.
///
/// Deliberately graph-slot-free: it arms a process-wide flag on the single
/// watcher thread, which already covers every bound graph in both regimes.
#[tauri::command]
pub(crate) fn rescan_graph_now(state: tauri::State<'_, AppState>) -> u64 {
    let sequence = crate::watcher::request_full_rescan();
    crate::state::poke_watcher(&state);
    sequence
}

/// Journal files whose names don't round-trip to a date, and the names they
/// would get. Concord invariant 4 (write-shyness): opening a graph used to
/// perform these renames silently; it now only proposes them here.
#[tauri::command]
pub(crate) fn list_journal_filename_migrations(
    state: GraphContext<'_>,
) -> Result<Vec<tine_core::model::JournalFilenameMigration>, CommandError> {
    with_filesystem_graph(&state, |g| Ok(g.journal_filename_migrations()))
}

/// Apply the proposed journal renames, on the user's explicit request. Takes the
/// same pre-migration snapshot the open path used to take, so the original
/// filenames stay recoverable in Backups & recovery. Returns how many were
/// renamed (the migration never clobbers an existing target).
///
/// Graph-text mutation with no managed analogue: renaming graph files is the
/// oplog's authority under managed storage and stays refused there.
#[tauri::command]
pub(crate) async fn apply_journal_filename_migrations(
    state: GraphContext<'_>,
) -> Result<usize, CommandError> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        let graph = slot.legacy_graph()?;
        crate::backup::backup_graph_now(&app, &graph, "");
        graph
            .migrate_journal_filenames_checked()
            .map_err(CommandError::from)
    })
    .await
    .map_err(CommandError::worker)?
}

/// Sync-tool conflict copies (Syncthing/Dropbox) sitting in the graph — for the
/// user to review + reconcile instead of them showing as garbage pages.
#[tauri::command]
pub(crate) fn list_sync_conflicts(
    state: GraphContext<'_>,
) -> Result<Vec<tine_core::model::SyncConflict>, CommandError> {
    with_filesystem_graph(&state, |g| Ok(g.list_sync_conflicts()))
}

/// Pages whose on-disk bytes carry unresolved VCS merge-conflict markers
/// (git/Fossil). They stay readable but saves to them are refused, so the
/// conflicts panel and the page banner can explain why.
#[tauri::command]
pub(crate) fn list_vcs_marker_conflicts(
    state: GraphContext<'_>,
) -> Result<Vec<tine_core::model::VcsMarkerConflict>, CommandError> {
    with_filesystem_graph(&state, |g| Ok(g.list_vcs_marker_conflicts()))
}

/// The Concord conflict queue (L3): ONE derived inventory of everything on disk
/// that needs the user's judgement — conflict copies AND marker-bearing pages —
/// behind the calm badge and the in-page resolver. Derived on every call from
/// what is on disk, so it survives restarts without storing anything.
///
/// Async + `spawn_blocking`: deriving the queue block-diffs every conflicted
/// page, so a pathological page must stall a worker thread, never the main
/// IPC thread (audit 2026-08-24, finding A3).
#[tauri::command]
pub(crate) async fn conflict_queue(
    state: GraphContext<'_>,
) -> Result<Vec<tine_core::concord_queue::ConflictObject>, CommandError> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        slot.with_filesystem_graph(|g| Ok(g.conflict_queue()))
    })
    .await
    .map_err(CommandError::worker)?
}

/// Block-level diff of a marker-bearing page's own sides (Concord L5): the
/// marker sections are parsed into complete page texts and run through the SAME
/// block diff the conflict-copy path uses. Read-only.
#[tauri::command]
pub(crate) async fn vcs_marker_conflict_diff(
    path: String,
    state: GraphContext<'_>,
) -> Result<Option<tine_core::concord_queue::MarkerConflictDiff>, CommandError> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        slot.with_filesystem_graph(|g| {
            g.vcs_marker_conflict_diff(&path)
                .map_err(CommandError::from)
        })
    })
    .await
    .map_err(CommandError::worker)?
}

/// Apply the user's per-row decisions to a marker-bearing page, writing the
/// clean merged result — the one write Concord invariant 3 permits to such a
/// file. `base_rev` guards against the VCS changing it under the review.
///
/// Graph-text write on a Direct Files phenomenon: managed storage has no
/// marker-bearing files, so this is deliberately legacy-authority only.
#[tauri::command]
pub(crate) async fn resolve_vcs_marker_conflict(
    path: String,
    decisions: std::collections::HashMap<String, String>,
    base_rev: String,
    pre_choice: Option<String>,
    state: GraphContext<'_>,
) -> Result<(), CommandError> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        slot.legacy_graph()?
            .resolve_vcs_marker_conflict(
                &path,
                &decisions,
                &base_rev,
                pre_choice.as_deref().unwrap_or("union"),
            )
            .map_err(direct_save_error_message)
    })
    .await
    .map_err(CommandError::worker)?
}

/// Block-level diff of a sync-conflict copy against its winner (both graph-root-
/// relative paths) — the data behind the two-column merge UI. Read-only.
#[tauri::command]
pub(crate) async fn sync_conflict_diff(
    winner: String,
    conflict: String,
    state: GraphContext<'_>,
) -> Result<Option<tine_core::sync_diff::SyncConflictDiff>, CommandError> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        slot.with_filesystem_graph(|g| {
            g.sync_conflict_diff(&winner, &conflict)
                .map_err(CommandError::from)
        })
    })
    .await
    .map_err(CommandError::worker)?
}

/// Two-way diff of a duplicate journal day's canonical file against one of its
/// strays — the data behind the same two-column merge UI the sync-copy path
/// uses. `Ok(None)` when the pair cannot be merged at all (a cross-format
/// `.md`/`.org` twin), which the UI renders as file rows without row choices.
/// Read-only.
#[tauri::command]
pub(crate) async fn duplicate_journal_diff(
    canonical: String,
    stray: String,
    state: GraphContext<'_>,
) -> Result<Option<tine_core::sync_diff::SyncConflictDiff>, CommandError> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        slot.with_filesystem_graph(|g| {
            g.duplicate_journal_diff(&canonical, &stray)
                .map_err(CommandError::from)
        })
    })
    .await
    .map_err(CommandError::worker)?
}

/// Fold one stray of a duplicate journal day into that day's canonical file with
/// the user's per-row decisions, moving the stray to recoverable trash. Guarded
/// so it can only ever touch two files of the SAME duplicate day.
#[tauri::command]
pub(crate) async fn resolve_duplicate_journal_day(
    canonical: String,
    stray: String,
    decisions: std::collections::HashMap<String, String>,
    base_rev: String,
    stray_rev: String,
    pre_choice: Option<String>,
    state: GraphContext<'_>,
) -> Result<PageDto, CommandError> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        slot.legacy_graph()?
            .resolve_duplicate_journal_day(
                &canonical,
                &stray,
                &decisions,
                &base_rev,
                &stray_rev,
                pre_choice.as_deref().unwrap_or("union"),
            )
            .map_err(CommandError::from)
    })
    .await
    .map_err(CommandError::worker)?
}

/// Block-level diff of two raw page texts — a pure function of its inputs,
/// needing no graph, path, or slot (Concord P3's path-free seam; future in-page
/// conflict UI builds on it). `format`: `"org"` selects the org parser,
/// anything else means markdown. Revs are `content_rev` of the exact inputs,
/// the same staleness tokens `Graph::sync_conflict_diff` issues.
#[tauri::command]
pub(crate) async fn text_block_diff(
    mine: String,
    theirs: String,
    format: Option<String>,
) -> Result<tine_core::sync_diff::SyncConflictDiff, CommandError> {
    tauri::async_runtime::spawn_blocking(move || {
        tine_core::sync_diff::diff_texts(&mine, &theirs, format.as_deref() == Some("org"))
    })
    .await
    .map_err(CommandError::worker)
}

/// 3-way variant of [`text_block_diff`]: classifies each aligned row against
/// `base` (the last-agreed text) and carries per-row suggestions the UI may
/// pre-select — never auto-apply. See ADR 0056.
#[tauri::command]
pub(crate) async fn text_block_diff3(
    base: String,
    mine: String,
    theirs: String,
    format: Option<String>,
) -> Result<tine_core::sync_diff::SyncConflictDiff, CommandError> {
    tauri::async_runtime::spawn_blocking(move || {
        tine_core::sync_diff::diff3_texts(&base, &mine, &theirs, format.as_deref() == Some("org"))
    })
    .await
    .map_err(CommandError::worker)
}

/// Diff a retained live Direct Files draft against the exact disk observation
/// that refused its save. The authority is inspected, never consumed.
#[tauri::command]
pub(crate) async fn live_save_conflict_diff(
    page: PageDto,
    base_rev: Option<String>,
    conflict_epoch: u64,
    state: GraphContext<'_>,
) -> Result<tine_core::sync_diff::SyncConflictDiff, CommandError> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        slot.with_filesystem_graph(|graph| {
            graph
                .live_save_conflict_diff(
                    &page,
                    base_rev.as_deref(),
                    tine_core::ConflictOverride {
                        observation_epoch: conflict_epoch,
                    },
                )
                .map_err(CommandError::from)
        })
    })
    .await
    .map_err(CommandError::worker)?
}

#[tauri::command]
pub(crate) async fn capture_live_save_conflict(
    page: PageDto,
    base_rev: Option<String>,
    conflict_epoch: u64,
    state: GraphContext<'_>,
) -> Result<Option<tine_core::LiveSaveConflictCapture>, CommandError> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        // Managed retains the exact frontend draft in the app-private capsule,
        // but its replacement observation is session-scoped and must never be
        // serialized. Crossing this same semantic capture boundary therefore
        // deliberately returns no durable authority payload for Managed.
        if slot.is_sparse_v2() {
            return Ok(None);
        }
        slot.with_filesystem_graph(|graph| {
            graph
                .capture_live_save_conflict(
                    &page,
                    base_rev.as_deref(),
                    tine_core::ConflictOverride {
                        observation_epoch: conflict_epoch,
                    },
                )
                .map(Some)
                .map_err(CommandError::from)
        })
    })
    .await
    .map_err(CommandError::worker)?
}

#[tauri::command]
pub(crate) async fn durable_live_save_conflict_diff(
    page: PageDto,
    base_text: Option<String>,
    state: GraphContext<'_>,
) -> Result<tine_core::sync_diff::SyncConflictDiff, CommandError> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        slot.with_filesystem_graph(|graph| {
            graph
                .durable_live_save_conflict_diff(&page, base_text.as_deref())
                .map_err(CommandError::from)
        })
    })
    .await
    .map_err(CommandError::worker)?
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ConflictCapsuleAuthority {
    DirectDurable { expected_disk_rev: String },
    DirectLive { conflict_epoch: u64 },
    Managed { path: String, revision: String },
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct ConflictCapsuleReview {
    diff: tine_core::sync_diff::SyncConflictDiff,
    authority: ConflictCapsuleAuthority,
}

/// Capsule pages become `Document`s through tine-core's single bounded
/// converter and merge their pre-blocks through its single union. Packet B3
/// briefly re-implemented both here over `BlockDto` (a recursive walker and a
/// `split_once("::")` union); that reinvented the depth bound packet F built
/// and diverged from Direct sync's property parsing (D-14).
fn capsule_document(page: &PageDto) -> Result<tine_core::Document, CommandError> {
    tine_core::model::page_dto_document(page)
        .map_err(|error| CommandError::coded("invalid retained conflict page", error.to_string()))
}

fn managed_capsule_current(
    handle: &SyncRuntimeHandle,
    page: &PageDto,
) -> Result<(PageDto, String), CommandError> {
    let selector = if page.path.is_empty() {
        SyncApplicationPageSelector::Logical {
            name: page.name.clone(),
            page_kind: page.kind.into(),
        }
    } else {
        SyncApplicationPageSelector::ExactPath {
            path: page.path.clone(),
        }
    };
    let current = load_sparse_page(handle, selector)?.ok_or_else(|| {
        CommandError::prose("managed conflict owner disappeared; reload before resolving")
    })?;
    let revision = current
        .rev
        .clone()
        .ok_or_else(|| CommandError::prose("managed conflict owner has no observable revision"))?;
    Ok((current, revision))
}

/// One semantic review surface for app-private conflict capsules. Managed
/// replacement authority is observed here and returned only to this live UI
/// review; it is never written into the durable capsule.
#[tauri::command]
pub(crate) async fn conflict_capsule_diff(
    page: PageDto,
    base_rev: Option<String>,
    conflict_epoch: i64,
    base_text: Option<String>,
    disk_rev: Option<String>,
    state: GraphContext<'_>,
) -> Result<ConflictCapsuleReview, CommandError> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        match sparse_application_handle(&slot)? {
            Some(handle) => {
                let (current, revision) = managed_capsule_current(handle, &page)?;
                let mine = capsule_document(&page)?;
                let theirs = capsule_document(&current)?;
                let mut diff = tine_core::sync_diff::diff_docs(&mine, &theirs);
                diff.base_rev = base_rev.unwrap_or_default();
                diff.conflict_rev = revision.clone();
                Ok(ConflictCapsuleReview {
                    diff,
                    authority: ConflictCapsuleAuthority::Managed {
                        path: current.path,
                        revision,
                    },
                })
            }
            None => {
                let graph = slot.legacy_graph()?;
                if let Some(expected_disk_rev) = disk_rev {
                    let diff = graph
                        .durable_live_save_conflict_diff(&page, base_text.as_deref())
                        .map_err(CommandError::from)?;
                    Ok(ConflictCapsuleReview {
                        diff,
                        authority: ConflictCapsuleAuthority::DirectDurable { expected_disk_rev },
                    })
                } else {
                    let conflict_epoch = u64::try_from(conflict_epoch).map_err(|_| {
                        CommandError::prose("direct conflict capsule has no live observation")
                    })?;
                    let diff = graph
                        .live_save_conflict_diff(
                            &page,
                            base_rev.as_deref(),
                            tine_core::ConflictOverride {
                                observation_epoch: conflict_epoch,
                            },
                        )
                        .map_err(CommandError::from)?;
                    Ok(ConflictCapsuleReview {
                        diff,
                        authority: ConflictCapsuleAuthority::DirectLive { conflict_epoch },
                    })
                }
            }
        }
    })
    .await
    .map_err(CommandError::worker)?
}

/// Apply decisions through the same semantic command for both storage modes.
/// The Managed branch re-proves the ephemeral observation in its actor turn;
/// Direct Files delegates to its existing one-shot/durable guards.
#[tauri::command]
pub(crate) async fn resolve_conflict_capsule(
    page: PageDto,
    base_rev: Option<String>,
    authority: ConflictCapsuleAuthority,
    decisions: std::collections::HashMap<String, String>,
    pre_choice: Option<String>,
    state: GraphContext<'_>,
) -> Result<PageDto, CommandError> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        match sparse_application_handle(&slot)? {
            Some(handle) => {
                let ConflictCapsuleAuthority::Managed { path, revision } = authority else {
                    return Err(CommandError::prose(
                        "managed conflict review authority is missing",
                    ));
                };
                let (current, observed_revision) = managed_capsule_current(handle, &page)?;
                if current.path != path || observed_revision != revision {
                    return Err(CommandError::prose(
                        "managed conflict owner changed; review the refreshed comparison",
                    ));
                }
                let mine = capsule_document(&page)?;
                let theirs = capsule_document(&current)?;
                let merged =
                    tine_core::sync_diff::merge_blocks(&mine.roots, &theirs.roots, &decisions)
                        .map_err(CommandError::from)?;
                let choice = pre_choice.as_deref().unwrap_or("union");
                let pre_block = match choice {
                    "theirs" => theirs.pre_block,
                    "mine" => mine.pre_block,
                    _ if page.format == tine_core::model::Format::Md => {
                        tine_core::model::union_pre(
                            mine.pre_block.as_deref(),
                            theirs.pre_block.as_deref(),
                        )
                    }
                    _ => mine.pre_block,
                };
                let mut resolved = page.clone();
                resolved.blocks = merged
                    .iter()
                    .map(tine_core::model::block_to_dto)
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(CommandError::from)?;
                resolved.pre_block = pre_block;
                resolved.path = current.path;
                resolved.format = current.format;
                resolved.read_only = current.read_only;
                resolved.activation = None;
                let saved_revision = save_sparse_page_with(
                    resolved.clone(),
                    base_rev,
                    true,
                    Some(ManagedConflictObservation { path, revision }),
                    |request| handle.save_application_page(request),
                )?;
                resolved.rev = Some(saved_revision);
                Ok(resolved)
            }
            None => {
                let graph = slot.legacy_graph()?;
                match authority {
                    ConflictCapsuleAuthority::DirectDurable { expected_disk_rev } => graph
                        .resolve_durable_live_save_conflict(
                            &page,
                            &expected_disk_rev,
                            &decisions,
                            pre_choice.as_deref().unwrap_or("union"),
                        )
                        .map_err(direct_save_error_message),
                    ConflictCapsuleAuthority::DirectLive { conflict_epoch } => graph
                        .resolve_live_save_conflict(
                            &page,
                            base_rev.as_deref(),
                            tine_core::ConflictOverride {
                                observation_epoch: conflict_epoch,
                            },
                            &decisions,
                            pre_choice.as_deref().unwrap_or("both"),
                        )
                        .map_err(direct_save_error_message),
                    ConflictCapsuleAuthority::Managed { .. } => Err(CommandError::prose(
                        "direct conflict review authority is missing",
                    )),
                }
            }
        }
    })
    .await
    .map_err(CommandError::worker)?
}

#[tauri::command]
pub(crate) async fn resolve_durable_live_save_conflict(
    page: PageDto,
    expected_disk_rev: String,
    decisions: std::collections::HashMap<String, String>,
    pre_choice: Option<String>,
    state: GraphContext<'_>,
) -> Result<PageDto, CommandError> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        let graph = slot.legacy_graph()?;
        graph
            .resolve_durable_live_save_conflict(
                &page,
                &expected_disk_rev,
                &decisions,
                pre_choice.as_deref().unwrap_or("union"),
            )
            .map_err(direct_save_error_message)
    })
    .await
    .map_err(CommandError::worker)?
}

/// Resolve a live Direct Files save conflict block-by-block, consuming the same
/// exact one-shot authority as the former Keep-mine action.
#[tauri::command]
pub(crate) async fn resolve_live_save_conflict(
    page: PageDto,
    base_rev: Option<String>,
    conflict_epoch: u64,
    decisions: std::collections::HashMap<String, String>,
    pre_choice: Option<String>,
    state: GraphContext<'_>,
) -> Result<PageDto, CommandError> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        let graph = slot.legacy_graph()?;
        graph
            .resolve_live_save_conflict(
                &page,
                base_rev.as_deref(),
                tine_core::ConflictOverride {
                    observation_epoch: conflict_epoch,
                },
                &decisions,
                pre_choice.as_deref().unwrap_or("both"),
            )
            .map_err(direct_save_error_message)
    })
    .await
    .map_err(CommandError::worker)?
}

/// Resolve a sync-conflict copy: merge it into its winner per the user's per-row
/// `decisions` (row id → "mine"/"theirs"/"both"/"merged") via the normal save path, then
/// trash the conflict copy. `base_rev` guards against the winner changing under
/// the merge; returns "conflict" if it did. `pre_choice`: "mine"/"theirs"/"union".
#[tauri::command]
pub(crate) async fn resolve_sync_conflict(
    winner: String,
    conflict: String,
    decisions: std::collections::HashMap<String, String>,
    base_rev: String,
    conflict_rev: String,
    merge_base_rev: Option<String>,
    pre_choice: Option<String>,
    state: GraphContext<'_>,
) -> Result<PageDto, CommandError> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        match sparse_application_handle(&slot)? {
            Some(handle) => {
                let winner_path = winner.clone();
                map_managed_sync_conflict_resolution(handle.mutate_application_graph(
                    SyncApplicationGraphMutationRequest::ResolveSyncConflict {
                        winner_path: winner,
                        conflict_path: conflict,
                        decisions,
                        base_revision: base_rev,
                        conflict_revision: conflict_rev,
                        pre_choice: pre_choice.unwrap_or_else(|| "union".into()),
                    },
                ))?;
                load_sparse_page(
                    handle,
                    SyncApplicationPageSelector::ExactPath {
                        path: winner_path.clone(),
                    },
                )?
                .ok_or_else(|| {
                    CommandError::prose(format!(
                        "resolved page disappeared after managed conflict merge: {winner_path}"
                    ))
                })
            }
            None => slot
                .legacy_graph()?
                .resolve_sync_conflict(
                    &winner,
                    &conflict,
                    &decisions,
                    &base_rev,
                    &conflict_rev,
                    merge_base_rev.as_deref(),
                    pre_choice.as_deref().unwrap_or("union"),
                )
                .map_err(direct_save_error_message),
        }
    })
    .await
    .map_err(CommandError::worker)?
}

/// Discard a sync-conflict copy without merging (move it to the recoverable
/// trash). Refuses anything that isn't a conflict copy.
#[tauri::command]
pub(crate) fn trash_sync_conflict(
    conflict: String,
    state: GraphContext<'_>,
) -> Result<(), CommandError> {
    with_trash_graph(&state, |g| {
        g.trash_sync_conflict(&conflict).map_err(CommandError::from)
    })
}

/// Move one journal file (by exact filename) to the recoverable trash.
#[tauri::command]
pub(crate) async fn trash_journal_file(
    name: String,
    state: GraphContext<'_>,
) -> Result<(), CommandError> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        match sparse_application_handle(&slot)? {
            Some(handle) => map_managed_graph_mutation(handle.mutate_application_graph(
                SyncApplicationGraphMutationRequest::TrashJournalFile { name },
            )),
            None => slot
                .legacy_graph()?
                .trash_journal_file(&name)
                .map_err(CommandError::from),
        }
    })
    .await
    .map_err(CommandError::worker)?
}

/// Raw contents of one journal file (by exact filename) — for inspecting a
/// duplicate day's files before reconciling.
#[tauri::command]
pub(crate) fn read_journal_file(
    name: String,
    state: GraphContext<'_>,
) -> Result<String, CommandError> {
    with_filesystem_graph(&state, |g| {
        g.read_journal_file(&name).map_err(CommandError::from)
    })
}

/// Load a page from a SPECIFIC file by its graph-root-relative path — lets the UI
/// navigate to a duplicate-day stray that shares a (kind,name) with the canonical
/// file and so is unreachable by name (#21).
#[tauri::command]
pub(crate) async fn get_page_by_path(
    path: String,
    state: GraphContext<'_>,
) -> Result<Option<PageDto>, CommandError> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        match sparse_application_handle(&slot)? {
            Some(handle) => {
                load_sparse_page(handle, SyncApplicationPageSelector::ExactPath { path })
            }
            None => slot
                .legacy_graph()?
                .load_by_path(&path)
                .map_err(CommandError::from),
        }
    })
    .await
    .map_err(CommandError::worker)?
}

/// Activate an editor over an existing file.
///
/// Deliberately separate from `get_page`/`get_page_by_path`. Those are
/// mixed-purpose reads — some results become store editors, others are read-only,
/// export, transient, or dropped because the page is already loaded — so minting
/// there would hand an identity to things that are not editors. An activation
/// exists exactly when a live editor does. (GH #254 increment 3.)
#[tauri::command]
pub(crate) async fn activate_editor(
    path: String,
    intent: tine_core::ActivationIntent,
    expected_revision: Option<String>,
    state: GraphContext<'_>,
) -> Result<Option<tine_core::EditorActivationHandle>, CommandError> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        match sparse_application_handle(&slot)? {
            Some(_) => Ok(None),
            None => slot
                .legacy_graph()?
                .activate_editor(&path, intent, expected_revision.as_deref())
                .map(Some)
                .map_err(CommandError::from),
        }
    })
    .await
    .map_err(CommandError::worker)?
}

/// Activate an editor for a page that has no file yet, returning the prospective
/// target it is live for. Reserves nothing on disk.
#[tauri::command]
pub(crate) async fn activate_absent_editor(
    name: String,
    kind: PageKind,
    state: GraphContext<'_>,
) -> Result<Option<tine_core::EditorActivationHandle>, CommandError> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        match sparse_application_handle(&slot)? {
            Some(_) => Ok(None),
            None => slot
                .legacy_graph()?
                .activate_absent_editor(&name, kind)
                .map(Some)
                .map_err(CommandError::from),
        }
    })
    .await
    .map_err(CommandError::worker)?
}

/// Present a conflict observation and learn its fate WITHOUT writing.
///
/// The "Use disk version" half of the authority contract. The frontend cannot
/// decide this locally: an observation can be revoked with no page event to react
/// to, so every local value still compares equal while the authority is already
/// gone. (GH #254 increment 3.)
#[tauri::command]
pub(crate) async fn present_conflict_override(
    path: String,
    base_rev: Option<String>,
    activation: u64,
    conflict_epoch: u64,
    state: GraphContext<'_>,
) -> Result<tine_core::ConflictPresentation, CommandError> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        slot.legacy_graph()?
            .present_conflict_override(&path, base_rev.as_deref(), activation, conflict_epoch)
            .map_err(CommandError::from)
    })
    .await
    .map_err(CommandError::worker)?
}

/// Retire an activation, but only if it is still the live one.
///
/// Compare-and-retire, never a bare "retire this path": a fire-and-forget
/// retirement can arrive after a newer activation was installed and would revoke
/// the wrong editor. Returns whether anything was retired, so a caller racing a
/// newer activation learns it was superseded instead of silently destroying it.
#[tauri::command]
pub(crate) async fn retire_editor_activation(
    path: String,
    activation: u64,
    state: GraphContext<'_>,
) -> Result<bool, CommandError> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        Ok(slot
            .legacy_graph()?
            .retire_editor_activation(&path, tine_core::EditorActivation::from_u64(activation)))
    })
    .await
    .map_err(CommandError::worker)?
}

#[cfg(test)]
mod application_page_authority_tests {
    use super::*;
    use std::cell::Cell;
    use tempfile::TempDir;
    use tine_core::model::Graph;
    use tine_core::sync_runtime::{
        SyncApplicationPageConflict, SyncApplicationPageRequestError, SyncEditorDeferred,
        SyncEditorRefusalCode, SyncPageKind,
    };

    fn page(name: &str, kind: PageKind, path: &str, raw: &str) -> PageDto {
        let mut block = BlockDto::default();
        block.id = format!("{name}-block");
        block.raw = raw.into();
        serde_json::from_value(serde_json::json!({
            "name": name,
            "kind": match kind {
                PageKind::Page => "page",
                PageKind::Journal => "journal",
            },
            "title": name,
            "pre_block": null,
            "blocks": [block],
            "rev": "frontend-revision",
            "path": path,
        }))
        .unwrap()
    }

    fn graph_with_files(files: &[(&str, &str)]) -> (TempDir, Graph) {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join("pages")).unwrap();
        std::fs::create_dir_all(temp.path().join("journals")).unwrap();
        for (relative, content) in files {
            let path = temp.path().join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, content).unwrap();
        }
        let graph = Graph::open(temp.path());
        (temp, graph)
    }

    fn inventory_entry(day: i64, path: &str) -> PageEntry {
        PageEntry {
            name: day.to_string(),
            kind: PageKind::Journal,
            date_key: Some(day),
            rel_path: path.into(),
            path: std::path::PathBuf::from(path),
        }
    }

    #[test]
    fn pdf_area_rollback_moves_the_real_nested_crop_to_typed_asset_trash() {
        let (temp, graph) = graph_with_files(&[]);
        let stored = graph
            .write_pdf_area_image("paper.pdf", 3, "area-id", 42, b"png")
            .unwrap();
        assert!(
            stored.contains('/'),
            "the fixture must exercise the nested OG layout"
        );
        let source = graph.assets_path().join(&stored);
        assert!(source.is_file());

        rollback_pdf_area_image_at(&graph, "paper.pdf", 3, "area-id", 42).unwrap();

        assert!(!source.exists());
        let trash = temp.path().join("logseq/.tine-trash/assets");
        let names = std::fs::read_dir(trash)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(names.len(), 1);
        assert!(names[0].contains("__pdf-area__"));
        assert!(names[0].ends_with("__3_area-id_42.png"));
    }

    #[test]
    fn sparse_load_mapping_sets_actor_revision_and_fails_closed() {
        let loaded = map_sparse_page_load(SyncApplicationPageLoadOutcome::Loaded {
            page: page("Loaded", PageKind::Page, "pages/Loaded.md", "- body"),
            revision: "actor-revision".into(),
        })
        .unwrap()
        .unwrap();
        assert_eq!(loaded.rev.as_deref(), Some("actor-revision"));

        assert!(
            map_sparse_page_load(SyncApplicationPageLoadOutcome::Missing { draft: None })
                .unwrap()
                .is_none()
        );
        let ambiguous =
            map_sparse_page_load(SyncApplicationPageLoadOutcome::Ambiguous).unwrap_err();
        let ambiguous = ambiguous.to_string();
        assert!(ambiguous.contains("could not identify this page"));
        assert!(ambiguous.contains("Reload it"));
        let deferred = map_sparse_page_load(SyncApplicationPageLoadOutcome::Deferred {
            state: SyncEditorDeferred::RetryableExternalWork,
        })
        .unwrap_err();
        let deferred = deferred.to_string();
        assert!(deferred.contains("updating this page"));
        assert!(deferred.contains("Try again"));
    }

    #[test]
    fn managed_capsule_adapter_applies_both_semantic_resolution_sides() {
        let draft = page(
            "Note",
            PageKind::Page,
            "pages/Note.md",
            "retained laptop draft",
        );
        let current = page(
            "Note",
            PageKind::Page,
            "pages/Note.md",
            "current phone body",
        );
        let mine = capsule_document(&draft).unwrap();
        let theirs = capsule_document(&current).unwrap();
        let diff = tine_core::sync_diff::diff_docs(&mine, &theirs);
        assert!(!diff.rows.is_empty());

        for (decision, expected) in [
            ("mine", "retained laptop draft"),
            ("theirs", "current phone body"),
        ] {
            let decisions = diff
                .rows
                .iter()
                .map(|row| (row.id.clone(), decision.to_owned()))
                .collect();
            let merged =
                tine_core::sync_diff::merge_blocks(&mine.roots, &theirs.roots, &decisions).unwrap();
            assert_eq!(merged.len(), 1);
            assert_eq!(merged[0].raw, expected);
            assert_eq!(
                tine_core::model::block_to_dto(&merged[0]).unwrap().raw,
                expected,
            );
        }
    }

    #[test]
    fn managed_capsule_authority_is_an_explicit_ephemeral_observation() {
        let authority = ConflictCapsuleAuthority::Managed {
            path: "pages/Note.md".into(),
            revision: "observed-after-restart".into(),
        };
        let encoded = serde_json::to_value(authority).unwrap();
        assert_eq!(encoded["kind"], "managed");
        assert_eq!(encoded["path"], "pages/Note.md");
        assert_eq!(encoded["revision"], "observed-after-restart");
    }

    #[test]
    fn sparse_inventory_mapping_preserves_full_parser_inventory_and_deferral() {
        let pages = vec![
            inventory_entry(20260729, "journals/2026_07_29.md"),
            inventory_entry(20260729, "journals/Jul 29th, 2026.md"),
        ];
        let loaded =
            map_sparse_page_inventory(SyncApplicationPageInventoryOutcome::Loaded { pages })
                .unwrap();
        assert_eq!(loaded.len(), 2, "inventory mapping must not deduplicate");
        assert_ne!(loaded[0].rel_path, loaded[1].rel_path);

        let deferred = map_sparse_page_inventory(SyncApplicationPageInventoryOutcome::Deferred {
            state: SyncEditorDeferred::RetryableExternalWork,
        })
        .unwrap_err();
        let deferred = deferred.to_string();
        assert!(deferred.contains("updating the page list"));
        assert!(deferred.contains("Try again"));
    }

    #[test]
    fn sparse_save_targets_existing_path_or_new_name_without_draft_token() {
        let nested_utf_path = "pages/研究/Crème brûlée.md";
        let existing = sparse_save_request(
            page("Crème brûlée", PageKind::Page, nested_utf_path, "- edited"),
            Some("actor-base".into()),
            false,
            None,
        )
        .unwrap();
        match existing.target {
            SyncApplicationPageSaveTarget::Existing { path, revision } => {
                assert_eq!(path, nested_utf_path);
                assert_eq!(revision, "actor-base");
            }
            other => panic!("expected exact existing target, got {other:?}"),
        }

        let new_page = sparse_save_request(
            page("New page", PageKind::Page, "", "- new"),
            None,
            false,
            None,
        )
        .unwrap();
        match &new_page.target {
            SyncApplicationPageSaveTarget::New { name, page_kind } => {
                assert_eq!(name, "New page");
                assert_eq!(*page_kind, SyncPageKind::Page);
            }
            other => panic!("expected new target, got {other:?}"),
        }
        let encoded = serde_json::to_value(&new_page.target).unwrap();
        assert!(encoded.get("revision").is_none());
        assert!(encoded.get("draft").is_none());
    }

    #[test]
    fn sparse_save_maps_saved_unchanged_conflict_and_routes_observed_force_to_actor() {
        let saved = map_sparse_page_save(SyncApplicationPageSaveOutcome::Saved {
            batch_id: "batch".into(),
            page: page("Saved", PageKind::Page, "pages/Saved.md", "- body"),
            revision: "saved-revision".into(),
        })
        .unwrap();
        assert_eq!(saved, "saved-revision");
        let unchanged = map_sparse_page_save(SyncApplicationPageSaveOutcome::Unchanged {
            page: page("Saved", PageKind::Page, "pages/Saved.md", "- body"),
            revision: "unchanged-revision".into(),
        })
        .unwrap();
        assert_eq!(unchanged, "unchanged-revision");
        let conflict = map_sparse_page_save(SyncApplicationPageSaveOutcome::Conflict {
            reason: SyncApplicationPageConflict::StaleBase,
        })
        .unwrap_err();
        let conflict = conflict.to_string();
        assert!(conflict.contains("conflict"));

        let called = Cell::new(false);
        let replaced = save_sparse_page_with(
            page("Saved", PageKind::Page, "pages/Saved.md", "- body"),
            Some("stale".into()),
            true,
            Some(ManagedConflictObservation {
                path: "pages/Saved.md".into(),
                revision: "observed-current".into(),
            }),
            |request| -> Result<SyncApplicationPageSaveOutcome, SyncApplicationPageRequestError> {
                called.set(true);
                assert!(matches!(
                    request.target,
                    SyncApplicationPageSaveTarget::ResolveConflict {
                        ref path,
                        ref observed_revision,
                    } if path == "pages/Saved.md" && observed_revision == "observed-current"
                ));
                Ok(SyncApplicationPageSaveOutcome::Saved {
                    batch_id: "replacement".into(),
                    page: request.page,
                    revision: "replacement-revision".into(),
                })
            },
        )
        .unwrap();
        assert_eq!(replaced, "replacement-revision");
        assert!(called.get());

        let new_page_replaced = save_sparse_page_with(
            page("Raced new", PageKind::Page, "", "- retained draft"),
            None,
            true,
            Some(ManagedConflictObservation {
                path: "pages/Raced new.md".into(),
                revision: "created-winner".into(),
            }),
            |request| -> Result<SyncApplicationPageSaveOutcome, SyncApplicationPageRequestError> {
                assert!(matches!(
                    request.target,
                    SyncApplicationPageSaveTarget::ResolveConflict {
                        ref path,
                        ref observed_revision,
                    } if path == "pages/Raced new.md" && observed_revision == "created-winner"
                ));
                Ok(SyncApplicationPageSaveOutcome::Saved {
                    batch_id: "new-page-replacement".into(),
                    page: request.page,
                    revision: "new-page-replacement-revision".into(),
                })
            },
        )
        .unwrap();
        assert_eq!(new_page_replaced, "new-page-replacement-revision");

        let unobserved = save_sparse_page_with(
            page("Saved", PageKind::Page, "pages/Saved.md", "- body"),
            Some("stale".into()),
            true,
            None,
            |_request| -> Result<SyncApplicationPageSaveOutcome, SyncApplicationPageRequestError> {
                unreachable!("an unobserved replacement must fail before actor invocation")
            },
        )
        .unwrap_err();
        let unobserved = unobserved.to_string();
        assert!(unobserved.contains("managed.conflict_unobserved"));
    }

    #[test]
    fn managed_save_debug_line_keeps_private_detail_out_of_public_error_rendering() {
        let detail = "Finalize: exact internal coordinator refusal";
        let error = SyncApplicationPageRequestError::ActorRefusedAtWithDebugDetail {
            stage: "committing the semantic page transaction",
            code: SyncEditorRefusalCode::TrustedLocalPreparationFinalize,
            debug_detail: detail.into(),
        };
        assert_eq!(
            error.to_string(),
            "sync actor refused application page intent at committing the semantic page transaction (reason code: trusted_local.preparation.finalize)"
        );
        assert!(!error.to_string().contains(detail));
        assert_eq!(
            managed_save_debug_detail_line(&error).as_deref(),
            Some(
                "managed storage save refusal detail: Finalize: exact internal coordinator refusal"
            )
        );
    }

    #[test]
    fn sparse_exact_selector_preserves_nested_utf_path() {
        let path = "journals/归档/2026_07_29–夜.md";
        let request = SyncApplicationPageLoadRequest {
            page: SyncApplicationPageSelector::ExactPath { path: path.into() },
        };
        match request.page {
            SyncApplicationPageSelector::ExactPath { path: actual } => {
                assert_eq!(actual, path);
            }
            other => panic!("expected exact-path selector, got {other:?}"),
        }
    }

    #[test]
    fn sparse_feed_inventory_matches_legacy_order_and_cutoff() {
        let (_temp, graph) = graph_with_files(&[
            ("journals/2026_07_29.md", "- newest\n"),
            ("journals/2026_07_28.md", "- older\n"),
            ("journals/2099_01_01.md", "- future\n"),
            ("pages/ordinary.md", "- page\n"),
        ]);
        let cutoff = 20260729;
        let sparse = journal_feed_inventory(graph.list_pages(), cutoff)
            .into_iter()
            .map(|entry| entry.rel_path)
            .collect::<Vec<_>>();
        let legacy = graph
            .feed_journals_desc_through(JournalDate::from_ordinal(cutoff))
            .into_iter()
            .map(|entry| entry.rel_path)
            .collect::<Vec<_>>();
        assert_eq!(sparse, legacy);
    }

    #[test]
    fn sparse_feed_prefers_canonical_duplicate_and_bounds_page_loads() {
        let entries = vec![
            inventory_entry(20260729, "journals/Jul 29th, 2026.md"),
            inventory_entry(20260729, "journals/2026_07_29.md"),
            inventory_entry(20260728, "journals/2026_07_28.md"),
        ];
        let inventory = journal_feed_inventory(entries, 20260729);
        assert_eq!(
            inventory
                .iter()
                .map(|entry| entry.rel_path.as_str())
                .collect::<Vec<_>>(),
            ["journals/2026_07_29.md", "journals/2026_07_28.md"]
        );
        let loads = Cell::new(0);
        let feed = collect_journal_feed_page(inventory.into_iter(), 1, |entry| {
            loads.set(loads.get() + 1);
            Ok(page(
                &entry.name,
                PageKind::Journal,
                &entry.rel_path,
                "- content",
            ))
        })
        .unwrap();
        assert_eq!(feed.pages.len(), 1);
        assert_eq!(loads.get(), 1);
        assert_eq!(feed.next_before_day, Some(20260729));
    }

    #[test]
    fn sparse_content_detection_matches_legacy_and_includes_nested_blocks() {
        let (_temp, graph) = graph_with_files(&[
            (
                "journals/2026_07_29.md",
                "- property:: only\n  - nested content\n",
            ),
            ("journals/2026_07_28.md", "- property:: only\n"),
            ("journals/2026_07_27.md", "-    \n"),
            ("pages/ordinary.md", "- ordinary\n"),
        ]);
        let legacy = graph.journal_content_days();
        let mut mapped = Vec::new();
        for entry in graph.list_pages() {
            if entry.kind != PageKind::Journal {
                continue;
            }
            let Some(day) = entry.date_key else {
                continue;
            };
            let loaded = graph.load_by_path(&entry.rel_path).unwrap().unwrap();
            if application_blocks_have_content(&loaded.blocks) {
                mapped.push(day);
            }
        }
        assert_eq!(mapped, legacy);
        assert!(mapped.contains(&20260729));
        assert!(!mapped.contains(&20260728));
        assert!(!mapped.contains(&20260727));
    }

    #[test]
    fn save_response_keeps_managed_compatibility_and_serializes_direct_activation() {
        let managed = serde_json::to_value(SavePageResult {
            revision: "managed-revision".into(),
            activation: None,
        })
        .unwrap();
        assert_eq!(
            managed,
            serde_json::json!({ "revision": "managed-revision" })
        );

        let direct = serde_json::to_value(SavePageResult {
            revision: "direct-revision".into(),
            activation: Some(tine_core::EditorActivationHandle {
                activation: tine_core::EditorActivation::from_u64(41),
                target: "pages/New.md".into(),
                prospective: false,
            }),
        })
        .unwrap();
        assert_eq!(direct["activation"]["activation"], 41);
        assert_eq!(direct["activation"]["target"], "pages/New.md");
        assert_eq!(direct["activation"]["prospective"], false);
    }

    #[test]
    fn legacy_page_load_and_unchanged_save_helpers_retain_their_contract() {
        let (_temp, graph) = graph_with_files(&[("pages/legacy.md", "- unchanged legacy body\n")]);
        let loaded = graph.load_named("legacy", PageKind::Page).unwrap().unwrap();
        let base = loaded.rev.clone().unwrap();
        assert_eq!(
            graph.save_page(&loaded, Some(&base)).unwrap(),
            base,
            "unchanged legacy saves still return the on-disk revision"
        );
        assert_eq!(graph.list_pages().len(), 1);
    }
}

/// Reconcile a duplicate-day pair: append the blocks of `src` to `dst`, then trash
/// `src` (both graph-root-relative paths). The merged `dst` is written through the
/// normal round-tripping save path (#21).
#[tauri::command]
pub(crate) async fn merge_pages(
    src: String,
    dst: String,
    rename_from: Option<String>,
    rename_to: Option<String>,
    state: GraphContext<'_>,
) -> Result<(), CommandError> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        match sparse_application_handle(&slot)? {
            Some(handle) => map_managed_graph_mutation(handle.mutate_application_graph(
                SyncApplicationGraphMutationRequest::MergePages {
                    source_path: src,
                    destination_path: dst,
                    rename_from,
                    rename_to,
                },
            )),
            None => match (rename_from.as_deref(), rename_to.as_deref()) {
                (Some(old), Some(new)) => slot
                    .legacy_graph()?
                    .merge_pages_after_rename(&src, &dst, old, new)
                    .map_err(CommandError::from),
                (None, None) => slot
                    .legacy_graph()?
                    .merge_pages(&src, &dst)
                    .map_err(CommandError::from),
                _ => Err(CommandError::prose(
                    "merge rename requires both source and destination names",
                )),
            },
        }
    })
    .await
    .map_err(CommandError::worker)?
}

/// Rescue a duplicate-day stray by moving it to a uniquely-named page
/// (`pages/<new_name>`), so it stops colliding and becomes normally navigable (#21).
#[tauri::command]
pub(crate) async fn rename_file_to_page(
    path: String,
    new_name: String,
    state: GraphContext<'_>,
) -> Result<(), CommandError> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        match sparse_application_handle(&slot)? {
            Some(handle) => map_managed_graph_mutation(handle.mutate_application_graph(
                SyncApplicationGraphMutationRequest::RenameFileToPage { path, new_name },
            )),
            None => slot
                .legacy_graph()?
                .rename_file_to_page(&path, &new_name)
                .map_err(CommandError::from),
        }
    })
    .await
    .map_err(CommandError::worker)?
}

#[tauri::command]
pub(crate) fn save_asset(
    name: String,
    bytes_b64: String,
    state: GraphContext<'_>,
) -> Result<String, CommandError> {
    let bytes = decode_asset_b64(&bytes_b64)?;
    let window_label = state.window.label().to_string();
    with_filesystem_graph(&state, |g| {
        let stored = g.save_asset(&name, &bytes).map_err(CommandError::from)?;
        crate::watcher::note_asset_self_write(&window_label, &g.assets_path().join(&stored));
        Ok(stored)
    })
}

#[tauri::command]
pub(crate) fn read_highlights(
    pdf: String,
    state: GraphContext<'_>,
) -> Result<Vec<tine_core::pdf::Highlight>, CommandError> {
    with_filesystem_graph(&state, |g| Ok(g.read_highlights(&pdf)))
}

#[tauri::command]
pub(crate) async fn open_pdf(
    pdf: String,
    label: String,
    state: GraphContext<'_>,
) -> Result<tine_core::pdf::PdfState, CommandError> {
    let (app, window_label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &window_label, Some(binding_generation))?;
        match sparse_application_handle(&slot)? {
            Some(handle) => match handle
                .open_application_pdf(pdf, label)
                .map_err(CommandError::from)?
            {
                SyncApplicationPdfOpenOutcome::Ready { state } => Ok(state),
                SyncApplicationPdfOpenOutcome::Deferred { .. } => Err(CommandError::prose(
                    "Tine-managed storage is updating PDF notes. Try again when it finishes.",
                )),
            },
            None => slot
                .legacy_graph()?
                .open_pdf(&pdf, &label)
                .map_err(CommandError::from),
        }
    })
    .await
    .map_err(CommandError::worker)?
}

#[tauri::command]
pub(crate) async fn write_highlights(
    pdf: String,
    label: String,
    highlights: Vec<tine_core::pdf::Highlight>,
    base_ids: Vec<String>,
    state: GraphContext<'_>,
) -> Result<(), CommandError> {
    let (app, window_label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &window_label, Some(binding_generation))?;
        match sparse_application_handle(&slot)? {
            Some(handle) => match handle
                .write_application_pdf_highlights(pdf, label, highlights, base_ids)
                .map_err(CommandError::from)?
            {
                SyncApplicationUnitOutcome::Applied => Ok(()),
                SyncApplicationUnitOutcome::Deferred { .. } => Err(CommandError::prose(
                    "Tine-managed storage is updating PDF notes. Try again when it finishes.",
                )),
            },
            None => slot
                .legacy_graph()?
                .write_highlights(&pdf, &label, &highlights, &base_ids)
                .map_err(CommandError::from),
        }
    })
    .await
    .map_err(CommandError::worker)?
}

#[tauri::command]
pub(crate) async fn write_pdf_view_state(
    pdf: String,
    page: i64,
    scale: f64,
    state: GraphContext<'_>,
) -> Result<(), CommandError> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        match sparse_application_handle(&slot)? {
            Some(handle) => match handle
                .write_application_pdf_view_state(pdf, page, scale)
                .map_err(CommandError::from)?
            {
                SyncApplicationUnitOutcome::Applied => Ok(()),
                SyncApplicationUnitOutcome::Deferred { .. } => Err(CommandError::prose(
                    "Tine-managed storage is updating PDF state. Try again when it finishes.",
                )),
            },
            None => slot
                .legacy_graph()?
                .write_pdf_view_state(&pdf, page, scale)
                .map_err(CommandError::from),
        }
    })
    .await
    .map_err(CommandError::worker)?
}

#[tauri::command]
pub(crate) fn save_pdf_area_image(
    pdf: String,
    page: i64,
    id: String,
    stamp: i64,
    bytes_b64: String,
    state: GraphContext<'_>,
) -> Result<String, CommandError> {
    let bytes = decode_asset_b64(&bytes_b64)?;
    let window_label = state.window.label().to_string();
    with_filesystem_graph(&state, |g| {
        let stored = g
            .write_pdf_area_image(&pdf, page, &id, stamp, &bytes)
            .map_err(CommandError::from)?;
        crate::watcher::note_asset_self_write(&window_label, &g.assets_path().join(&stored));
        Ok(stored)
    })
}

fn rollback_pdf_area_image_at(
    graph: &tine_core::model::Graph,
    pdf: &str,
    page: i64,
    id: &str,
    stamp: i64,
) -> Result<(), CommandError> {
    graph
        .rollback_pdf_area_image(pdf, page, id, stamp)
        .map_err(CommandError::from)
}

#[tauri::command]
pub(crate) fn rollback_pdf_area_image(
    pdf: String,
    page: i64,
    id: String,
    stamp: i64,
    state: GraphContext<'_>,
) -> Result<(), CommandError> {
    let window_label = state.window.label().to_string();
    with_trash_graph(&state, |graph| {
        let source = graph
            .assets_path()
            .join(tine_core::pdf::asset_key(&pdf))
            .join(format!("{page}_{id}_{stamp}.png"));
        rollback_pdf_area_image_at(graph, &pdf, page, &id, stamp)?;
        crate::watcher::note_asset_self_write(&window_label, &source);
        Ok(())
    })
}

#[cfg(test)]
mod direct_save_error_tests {
    use super::direct_save_error_message;
    use std::io;
    use tine_core::model::{DirectSaveError, DirectSaveFailureCode};

    fn code(value: &str) -> DirectSaveFailureCode {
        DirectSaveFailureCode::ALL
            .into_iter()
            .find(|code| code.as_str() == value)
            .unwrap_or_else(|| panic!("missing DirectSaveFailureCode for {value}"))
    }

    fn payload(
        value: &str,
        kind: io::ErrorKind,
        epoch: Option<u64>,
        message: &str,
    ) -> serde_json::Value {
        let source = io::Error::new(kind, message.to_owned());
        let error = DirectSaveError::into_io_with_conflict_epoch(code(value), epoch, source);
        serde_json::from_str(&direct_save_error_message(error).to_string()).unwrap()
    }

    /// The frontend puts up a conflict prompt ("Keep mine" / "Use disk version")
    /// for exactly one message, and a page it marks conflicted stops saving until
    /// the user resolves it. So the set of failures that produce that message is
    /// a contract, not a formatting detail: anything in it that the two buttons
    /// cannot resolve strands the page.
    #[test]
    fn only_a_real_base_revision_conflict_raises_the_conflict_prompt() {
        assert_eq!(
            payload(
                "conflict.base_rev",
                io::ErrorKind::AlreadyExists,
                None,
                "conflict",
            ),
            serde_json::json!({
                "kind": "save-conflict",
                "reason_code": "conflict.base_rev",
                "detail": { "io_error_kind": "AlreadyExists", "epoch": null },
            })
        );

        for (message, expected_code) in [
            (
                "graph text paths share one portable case/NFC identity: pages/foo.md and pages/Foo.md",
                "precheck.portable_collision",
            ),
            (
                "graph text files alias one physical resource: pages/a.md and pages/b.md",
                "precheck.resource_alias",
            ),
            (
                "managed text entry is a symlink or reparse point: pages/Alias.md",
                "precheck.symlink",
            ),
            (
                "existing page identity changed since load",
                "identity.changed_since_load",
            ),
            (
                "another graph document owns this effective page identity",
                "identity.owned_elsewhere",
            ),
            (
                "a page with that name already exists",
                "identity.name_taken",
            ),
            (
                "target page exists in another supported text extension",
                "identity.name_taken",
            ),
            // The class this contract exists to keep out: an unclassified
            // AlreadyExists. It used to reach a `conflict.other` catch-all, so
            // a failure that had PRESERVED the user's bytes under a recovery
            // name was reported as a bare "conflict" -- the one message that
            // both hides the retention text and offers a "use disk" button
            // that throws those very edits away.
            (
                "displaced target retained as pages/Note.md.editor-recovery",
                "unknown",
            ),
        ] {
            let reported = payload(
                expected_code,
                io::ErrorKind::AlreadyExists,
                None,
                message,
            );
            assert_eq!(reported["kind"], "direct-save-failure");
            assert_eq!(reported["reason_code"], expected_code);
        }
    }

    /// The counterpart: a page whose file moved between load and save IS a
    /// content conflict, and since `011658a9` "keep mine" can actually resolve
    /// it. It must reach the prompt.
    #[test]
    fn an_unobserved_external_change_still_raises_the_conflict_prompt() {
        let reported = payload(
            "conflict.pinned_owner",
            io::ErrorKind::AlreadyExists,
            Some(17),
            "path-pinned page does not match its captured exact owner",
        );
        assert_eq!(reported["kind"], "save-conflict");
        assert_eq!(reported["reason_code"], "conflict.pinned_owner");
        assert_eq!(reported["detail"]["epoch"], 17);
    }

    #[test]
    fn every_minted_site_and_no_tokenless_site_reaches_the_banner() {
        for code in DirectSaveFailureCode::ALL {
            let value = code.as_str();
            let conflict = value.starts_with("conflict.");
            let reported = payload(
                value,
                io::ErrorKind::Other,
                conflict.then_some(1),
                "display text is not classification data",
            );
            assert_eq!(reported["reason_code"], value);
            assert_eq!(
                reported["kind"],
                if conflict {
                    "save-conflict"
                } else {
                    "direct-save-failure"
                }
            );
        }

        for (code, message) in [
            (
                "conflict.save_baseline_present",
                "editor conflict: save baseline present",
            ),
            (
                "conflict.save_baseline_absent",
                "editor conflict: save baseline absent",
            ),
            ("conflict.commit_recheck", "editor conflict: commit recheck"),
            (
                "conflict.replace_pre_retirement",
                "editor conflict: replace pre-retirement",
            ),
            (
                "conflict.replace_retired_mismatch",
                "editor conflict: retired mismatch",
            ),
            (
                "conflict.replace_publication_collision",
                "editor conflict: publication collision",
            ),
            (
                "conflict.create_publication_collision",
                "editor conflict: create publication collision",
            ),
            (
                "conflict.final_reread_absent",
                "editor conflict: final reread absent",
            ),
            (
                "conflict.final_reread_present",
                "editor conflict: final reread present",
            ),
            (
                "conflict.replace_post_publication",
                "editor conflict: post-publication validation",
            ),
        ] {
            let reported = payload(code, io::ErrorKind::AlreadyExists, Some(1), message);
            assert_eq!(reported["kind"], "save-conflict");
            assert_eq!(reported["reason_code"], code);
        }
        for (code, message) in [
            (
                "conflict_retry.commit_recheck",
                "tokenless editor conflict: commit recheck: continued churn",
            ),
            (
                "conflict_retry.replace_pre_retirement",
                "tokenless editor conflict: replace pre-retirement: transient I/O",
            ),
            (
                "conflict_retry.final_reread_present",
                "tokenless editor conflict: final reread present: transient I/O",
            ),
        ] {
            let reported = payload(code, io::ErrorKind::WouldBlock, None, message);
            assert_eq!(reported["kind"], "direct-save-failure");
            assert_eq!(reported["reason_code"], code);
        }
    }
}

#[cfg(test)]
mod query_command_surface_tests {
    //! SPEC §7.1, O12. The six commands are `#[tauri::command]` wrappers around
    //! decisions made in the helpers above; those decisions are what a test can
    //! actually pin, and they are what would be wrong. The routing half is
    //! pinned by `managed_command_surface`, which now classifies all six.

    use super::*;
    use tine_core::query::ir::{Query, ViewSettings};

    fn graph_free_registry() -> tine_core::query::registry::Registry {
        tine_core::query::registry::Registry::from_snapshot(
            &tine_core::query::ir::RegistrySnapshot {
                rows: Vec::new(),
                generation: 0,
            },
        )
    }

    fn parsed(text: &str, dialect: QueryTextDialect) -> ParsedQuery {
        parse_query_pair(text, dialect, &[], &graph_free_registry())
    }

    #[test]
    fn query_parse_returns_the_pair_for_both_dialects() {
        let og = parsed("(and (task TODO) [[Project]])", QueryTextDialect::Og);
        assert!(!og.query.is_invalid(), "{:?}", og.query.diagnostics);
        // OG's `(task TODO)` is a marker SET, so its TQL spelling is `in`;
        // `task = 'TODO'` is the one-marker special case and a different node.
        let tql = parsed("task in ('TODO') and [[Project]]", QueryTextDialect::Tql);
        assert!(!tql.query.is_invalid(), "{:?}", tql.query.diagnostics);
        assert_eq!(
            og.query.normalized().filter,
            tql.query.normalized().filter,
            "one IR, two spellings"
        );
    }

    #[test]
    fn query_parse_reports_an_unknown_head_instead_of_a_shorter_query() {
        let parsed = parsed("(and (task TODO) (frobnicate x))", QueryTextDialect::Og);
        assert!(parsed
            .query
            .diagnostics
            .iter()
            .any(|d| d.kind == tine_core::query::ir::DiagnosticKind::UnknownHead));
    }

    #[test]
    fn query_parse_merges_the_host_blocks_view_properties() {
        let merged = parse_query_pair(
            "(and (task TODO) (sort-by page asc))",
            QueryTextDialect::Og,
            &[("tine.sample".to_string(), "5".to_string())],
            &graph_free_registry(),
        );
        assert_eq!(merged.view.sample, Some(5));
        assert!(
            !merged.view.sort.is_empty(),
            "the directive survives where no property covers it"
        );
    }

    #[test]
    fn query_print_prints_tql_for_every_ir_and_og_where_it_can() {
        let parsed = parsed("(and (task TODO) [[Project]])", QueryTextDialect::Og);
        let tql = print_query_text(&parsed.query, &parsed.view, QueryPrintDialect::Tql, false)
            .expect("TQL is total");
        assert!(tql.contains("[[Project]]"), "{tql}");
        let og = print_query_text(&parsed.query, &parsed.view, QueryPrintDialect::Og, false)
            .expect("this filter is OG-expressible");
        assert!(og.starts_with('('), "{og}");
    }

    /// A4: the OG printer is partial and REJECTS, carrying the whole
    /// diagnostic. Never an empty string, never a stringified message.
    #[test]
    fn query_print_rejects_a_non_og_expressible_ir_with_the_diagnostic() {
        let parsed = parsed("any(children, task = 'DONE')", QueryTextDialect::Tql);
        assert!(!parsed.query.is_invalid(), "{:?}", parsed.query.diagnostics);
        assert!(
            !tine_core::query::print::og_expressible(&parsed.query, &parsed.view),
            "the fixture must be a filter the OG DSL cannot say"
        );
        let error = print_query_text(&parsed.query, &parsed.view, QueryPrintDialect::Og, false)
            .expect_err("the OG printer is partial");
        let wire = serde_json::to_string(&error).expect("the rejection serializes");
        assert!(wire.contains("query-print-refused"), "{wire}");
        assert!(wire.contains("not_applicable"), "{wire}");
        assert!(
            wire.contains("not_applicable\\\",\\\"message") || wire.contains("message"),
            "the diagnostic travels as structure, not as prose: {wire}"
        );
    }

    #[test]
    fn query_og_expressible_separates_the_two_printers() {
        let og = parsed("(and (task TODO) [[Project]])", QueryTextDialect::Og);
        assert!(tine_core::query::print::og_expressible(&og.query, &og.view));
        let tql_only = parsed("any(children, task = 'DONE')", QueryTextDialect::Tql);
        assert!(!tine_core::query::print::og_expressible(
            &tql_only.query,
            &tql_only.view
        ));
    }

    /// `query_registry` returns whatever the bound storage mode published, and
    /// a registry rebuilt from that wire shape answers suggestions with it.
    #[test]
    fn the_registry_snapshot_round_trips_through_the_wire_shape() {
        let snapshot = tine_core::query::ir::RegistrySnapshot {
            rows: vec![tine_core::query::ir::RegistryRow {
                normalized_name: "status".into(),
                cardinality: tine_core::query::ir::Cardinality::One,
                observed_type: tine_core::query::ir::ObservedType::Text,
                count_blocks: 2,
                count_pages: 0,
                histogram: Vec::new(),
                mismatch_count: 0,
                declared: None,
                top_values: Vec::new(),
            }],
            generation: 7,
        };
        let registry = tine_core::query::registry::Registry::from_snapshot(&snapshot);
        assert_eq!(registry.generation(), 7);
        assert_eq!(registry.snapshot(), snapshot);
        let parsed = parse_query_pair("statuss = 'x'", QueryTextDialect::Tql, &[], &registry);
        assert_eq!(
            parsed
                .query
                .diagnostics
                .iter()
                .find(|d| d.kind == tine_core::query::ir::DiagnosticKind::UnknownIdent)
                .map(|d| d.suggestions.clone()),
            Some(vec!["prop('status')".to_string()]),
            "the registry the command fetched is the one the parse reads"
        );
    }

    /// `query_run` never truncates: an over-budget answer is a refusal with the
    /// count, the same rule `run_query` applies.
    #[test]
    fn query_run_refuses_an_over_budget_result_rather_than_truncating_it() {
        let result = tine_core::query::ir::QueryResult {
            rows: tine_core::query::ir::QueryRows::Block { groups: Vec::new() },
            diagnostics: Vec::new(),
            report: tine_core::query::ir::QueryReport {
                supported: true,
                ..Default::default()
            },
            total: 99_999,
            exceeded: true,
        };
        let error = query_result_or_error(result).expect_err("an exceeded result is a refusal");
        let wire = serde_json::to_string(&error).expect("the rejection serializes");
        assert!(wire.contains("result-too-large"), "{wire}");
        assert!(wire.contains("99999"), "{wire}");
    }

    #[test]
    fn query_run_passes_a_result_within_budget_through_unchanged() {
        let result = tine_core::query::ir::QueryResult {
            rows: tine_core::query::ir::QueryRows::Page { pages: Vec::new() },
            diagnostics: Vec::new(),
            report: tine_core::query::ir::QueryReport {
                supported: true,
                ..Default::default()
            },
            total: 3,
            exceeded: false,
        };
        let passed = query_result_or_error(result).expect("within budget");
        assert_eq!(passed.total, 3);
        assert!(matches!(
            passed.rows,
            tine_core::query::ir::QueryRows::Page { .. }
        ));
    }

    /// `query_explain_empty` (N19): a root `And` explains per conjunct with a
    /// `without` count; anything else explains as a whole and has none.
    #[test]
    fn query_explain_empty_answers_per_conjunct_only_for_a_root_and() {
        let dir = std::env::temp_dir().join(format!("tine-query-explain-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("pages")).unwrap();
        std::fs::write(
            dir.join("pages/Explain.md"),
            "- TODO a task with no project\n- a project mention [[Project]]\n",
        )
        .unwrap();
        let graph = tine_core::model::Graph::open(&dir);
        graph.warm_cache();
        let bounds = tine_core::query::ir::Bounds::unbounded();

        let conjunction = parsed("(and (task TODO) [[Project]])", QueryTextDialect::Og);
        let context = tine_core::query::ir::ExecutionContext::none();
        let explained = tine_core::query::explain_empty_query(
            &graph,
            &conjunction.query,
            &conjunction.view,
            bounds,
            &context,
        );
        let lines = &explained.rows;
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(
            lines.iter().all(|line| line.without.is_some()),
            "every conjunct of a root `And` reports what the others match: {lines:?}"
        );
        assert!(
            lines.iter().any(|line| line.alone == 1),
            "each conjunct matches a row on its own: {lines:?}"
        );
        assert!(
            explained.report.supported && explained.report.ignored.is_empty(),
            "an OG source reports supported with nothing ignored (§4.4): {:?}",
            explained.report
        );

        let single = parsed("(task TODO)", QueryTextDialect::Og);
        let explained = tine_core::query::explain_empty_query(
            &graph,
            &single.query,
            &single.view,
            bounds,
            &context,
        );
        let lines = &explained.rows;
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert_eq!(lines[0].without, None, "there is no `other` to be without");
        assert_eq!(lines[0].alone, 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    fn parse_and_run(graph: &tine_core::model::Graph, text: &str) -> Vec<String> {
        let parsed = parsed(text, QueryTextDialect::Tql);
        let result = tine_core::query::run_query_result_ir(
            graph,
            &parsed.query,
            &parsed.view,
            tine_core::query::ir::Bounds::unbounded(),
            &tine_core::query::ir::ExecutionContext::none(),
        );
        match result.rows {
            tine_core::query::ir::QueryRows::Page { pages } => {
                pages.into_iter().map(|page| page.name).collect()
            }
            tine_core::query::ir::QueryRows::Block { groups } => groups
                .into_iter()
                .flat_map(|group| group.blocks.into_iter())
                .map(|block| {
                    block
                        .raw
                        .lines()
                        .next()
                        .unwrap_or_default()
                        .trim()
                        .to_string()
                })
                .collect(),
        }
    }

    /// K16: a `@page` query answers with page rows and never loads a document.
    #[test]
    fn query_run_answers_page_rows_for_a_page_anchored_query() {
        let dir = std::env::temp_dir().join(format!("tine-query-run-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("pages")).unwrap();
        std::fs::create_dir_all(dir.join("journals")).unwrap();
        std::fs::write(dir.join("pages/Proj%2FSub.md"), "- under a namespace\n").unwrap();
        std::fs::write(dir.join("pages/Other.md"), "- elsewhere\n").unwrap();
        let graph = tine_core::model::Graph::open(&dir);
        graph.warm_cache();

        assert_eq!(
            parse_and_run(&graph, "@page and name like 'proj/%'"),
            vec!["Proj/Sub".to_string()]
        );
        assert_eq!(
            parse_and_run(&graph, "content like '%namespace%'"),
            vec!["under a namespace".to_string()]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
