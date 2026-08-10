#[cfg(desktop)]
use crate::debug::diag;
#[cfg(desktop)]
use crate::platform::{open_page_source, opener_command, reveal_page_source};
use crate::state::{
    capture_quick_switch_slot, owned_graph_context, refresh_graph, slot_for_bound_window,
    slot_for_context, with_config_graph, with_filesystem_graph, with_graph, with_trash_graph,
    AppState, GraphContext,
};
use serde::Serialize;
use std::sync::Arc;
use std::time::Instant;
use tauri::{Emitter, Manager, State, WebviewWindow};
use tine_core::date::JournalDate;
use tine_core::model::{
    BacklinkFilterContext, BacklinkFilterTarget, BlockDto, PageDto, PageEntry, PageKind, RefGroup,
};
use tine_core::sync_runtime::{
    SyncApplicationGraphMutationRequest, SyncApplicationGuideCopyOutcome,
    SyncApplicationNavigationOutcome, SyncApplicationNavigationReply,
    SyncApplicationNavigationRequest, SyncApplicationPageInventoryOutcome,
    SyncApplicationPageLoadOutcome, SyncApplicationPageLoadRequest, SyncApplicationPageSaveOutcome,
    SyncApplicationPageSaveRequest, SyncApplicationPageSaveTarget, SyncApplicationPageSelector,
    SyncApplicationPdfOpenOutcome, SyncApplicationPublishOutcome, SyncApplicationUnitOutcome,
    SyncRuntimeHandle,
};

#[tauri::command]
pub(crate) fn load_workspaces(
    app: tauri::AppHandle,
    state: GraphContext<'_>,
) -> Result<String, String> {
    crate::settings::load_workspaces(app, state)
}

#[tauri::command]
pub(crate) fn save_workspaces(
    data: String,
    app: tauri::AppHandle,
    state: GraphContext<'_>,
) -> Result<(), String> {
    crate::settings::save_workspaces(data, app, state)
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

fn validate_query_source(query: &str) -> Result<(), String> {
    if !tine_core::query::query_source_within_limit(query) {
        return Err(format!(
            "query-too-large: query source is {} bytes (limit: {} bytes)",
            query.len(),
            tine_core::query::QUERY_SOURCE_MAX_BYTES
        ));
    }
    if !tine_core::query::query_nesting_within_limit(query) {
        return Err("query-nesting-too-deep: simplify nested boolean clauses".to_string());
    }
    Ok(())
}

fn enforce_result_bridge_budget(groups: &[RefGroup]) -> Result<(), String> {
    let rows = groups.iter().map(|group| group.blocks.len()).sum::<usize>();
    let bytes = tine_core::model::ref_groups_estimated_bytes(groups);
    if rows > RESULT_BRIDGE_MAX_ROWS || bytes > RESULT_BRIDGE_MAX_BYTES {
        return Err(format!(
            "result-too-large: {rows} matching blocks (~{bytes} bytes); narrow the query or add (sample N) (limits: {RESULT_BRIDGE_MAX_ROWS} blocks / {RESULT_BRIDGE_MAX_BYTES} bytes)"
        ));
    }
    Ok(())
}

fn enforce_optional_result_bridge_budget(groups: &[Option<RefGroup>]) -> Result<(), String> {
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
        return Err(format!(
            "result-too-large: {rows} matching blocks (~{bytes} bytes); narrow the query or add (sample N) (limits: {RESULT_BRIDGE_MAX_ROWS} blocks / {RESULT_BRIDGE_MAX_BYTES} bytes)"
        ));
    }
    Ok(())
}

fn bounded_groups_or_error(
    result: tine_core::model::BoundedRefGroups,
) -> Result<Arc<Vec<RefGroup>>, String> {
    if result.exceeded {
        return Err(format!(
            "result-too-large: {} matching blocks; narrow the query or add (sample N) (construction limits: {RESULT_BRIDGE_MAX_ROWS} blocks / {RESULT_BRIDGE_MAX_BYTES} bytes)",
            result.total
        ));
    }
    Ok(result.groups)
}

fn enforce_query_execution_budget(
    execution: &tine_core::query_plan::QueryExecution,
) -> Result<(), String> {
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
        return Err(format!(
            "result-too-large: {} search hits (~{bytes} bytes); narrow the search (limits: {RESULT_BRIDGE_MAX_ROWS} hits / {RESULT_BRIDGE_MAX_BYTES} bytes)",
            execution.hits.len()
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
            .starts_with("result-too-large:"));
    }

    #[test]
    fn rejects_oversized_result_bytes_before_ipc() {
        let mut block = BlockDto::default();
        block.raw = "x".repeat(RESULT_BRIDGE_MAX_BYTES + 1);
        assert!(enforce_result_bridge_budget(&[group(vec![block])])
            .unwrap_err()
            .starts_with("result-too-large:"));
    }

    #[test]
    fn rejects_oversized_query_source_before_cache_or_parser() {
        let source = "x".repeat(tine_core::query::QUERY_SOURCE_MAX_BYTES + 1);
        assert!(validate_query_source(&source)
            .unwrap_err()
            .starts_with("query-too-large:"));

        let nested = format!("{}(task TODO){}", "(and ".repeat(65), ")".repeat(65));
        assert!(validate_query_source(&nested)
            .unwrap_err()
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

pub(crate) fn decode_asset_b64(b64: &str) -> Result<Vec<u8>, String> {
    use base64::Engine;
    let max_encoded = ASSET_INGRESS_MAX_BYTES.div_ceil(3) * 4;
    if b64.len() > max_encoded
        || decoded_base64_len(b64).is_some_and(|len| len > ASSET_INGRESS_MAX_BYTES)
    {
        return Err("asset payload exceeds 64 MiB ingress limit".into());
    }
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| format!("bad base64 asset payload: {e}"))?;
    if decoded.len() > ASSET_INGRESS_MAX_BYTES {
        return Err("asset payload exceeds 64 MiB ingress limit".into());
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
) -> Result<Option<&SyncRuntimeHandle>, String> {
    if slot.is_sparse_v2() {
        crate::sync_runtime::active_handle(slot).map(Some)
    } else {
        Ok(None)
    }
}

fn map_managed_graph_mutation(
    outcome: Result<
        SyncApplicationUnitOutcome,
        tine_core::sync_runtime::SyncApplicationPageRequestError,
    >,
) -> Result<(), String> {
    match outcome.map_err(|error| error.to_string())? {
        SyncApplicationUnitOutcome::Applied => Ok(()),
        SyncApplicationUnitOutcome::Deferred { .. } => Err(
            "Tine-managed storage is updating pages. Try the operation again when it finishes."
                .into(),
        ),
    }
}

fn map_managed_sync_conflict_resolution(
    outcome: Result<
        SyncApplicationUnitOutcome,
        tine_core::sync_runtime::SyncApplicationPageRequestError,
    >,
) -> Result<(), String> {
    match outcome {
        Err(tine_core::sync_runtime::SyncApplicationPageRequestError::ActorRefusedAt(
            "sync_conflict_changed",
        )) => Err("conflict".into()),
        other => map_managed_graph_mutation(other),
    }
}

fn sparse_page_inventory(handle: &SyncRuntimeHandle) -> Result<Vec<PageEntry>, String> {
    let outcome = handle
        .application_page_inventory()
        .map_err(|error| error.to_string())?;
    map_sparse_page_inventory(outcome)
}

fn sparse_navigation(
    handle: &SyncRuntimeHandle,
    request: SyncApplicationNavigationRequest,
) -> Result<SyncApplicationNavigationReply, String> {
    match handle
        .application_navigation(request)
        .map_err(|error| error.to_string())?
    {
        SyncApplicationNavigationOutcome::Loaded { reply } => Ok(reply),
        SyncApplicationNavigationOutcome::Deferred { state: _ } => Err(
            "Tine-managed storage is updating page navigation. Try again when it finishes.".into(),
        ),
    }
}

fn map_sparse_page_inventory(
    outcome: SyncApplicationPageInventoryOutcome,
) -> Result<Vec<PageEntry>, String> {
    match outcome {
        SyncApplicationPageInventoryOutcome::Loaded { pages } => Ok(pages),
        SyncApplicationPageInventoryOutcome::Deferred { state: _ } => Err(
            "Tine-managed storage is updating the page list. Try again when it finishes.".into(),
        ),
    }
}

fn map_sparse_page_load(
    outcome: SyncApplicationPageLoadOutcome,
) -> Result<Option<PageDto>, String> {
    match outcome {
        SyncApplicationPageLoadOutcome::Loaded {
            mut page,
            revision,
        } => {
            page.rev = Some(revision);
            Ok(Some(page))
        }
        SyncApplicationPageLoadOutcome::Missing { .. } => Ok(None),
        SyncApplicationPageLoadOutcome::Ambiguous => Err(
            "Tine-managed storage could not identify this page. Reload it and resolve any conflicts."
                .into(),
        ),
        SyncApplicationPageLoadOutcome::Deferred { state: _ } => Err(
            "Tine-managed storage is updating this page. Try again when it finishes.".into(),
        ),
    }
}

fn load_sparse_page(
    handle: &SyncRuntimeHandle,
    selector: SyncApplicationPageSelector,
) -> Result<Option<PageDto>, String> {
    let outcome = handle
        .load_application_page(SyncApplicationPageLoadRequest { page: selector })
        .map_err(|error| error.to_string())?;
    map_sparse_page_load(outcome)
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ManagedConflictObservation {
    path: String,
    revision: String,
}

fn sparse_save_request(
    page: PageDto,
    base_rev: Option<String>,
    force: bool,
    managed_conflict_observation: Option<ManagedConflictObservation>,
) -> Result<SyncApplicationPageSaveRequest, String> {
    let target = match (force, base_rev) {
        (true, _) => {
            let observation = managed_conflict_observation.ok_or_else(|| {
                "managed.conflict_unobserved: Keep mine needs an identifiable current managed page. Use current or wait for the page to become identifiable.".to_owned()
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

fn map_sparse_page_save(outcome: SyncApplicationPageSaveOutcome) -> Result<String, String> {
    match outcome {
        SyncApplicationPageSaveOutcome::Saved { revision, .. }
        | SyncApplicationPageSaveOutcome::Unchanged { revision, .. } => Ok(revision),
        // This bounded family tells the frontend to retain the draft, observe
        // the exact current managed page through the actor, and raise the same
        // explicit resolution surface as Direct Files. No revision is embedded
        // in this error: the follow-up exact-path load is the observation, and
        // the actor re-proves its revision in the replacement turn.
        SyncApplicationPageSaveOutcome::Conflict { reason } => Err(format!(
            "managed.conflict: this page changed in Tine-managed storage ({reason:?})"
        )),
        SyncApplicationPageSaveOutcome::Deferred { state: _ } => Err(
            "Tine-managed storage is updating this page. Try saving again when it finishes.".into(),
        ),
    }
}

fn save_sparse_page_with<E>(
    page: PageDto,
    base_rev: Option<String>,
    force: bool,
    managed_conflict_observation: Option<ManagedConflictObservation>,
    save: impl FnOnce(SyncApplicationPageSaveRequest) -> Result<SyncApplicationPageSaveOutcome, E>,
) -> Result<String, String>
where
    E: std::fmt::Display,
{
    let request = sparse_save_request(page, base_rev, force, managed_conflict_observation)?;
    let outcome = save(request).map_err(|error| error.to_string())?;
    map_sparse_page_save(outcome)
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
pub(crate) async fn list_pages(state: GraphContext<'_>) -> Result<Vec<PageEntry>, String> {
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
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn referenced_page_names(
    known_digest: Option<u64>,
    state: GraphContext<'_>,
) -> Result<tine_core::ReferencedPageNames, String> {
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
                _ => Err("managed navigation returned the wrong reply".into()),
            },
            None => Ok(slot
                .legacy_graph()?
                .referenced_page_names_versioned(known_digest)),
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

#[derive(Serialize)]
pub(crate) struct JournalFeedPage {
    pages: Vec<PageDto>,
    next_before_day: Option<i64>,
    done: bool,
    as_of_day: i64,
}

fn collect_journal_feed_page<F>(
    entries: Vec<PageEntry>,
    limit: usize,
    before_day: Option<i64>,
    as_of_day: i64,
    mut load: F,
) -> Result<JournalFeedPage, String>
where
    F: FnMut(&PageEntry) -> Result<PageDto, std::io::Error>,
{
    // A zero limit is authoritative: do not scan/load the feed merely to
    // discover that the caller requested no rows. No cursor advances because
    // no day was examined.
    if limit == 0 {
        let done = !entries
            .iter()
            .any(|entry| before_day.is_none_or(|before| entry.date_key.unwrap_or(0) < before));
        return Ok(JournalFeedPage {
            pages: Vec::new(),
            next_before_day: None,
            done,
            as_of_day,
        });
    }
    let mut out = Vec::new();
    let mut last_examined = None;
    let mut candidates = entries
        .into_iter()
        .filter(|e| before_day.is_none_or(|before| e.date_key.unwrap_or(0) < before))
        .peekable();
    while let Some(e) = candidates.next() {
        let day = e
            .date_key
            .expect("feed inventory only contains dated journals");
        last_examined = Some(day);
        match load(&e) {
            Ok(dto) => out.push(dto),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err.to_string()),
        }
        if out.len() == limit {
            break;
        }
    }
    let done = candidates.peek().is_none();
    Ok(JournalFeedPage {
        pages: out,
        next_before_day: if done { None } else { last_examined },
        done,
        as_of_day,
    })
}

fn canonical_journal_entry(entry: &PageEntry) -> bool {
    let relative = std::path::Path::new(&entry.rel_path);
    let path = if entry.rel_path.is_empty() {
        entry.path.as_path()
    } else {
        relative
    };
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .is_some_and(|stem| JournalDate::from_file_stem(stem).is_some())
}

fn journal_feed_inventory(mut entries: Vec<PageEntry>, as_of_day: i64) -> Vec<PageEntry> {
    entries.retain(|entry| {
        entry.kind == PageKind::Journal && entry.date_key.is_some_and(|day| day <= as_of_day)
    });
    let mut positions = std::collections::HashMap::new();
    let mut deduplicated: Vec<PageEntry> = Vec::new();
    for entry in entries {
        let day = entry
            .date_key
            .expect("feed inventory only contains dated journals");
        if let Some(&position) = positions.get(&day) {
            if canonical_journal_entry(&entry) && !canonical_journal_entry(&deduplicated[position])
            {
                deduplicated[position] = entry;
            }
        } else {
            positions.insert(day, deduplicated.len());
            deduplicated.push(entry);
        }
    }
    deduplicated.sort_by_key(|entry| std::cmp::Reverse(entry.date_key.unwrap_or(0)));
    deduplicated
}

/// Feed-only pagination. `before_day` is an ordinal-day cursor rather than a
/// mutable vector offset, so a file disappearing after inventory cannot make a
/// later day duplicate or disappear from the next request.
#[tauri::command]
pub(crate) async fn journal_feed_page(
    limit: usize,
    before_day: Option<i64>,
    state: GraphContext<'_>,
) -> Result<JournalFeedPage, String> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        let as_of_day = JournalDate::today().ordinal_key();
        match sparse_application_handle(&slot)? {
            Some(handle) => {
                let entries = journal_feed_inventory(sparse_page_inventory(handle)?, as_of_day);
                collect_journal_feed_page(entries, limit, before_day, as_of_day, |entry| {
                    match load_sparse_page(
                        handle,
                        SyncApplicationPageSelector::ExactPath {
                            path: entry.rel_path.clone(),
                        },
                    ) {
                        Ok(Some(page)) => Ok(page),
                        Ok(None) => Err(std::io::Error::from(std::io::ErrorKind::NotFound)),
                        Err(error) => Err(std::io::Error::other(error)),
                    }
                })
            }
            None => {
                let graph = slot.legacy_graph()?;
                let entries =
                    graph.feed_journals_desc_through(JournalDate::from_ordinal(as_of_day));
                collect_journal_feed_page(entries, limit, before_day, as_of_day, |entry| {
                    // A journal deleted from disk between inventory and load is skipped,
                    // but its day still advances the cursor in the helper above.
                    graph.load_page(entry)
                })
            }
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

#[cfg(test)]
mod journal_feed_tests {
    use super::*;
    use std::path::PathBuf;

    fn entry(day: i64) -> PageEntry {
        PageEntry {
            name: day.to_string(),
            kind: PageKind::Journal,
            date_key: Some(day),
            rel_path: String::new(),
            path: PathBuf::new(),
        }
    }
    fn dto(entry: &PageEntry) -> PageDto {
        serde_json::from_value(serde_json::json!({
            "name": entry.name, "kind": "journal", "title": entry.name,
            "pre_block": null, "blocks": []
        }))
        .unwrap()
    }

    #[test]
    fn deletion_stable_day_cursor_fills_then_continues_without_duplicates() {
        let entries = [5, 4, 3, 2, 1].into_iter().map(entry).collect();
        let first = collect_journal_feed_page(entries, 3, None, 5, |e| {
            if e.date_key == Some(5) {
                Err(std::io::Error::from(std::io::ErrorKind::NotFound))
            } else {
                Ok(dto(e))
            }
        })
        .unwrap();
        assert_eq!(
            first
                .pages
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>(),
            ["4", "3", "2"]
        );
        assert_eq!(first.next_before_day, Some(2));
        assert!(!first.done);
        let entries = [5, 4, 3, 2, 1].into_iter().map(entry).collect();
        let second =
            collect_journal_feed_page(entries, 3, first.next_before_day, 5, |e| Ok(dto(e)))
                .unwrap();
        assert_eq!(
            second
                .pages
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>(),
            ["1"]
        );
        assert!(second.done);
        assert_eq!(second.next_before_day, None);
    }

    #[test]
    fn cursor_handles_second_page_loss_empty_suffix_exact_limit_zero_and_hard_errors() {
        let first = collect_journal_feed_page(
            [5, 4, 3, 2, 1].into_iter().map(entry).collect(),
            3,
            None,
            5,
            |e| Ok(dto(e)),
        )
        .unwrap();
        assert_eq!(first.next_before_day, Some(3));
        let second = collect_journal_feed_page(
            [5, 4, 3, 2, 1].into_iter().map(entry).collect(),
            3,
            first.next_before_day,
            5,
            |e| {
                if e.date_key == Some(2) {
                    Err(std::io::Error::from(std::io::ErrorKind::NotFound))
                } else {
                    Ok(dto(e))
                }
            },
        )
        .unwrap();
        assert_eq!(
            second
                .pages
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>(),
            ["1"]
        );
        assert!(
            second.done,
            "a missing second-page row still exhausts the suffix"
        );

        let empty = collect_journal_feed_page(
            [5, 4].into_iter().map(entry).collect(),
            3,
            Some(4),
            5,
            |e| Ok(dto(e)),
        )
        .unwrap();
        assert!(empty.pages.is_empty());
        assert!(empty.done);

        let exact = collect_journal_feed_page(
            [3, 2, 1].into_iter().map(entry).collect(),
            3,
            None,
            3,
            |e| Ok(dto(e)),
        )
        .unwrap();
        assert!(exact.done, "an exactly-full final page is done");
        assert_eq!(exact.next_before_day, None);

        let mut loads = 0;
        let zero = collect_journal_feed_page(
            [3, 2, 1].into_iter().map(entry).collect(),
            0,
            None,
            3,
            |_e| {
                loads += 1;
                Ok(dto(&entry(0)))
            },
        )
        .unwrap();
        assert_eq!(loads, 0, "zero limit loads no entries");
        assert!(!zero.done);

        let hard =
            collect_journal_feed_page([3].into_iter().map(entry).collect(), 1, None, 3, |_e| {
                Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "denied",
                ))
            });
        assert!(matches!(hard, Err(err) if err.contains("denied")));
    }
}

#[tauri::command]
pub(crate) async fn get_page(
    name: String,
    kind: PageKind,
    state: GraphContext<'_>,
) -> Result<Option<PageDto>, String> {
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
                .map_err(|error| error.to_string()),
        }
    })
    .await
    .map_err(|error| error.to_string())?
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
) -> Result<Vec<GraphSourceFile>, String> {
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
fn direct_save_error_message(error: std::io::Error) -> String {
    let code = tine_core::model::direct_save_failure_code(&error);
    // `conflict.*` is the one family the frontend turns into the keep-mine /
    // use-disk banner, so membership is decided in `direct_save_failure_code`
    // by naming the condition, never by a catch-all on an error kind. Matching
    // the prefix keeps that set open to new named conflicts without this arm
    // silently widening to cover unclassified failures.
    if code.starts_with("conflict.") {
        // The observation epoch rides with the banner so "Keep mine" can name
        // the conflict the user actually saw. Without it a second, already-issued
        // force request consumes authority minted for a NEWER unseen winner and
        // overwrites it (GH #254 increment 2, implementation verification,
        // finding 1). A conflict that somehow carries no epoch stays the bare
        // literal, and the frontend then has nothing to present, so its force is
        // refused rather than silently allowed.
        return match tine_core::model::direct_save_conflict_epoch(&error) {
            Some(epoch) => format!("conflict:{epoch}"),
            None => "conflict".to_string(),
        };
    }
    format!("{code}: {error}")
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
) -> Result<String, String> {
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
                    result
                }
                None => {
                    let graph = slot.legacy_graph()?;
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
                            None => Err(std::io::Error::new(
                                std::io::ErrorKind::PermissionDenied,
                                "conflict override authority is missing or already consumed",
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
                    result.map_err(direct_save_error_message)
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
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) fn managed_sync_status(
    state: GraphContext<'_>,
) -> Result<Option<tine_core::crdt::CrdtStatus>, String> {
    with_graph(&state, |g| Ok(g.managed_sync_status()))
}

#[tauri::command]
pub(crate) fn managed_sync_identity_plan(
    state: GraphContext<'_>,
) -> Result<tine_core::model::SyncIdentityPlan, String> {
    with_graph(&state, |g| {
        g.sync_identity_plan().map_err(|error| error.to_string())
    })
}

#[tauri::command]
pub(crate) fn enable_managed_sync(
    app: tauri::AppHandle,
    state: GraphContext<'_>,
) -> Result<tine_core::model::ManagedSyncEnableResult, String> {
    let device_id = crate::settings::managed_sync_device_id(&app)?;
    let result = with_graph(&state, |g| {
        if !g.managed_sync_configured() {
            let (_, complete) = crate::backup::backup_graph_now(&app, g, "pre-sync-enable");
            if !complete {
                return Err(
                    "couldn't create a complete pre-sync safety snapshot; activation aborted"
                        .into(),
                );
            }
        }
        g.enable_managed_sync(device_id, uuid::Uuid::new_v4())
            .map_err(|error| error.to_string())
    })?;
    crate::state::poke_watcher(&state.state);
    Ok(result)
}

#[tauri::command]
pub(crate) fn guide_pages() -> Result<Vec<tine_core::onboarding::GuidePage>, String> {
    tine_core::onboarding::bundled_guide_pages().map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn copy_guide_into_graph(
    title: String,
    state: GraphContext<'_>,
) -> Result<tine_core::onboarding::GuideCopyResult, String> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        match sparse_application_handle(&slot)? {
            Some(handle) => match handle
                .copy_application_guide(title)
                .map_err(|error| error.to_string())?
            {
                SyncApplicationGuideCopyOutcome::Copied { result } => Ok(result),
                SyncApplicationGuideCopyOutcome::Deferred { .. } => Err(
                    "Tine-managed storage is updating pages. Try copying the guide again when it finishes."
                        .into(),
                ),
            },
            None => {
                let graph = slot.legacy_graph()?;
                tine_core::onboarding::copy_guide_into_graph(&graph, &title)
                    .map_err(|error| error.to_string())
            }
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn get_backlinks(
    name: String,
    state: GraphContext<'_>,
) -> Result<Arc<Vec<RefGroup>>, String> {
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
                        Err(format!(
                            "result-too-large: {} matching blocks; narrow the query or add (sample N) (construction limits: {RESULT_BRIDGE_MAX_ROWS} blocks / {RESULT_BRIDGE_MAX_BYTES} bytes)",
                            result.total
                        ))
                    } else {
                        Ok(Arc::new(result.groups))
                    }
                }
                _ => Err("managed backlinks returned the wrong reply kind".into()),
            },
            None => bounded_groups_or_error(slot.legacy_graph()?.backlinks_bounded(
                &name,
                RESULT_BRIDGE_MAX_ROWS,
                RESULT_BRIDGE_MAX_BYTES,
            )),
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn get_backlink_filter_context(
    name: String,
    targets: Vec<BacklinkFilterTarget>,
    state: GraphContext<'_>,
) -> Result<BacklinkFilterContext, String> {
    if targets.len() > RESULT_BRIDGE_MAX_ROWS {
        return Err(format!(
            "too many backlink filter roots: {} (limit: {RESULT_BRIDGE_MAX_ROWS})",
            targets.len()
        ));
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
                _ => Err("managed backlink filter context returned the wrong reply kind".into()),
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
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn get_unlinked_refs(
    name: String,
    state: GraphContext<'_>,
) -> Result<Arc<Vec<RefGroup>>, String> {
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
                        Err(format!(
                            "result-too-large: {} matching blocks; narrow the query or add (sample N) (construction limits: {RESULT_BRIDGE_MAX_ROWS} blocks / {RESULT_BRIDGE_MAX_BYTES} bytes)",
                            result.total
                        ))
                    } else {
                        Ok(Arc::new(result.groups))
                    }
                }
                _ => Err("managed unlinked references returned the wrong reply kind".into()),
            },
            None => bounded_groups_or_error(slot.legacy_graph()?.unlinked_refs_bounded(
                &name,
                RESULT_BRIDGE_MAX_ROWS,
                RESULT_BRIDGE_MAX_BYTES,
            )),
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

/// `block uuid → # of referrers` over the whole graph (drives the per-block
/// reference-count badge). Small map (only referenced uuids); fetched once per
/// graph generation by the frontend.
#[tauri::command]
pub(crate) async fn block_ref_counts(
    state: GraphContext<'_>,
) -> Result<Arc<std::collections::HashMap<String, usize>>, String> {
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
                _ => Err("managed block-reference counts returned the wrong reply kind".into()),
            },
            None => slot
                .legacy_graph()?
                .block_ref_counts()
                .map_err(|error| error.to_string()),
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

/// The blocks that reference block `uuid`, grouped by page (the badge's referrers
/// panel). Lazy: called only when a badge is clicked open.
#[tauri::command]
pub(crate) async fn block_referrers(
    uuid: String,
    state: GraphContext<'_>,
) -> Result<Arc<Vec<RefGroup>>, String> {
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
                        Err(format!(
                            "result-too-large: {} matching blocks; narrow the query or add (sample N) (construction limits: {RESULT_BRIDGE_MAX_ROWS} blocks / {RESULT_BRIDGE_MAX_BYTES} bytes)",
                            result.total
                        ))
                    } else {
                        Ok(Arc::new(result.groups))
                    }
                }
                _ => Err("managed block referrers returned the wrong reply kind".into()),
            },
            None => bounded_groups_or_error(slot.legacy_graph()?.block_referrers_bounded(
                &uuid,
                RESULT_BRIDGE_MAX_ROWS,
                RESULT_BRIDGE_MAX_BYTES,
            )),
        }
    })
    .await
    .map_err(|error| error.to_string())?
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
) -> Result<(), String> {
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
                .map_err(|e| e.to_string()),
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn rename_page(
    old: String,
    new: String,
    expected_path: Option<String>,
    state: GraphContext<'_>,
) -> Result<(), String> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        match sparse_application_handle(&slot)? {
            Some(handle) => map_managed_graph_mutation(handle.mutate_application_graph(
                SyncApplicationGraphMutationRequest::RenamePage {
                    old,
                    new,
                    expected_path,
                },
            )),
            None => slot
                .legacy_graph()?
                .rename_page_expected(&old, &new, expected_path.as_deref())
                .map_err(|e| e.to_string()),
        }
    })
    .await
    .map_err(|error| error.to_string())?
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
pub(crate) async fn publish_html(state: GraphContext<'_>) -> Result<(String, usize), String> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        match sparse_application_handle(&slot)? {
            Some(handle) => match handle
                .publish_application_html()
                .map_err(|error| error.to_string())?
            {
                SyncApplicationPublishOutcome::Published { path, pages } => Ok((path, pages)),
                SyncApplicationPublishOutcome::Deferred { .. } => Err(
                    "Tine-managed storage is updating pages. Try publishing again when it finishes."
                        .into(),
                ),
            },
            None => slot
                .legacy_graph()?
                .publish_html()
                .map_err(|error| error.to_string()),
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Render one page to a self-contained HTML document (assets inlined, no sidebar)
/// for the print-to-PDF export, with the dialog's options. `Err("no-page")` if the
/// page doesn't exist.
#[tauri::command]
pub(crate) async fn page_print_html(
    name: String,
    opts: tine_core::publish::PrintOpts,
    state: GraphContext<'_>,
) -> Result<String, String> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        match sparse_application_handle(&slot)? {
            Some(handle) => {
                let entry = sparse_page_inventory(handle)?
                    .into_iter()
                    .find(|entry| entry.name == name)
                    .ok_or_else(|| "no-page".to_string())?;
                let page = load_sparse_page(
                    handle,
                    SyncApplicationPageSelector::ExactPath {
                        path: entry.rel_path,
                    },
                )?
                .ok_or_else(|| "no-page".to_string())?;
                slot.with_filesystem_graph(|graph| {
                    graph
                        .page_print_html_page(&page, opts)
                        .map_err(|error| error.to_string())
                })
            }
            None => slot
                .legacy_graph()?
                .page_print_html(&name, opts)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "no-page".to_string()),
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn run_query(
    query: String,
    state: GraphContext<'_>,
) -> Result<Arc<Vec<RefGroup>>, String> {
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
                        Err(format!(
                            "result-too-large: {} matching blocks; narrow the query or add (sample N) (construction limits: {RESULT_BRIDGE_MAX_ROWS} blocks / {RESULT_BRIDGE_MAX_BYTES} bytes)",
                            result.total
                        ))
                    } else {
                        Ok(Arc::new(result.groups))
                    }
                }
                _ => Err("managed navigation returned the wrong reply".into()),
            },
            None => bounded_groups_or_error(slot.legacy_graph()?.run_query_bounded(
                &query,
                RESULT_BRIDGE_MAX_ROWS,
                RESULT_BRIDGE_MAX_BYTES,
            )),
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Resolve every query macro in one Copy / Export session under one cumulative
/// construction budget. Unlike `get_page`, this returns only selected subtrees;
/// unrelated page content is never cloned across IPC or retained by the WebView.
#[tauri::command]
pub(crate) async fn export_query_subtrees(
    specs: Vec<tine_core::query::QueryExportSpec>,
    state: GraphContext<'_>,
) -> Result<tine_core::query::QueryExportBatch, String> {
    let query_bytes = specs.iter().fold(0usize, |total, spec| {
        total
            .saturating_add(spec.key.len())
            .saturating_add(spec.query.len())
    });
    if specs.len() > QUERY_EXPORT_REQUEST_MAX_QUERIES || query_bytes > QUERY_EXPORT_MAX_QUERY_BYTES
    {
        return Err(format!(
            "query-export-request-too-large: {} macros / {} bytes (request limits: {} macros / {} bytes; processing cap: {} macros)",
            specs.len(),
            query_bytes,
            QUERY_EXPORT_REQUEST_MAX_QUERIES,
            QUERY_EXPORT_MAX_QUERY_BYTES,
            QUERY_EXPORT_MAX_QUERIES,
        ));
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
                _ => return Err("managed navigation returned the wrong reply".into()),
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
            return Err(format!(
                "query-export-result-too-large: ~{bytes} bytes (limit: {QUERY_EXPORT_MAX_BYTES} bytes)"
            ));
        }
        Ok(batch)
    })
    .await
    .map_err(|error| error.to_string())?
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
) -> Result<tine_core::query_plan::QueryExecution, String> {
    let page_limit = page_limit.min(RESULT_BRIDGE_MAX_ROWS);
    let block_limit = block_limit.min(RESULT_BRIDGE_MAX_ROWS - page_limit);
    let (app, label, binding_generation) = owned_graph_context(state)?;
    let execution = tauri::async_runtime::spawn_blocking(move || -> Result<_, String> {
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
                _ => Err("managed navigation returned the wrong reply".into()),
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
    .map_err(|e| e.to_string())??;
    enforce_query_execution_budget(&execution)?;
    Ok(execution)
}

#[tauri::command]
pub(crate) async fn run_advanced_query(
    query: String,
    current_page: Option<String>,
    state: GraphContext<'_>,
) -> Result<tine_core::query::AdvancedResult, String> {
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
                _ => return Err("managed navigation returned the wrong reply".into()),
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
            Err(format!(
                "result-too-large: {} advanced-query matches; narrow the query",
                bounded.total
            ))
        } else {
            Ok(bounded.result)
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn query_facets(
    state: GraphContext<'_>,
    autocomplete: Option<bool>,
) -> Result<Vec<(String, Vec<String>)>, String> {
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
                            Err(
                                "result-too-large: property facets exceed the construction budget"
                                    .into(),
                            )
                        } else {
                            Ok(facets)
                        }
                    }
                    _ => Err("managed navigation returned the wrong reply".into()),
                }
            }
            None => {
                let graph = slot.legacy_graph()?;
                if autocomplete {
                    return Ok(tine_core::query::autocomplete_property_facets_bounded(
                        &graph,
                        AUTOCOMPLETE_FACET_MAX_ITEMS,
                        AUTOCOMPLETE_FACET_MAX_BYTES,
                    )
                    .0);
                }
                let (facets, exceeded) = tine_core::query::property_facets_bounded(
                    &graph,
                    RESULT_BRIDGE_MAX_ROWS,
                    RESULT_BRIDGE_MAX_BYTES,
                );
                if exceeded {
                    Err("result-too-large: property facets exceed the construction budget".into())
                } else {
                    Ok(facets)
                }
            }
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn page_aliases(state: GraphContext<'_>) -> Result<Vec<(String, String)>, String> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        match sparse_application_handle(&slot)? {
            Some(handle) => {
                match sparse_navigation(handle, SyncApplicationNavigationRequest::PageAliases)? {
                    SyncApplicationNavigationReply::PageAliases(aliases) => Ok(aliases),
                    _ => Err("managed navigation returned the wrong reply".into()),
                }
            }
            None => Ok(slot.legacy_graph()?.page_aliases()),
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn page_icons(
    names: Vec<String>,
    state: GraphContext<'_>,
) -> Result<std::collections::HashMap<String, String>, String> {
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
                _ => Err("managed navigation returned the wrong reply".into()),
            },
            None => Ok(slot.legacy_graph()?.page_icons(&names)),
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn existing_page_names(
    names: Vec<String>,
    state: GraphContext<'_>,
) -> Result<Vec<String>, String> {
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
                _ => Err("managed navigation returned the wrong reply".into()),
            },
            None => Ok(slot.legacy_graph()?.existing_page_names(&names)),
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) fn set_favorites(names: Vec<String>, state: GraphContext<'_>) -> Result<(), String> {
    with_config_graph(&state, |g| {
        g.set_favorites(&names).map_err(|e| e.to_string())
    })
}

#[tauri::command]
pub(crate) fn set_preferred_workflow(
    workflow: String,
    state: GraphContext<'_>,
) -> Result<(), String> {
    with_config_graph(&state, |g| {
        g.set_preferred_workflow(&workflow)
            .map_err(|e| e.to_string())
    })
}

#[tauri::command]
pub(crate) fn set_timetracking_enabled(
    enabled: bool,
    state: GraphContext<'_>,
) -> Result<(), String> {
    with_config_graph(&state, |g| {
        g.set_timetracking_enabled(enabled)
            .map_err(|e| e.to_string())
    })?;
    refresh_graph(&state)?;
    Ok(())
}

#[tauri::command]
pub(crate) fn set_show_brackets(enabled: bool, state: GraphContext<'_>) -> Result<(), String> {
    with_config_graph(&state, |g| {
        g.set_show_brackets(enabled).map_err(|e| e.to_string())
    })?;
    refresh_graph(&state)?;
    Ok(())
}

#[tauri::command]
pub(crate) fn set_doc_mode_enter_for_new_block(
    enabled: bool,
    state: GraphContext<'_>,
) -> Result<(), String> {
    with_config_graph(&state, |g| {
        g.set_doc_mode_enter_for_new_block(enabled)
            .map_err(|e| e.to_string())
    })?;
    refresh_graph(&state)?;
    Ok(())
}

#[tauri::command]
pub(crate) fn set_logical_outdenting(enabled: bool, state: GraphContext<'_>) -> Result<(), String> {
    with_config_graph(&state, |g| {
        g.set_logical_outdenting(enabled).map_err(|e| e.to_string())
    })?;
    refresh_graph(&state)?;
    Ok(())
}

#[tauri::command]
pub(crate) fn set_guide_announced(announced: bool, state: GraphContext<'_>) -> Result<(), String> {
    with_config_graph(&state, |g| {
        g.set_guide_announced(announced).map_err(|e| e.to_string())
    })?;
    refresh_graph(&state)?;
    Ok(())
}

#[tauri::command]
pub(crate) fn set_default_journal_template(
    name: Option<String>,
    state: GraphContext<'_>,
) -> Result<(), String> {
    with_config_graph(&state, |g| {
        g.set_default_journal_template(name.as_deref())
            .map_err(|e| e.to_string())
    })
}

#[tauri::command]
pub(crate) fn set_start_of_week(n: u32, state: GraphContext<'_>) -> Result<(), String> {
    with_config_graph(&state, |g| {
        g.set_start_of_week(n).map_err(|e| e.to_string())
    })
}

/// Set the graph's `:preferred-format` for new pages/journals ("md" or "org").
#[tauri::command]
pub(crate) fn set_preferred_format(format: String, state: GraphContext<'_>) -> Result<(), String> {
    let fmt = if format.eq_ignore_ascii_case("org") {
        tine_core::model::Format::Org
    } else {
        tine_core::model::Format::Md
    };
    with_config_graph(&state, |g| {
        g.set_preferred_format(fmt).map_err(|e| e.to_string())
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
) -> Result<(), String> {
    with_config_graph(&state, |g| {
        g.set_journal_page_title_format(&format)
            .map_err(|e| e.to_string())
    })?;
    refresh_graph(&state)?; // pick up the new format + migrate any title-named journals
    Ok(())
}

#[tauri::command]
pub(crate) fn read_custom_css(state: GraphContext<'_>) -> Result<String, String> {
    with_filesystem_graph(&state, |g| Ok(g.custom_css()))
}

#[tauri::command]
pub(crate) async fn search(
    query: String,
    limit: usize,
    lane: Option<String>,
    state: GraphContext<'_>,
) -> Result<Vec<RefGroup>, String> {
    let limit = limit.min(RESULT_BRIDGE_MAX_ROWS);
    let (app, label, binding_generation) = owned_graph_context(state)?;
    let groups = tauri::async_runtime::spawn_blocking(move || -> Result<_, String> {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        match sparse_application_handle(&slot)? {
            Some(handle) => match sparse_navigation(
                handle,
                SyncApplicationNavigationRequest::BlockSearch { query, limit, lane },
            )? {
                SyncApplicationNavigationReply::BlockSearch(groups) => Ok(groups),
                _ => Err("managed navigation returned the wrong reply".into()),
            },
            None => Ok(match lane.as_deref() {
                Some(lane) => slot.legacy_graph()?.search_latest(lane, &query, limit),
                None => slot.legacy_graph()?.search(&query, limit),
            }),
        }
    })
    .await
    .map_err(|e| e.to_string())??;
    enforce_result_bridge_budget(&groups)?;
    Ok(groups)
}

#[tauri::command]
pub(crate) async fn quick_switch(
    query: String,
    limit: usize,
    state: GraphContext<'_>,
) -> Result<Vec<PageEntry>, String> {
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
                _ => Err("managed navigation returned the wrong reply".into()),
            },
            None => Ok(slot.legacy_graph()?.quick_switch(&query, limit)),
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

fn capture_quick_switch_for(
    state: &AppState,
    caller: &str,
    binding_generation: Option<u64>,
    query: &str,
    limit: usize,
) -> Result<Vec<PageEntry>, String> {
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
) -> Result<Vec<PageEntry>, String> {
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
            graph_load: Mutex::new(()),
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
                .unwrap_err(),
            "stale-graph-binding"
        );
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn rejects_non_capture_callers() {
        let (state, base) = state_with_selected_graph();
        let generation = state.capture_graph_binding().unwrap().binding_generation;
        assert_eq!(
            capture_quick_switch_for(&state, "main", Some(generation), "Selected", 8).unwrap_err(),
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
                .unwrap(),
            "no graph loaded for window capture"
        );
        std::fs::remove_dir_all(base).unwrap();
    }
}

#[tauri::command]
pub(crate) async fn list_templates(
    state: GraphContext<'_>,
) -> Result<Vec<tine_core::model::TemplateDto>, String> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        match sparse_application_handle(&slot)? {
            Some(handle) => {
                match sparse_navigation(handle, SyncApplicationNavigationRequest::ListTemplates)? {
                    SyncApplicationNavigationReply::Templates(templates) => Ok(templates),
                    _ => Err("managed navigation returned the wrong reply".into()),
                }
            }
            None => Ok(slot.legacy_graph()?.templates()),
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

fn application_property_line(line: &str) -> bool {
    let Some(separator) = line.find("::") else {
        return false;
    };
    let key = line[..separator].trim();
    !key.is_empty()
        && key.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.' | '/')
        })
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

fn sparse_journal_content_days(handle: &SyncRuntimeHandle) -> Result<Vec<i64>, String> {
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
pub(crate) async fn journal_content_days(state: GraphContext<'_>) -> Result<Vec<i64>, String> {
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
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn resolve_block(
    uuid: String,
    state: GraphContext<'_>,
) -> Result<Option<RefGroup>, String> {
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
                _ => return Err("managed block resolution returned the wrong reply kind".into()),
            },
            None => slot.legacy_graph()?.resolve_block(&uuid),
        };
        if let Some(group) = &group {
            enforce_result_bridge_budget(std::slice::from_ref(group))?;
        }
        Ok(group)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn resolve_blocks(
    uuids: Vec<String>,
    state: GraphContext<'_>,
) -> Result<Vec<Option<RefGroup>>, String> {
    if uuids.len() > RESULT_BRIDGE_MAX_ROWS {
        return Err(format!(
            "result-too-large: {} requested block references (limit: {RESULT_BRIDGE_MAX_ROWS})",
            uuids.len()
        ));
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
                _ => Err("managed block resolution returned the wrong reply kind".into()),
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
                    Err(format!("result-too-large: {total} resolved block-reference rows exceed the construction budget"))
                } else {
                    Ok(groups)
                }
            }
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Explicit, bounded subtree resolution for hover previews. Ordinary
/// `resolve_block(s)` stays shallow so a page containing nested references
/// cannot multiply the same descendants across the IPC bridge.
#[tauri::command]
pub(crate) async fn preview_block(
    uuid: String,
    max_nodes: usize,
    state: GraphContext<'_>,
) -> Result<Option<tine_core::BlockPreview>, String> {
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
                _ => return Err("managed block preview returned the wrong reply kind".into()),
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
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) fn read_asset(
    name: String,
    max_bytes: Option<u64>,
    state: GraphContext<'_>,
) -> Result<tauri::ipc::Response, String> {
    // Return RAW bytes (not a JSON number[]), so a multi-MB PDF/image isn't
    // serialized element-by-element and re-parsed on the JS side — the frontend
    // receives an ArrayBuffer directly.
    with_filesystem_graph(&state, |g| {
        max_bytes
            .map_or_else(
                || g.read_asset(&name),
                |limit| g.read_asset_limited(&name, limit),
            )
            .map(tauri::ipc::Response::new)
            .map_err(|e| e.to_string())
    })
}

/// Validate one graph media file and return its top-level asset name for the
/// range-aware `tine-media:` protocol. The protocol revalidates against the
/// requesting window's current graph on every request.
#[tauri::command]
pub(crate) fn stream_asset_path(name: String, state: GraphContext<'_>) -> Result<String, String> {
    let slot = slot_for_context(&state)?;
    slot.with_filesystem_graph(|graph| graph.stream_asset_path(&name).map_err(|e| e.to_string()))?;
    Ok(format!("{}/{}", slot.binding_generation, name))
}

/// Quit the app cleanly. On Linux, first SIGKILL WebKitGTK's helper subprocesses so
/// they don't run their buggy GL-driver atexit teardown and dump a SIGABRT core on
/// exit (GH #28). The JS close handler calls this only AFTER `flushAll()`/
/// `flushSession()` have resolved, so tearing the web process down hard loses no
/// edits. Then hand off to Tauri's normal exit (the main process still tears down
/// the way it always has — no dump there). On non-Linux this is just `app.exit(0)`.
#[tauri::command]
pub(crate) fn tine_quit(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::state::AppState>,
) -> Result<(), String> {
    for (_, slot) in state.graphs.read().unwrap().entries() {
        crate::sync_runtime::clean_shutdown_slot(&slot)
            .map_err(|error| format!("sparse-v2-shutdown-refused: {error}"))?;
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
) -> Result<(), String> {
    let slot = crate::state::slot_for_window(&state, window.label())?;
    crate::sync_runtime::clean_shutdown_slot(&slot)
        .map_err(|error| format!("sparse-v2-shutdown-refused: {error}"))?;
    if state.graphs.read().unwrap().len() <= 1 {
        #[cfg(target_os = "linux")]
        crate::platform::kill_webkit_children();
        app.exit(0);
        return Ok(());
    }
    window.destroy().map_err(|e| e.to_string())
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
) -> Result<tauri::ipc::Response, String> {
    // Read an image from an ABSOLUTE path OUTSIDE the graph, for raw-HTML `<img>`
    // srcs the user has explicitly opted into (Settings → "Load local-file images").
    // OFF by default; gated here too (defense in depth — the frontend also checks),
    // restricted to image extensions + a size cap so an allowed note can't slurp an
    // arbitrary file. Returns RAW bytes like `read_asset`. See ADR 0019.
    if !crate::settings::get_app_bool("allow_local_file_images".into(), false, app) {
        return Err("local-file images are disabled".into());
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
        return Err("not an image file".into());
    }
    let meta = std::fs::metadata(p).map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err("not a file".into());
    }
    const MAX_BYTES: u64 = 64 * 1024 * 1024;
    if meta.len() > MAX_BYTES {
        return Err("image too large".into());
    }
    std::fs::read(p)
        .map(tauri::ipc::Response::new)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) fn import_asset(
    path: String,
    name: Option<String>,
    state: GraphContext<'_>,
) -> Result<String, String> {
    with_filesystem_graph(&state, |g| {
        g.import_asset(std::path::Path::new(&path), name.as_deref())
            .map_err(|e| e.to_string())
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
) -> Result<String, String> {
    use cap_std::{ambient_authority, fs::Dir};
    use tauri::Manager;

    const MAX_PHOTO_BYTES: u64 = 64 * 1024 * 1024;
    const MAX_RECORDING_BYTES: u64 = 32 * 1024 * 1024;
    let source = std::path::Path::new(&path);
    let filename = source
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "invalid native capture token".to_string())?;
    let (max_bytes, media_label) =
        if filename.starts_with("tine_memo_") && filename.ends_with(".m4a") {
            (MAX_RECORDING_BYTES, "recording")
        } else if filename.starts_with("tine_photo_") && filename.ends_with(".jpg") {
            (MAX_PHOTO_BYTES, "photo")
        } else {
            return Err("invalid native capture token".into());
        };
    let cache_path = app
        .path()
        .app_cache_dir()
        .map_err(|error| error.to_string())?;
    let token_parent = source
        .parent()
        .ok_or_else(|| "recording has no cache parent".to_string())?;
    let cache_dir = Dir::open_ambient_dir(&cache_path, ambient_authority())
        .map_err(|error| error.to_string())?;
    let token_dir = Dir::open_ambient_dir(token_parent, ambient_authority())
        .map_err(|error| error.to_string())?;
    let cache_identity = same_file::Handle::from_file(
        cache_dir
            .try_clone()
            .map_err(|error| error.to_string())?
            .into_std_file(),
    )
    .map_err(|error| error.to_string())?;
    let token_identity = same_file::Handle::from_file(
        token_dir
            .try_clone()
            .map_err(|error| error.to_string())?
            .into_std_file(),
    )
    .map_err(|error| error.to_string())?;
    if token_identity != cache_identity {
        return Err("capture is outside Tine's native cache".into());
    }

    let capture = token_dir
        .open(filename)
        .map_err(|error| error.to_string())?;
    let metadata = capture.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > max_bytes {
        return Err(format!(
            "{media_label} is empty or exceeds the {} MiB limit",
            max_bytes / (1024 * 1024)
        ));
    }
    let mut capture = capture.into_std();
    let stored = with_filesystem_graph(&state, |graph| {
        graph
            .import_asset_file(&mut capture, &name, max_bytes)
            .map_err(|error| error.to_string())
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
pub(crate) fn read_text_file(path: String) -> Result<String, String> {
    fn delimited_ext(p: &std::path::Path) -> bool {
        p.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("csv") || e.eq_ignore_ascii_case("tsv"))
            .unwrap_or(false)
    }
    let p = std::path::Path::new(&path);
    if !delimited_ext(p) {
        return Err("unsupported file type".into());
    }
    // Re-check on the RESOLVED path too — a symlink named x.csv pointing at an
    // arbitrary file must not pass the extension gate (review finding).
    let resolved = std::fs::canonicalize(p).map_err(|e| e.to_string())?;
    if !delimited_ext(&resolved) {
        return Err("unsupported file type".into());
    }
    let meta = std::fs::metadata(&resolved).map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err("not a file".into());
    }
    const MAX_BYTES: u64 = 10 * 1024 * 1024;
    if meta.len() > MAX_BYTES {
        return Err("text file too large".into());
    }
    std::fs::read_to_string(&resolved).map_err(|e| e.to_string())
}

/// Open a graph asset (by its `assets/`-relative name) in the OS default app,
/// e.g. a video/audio file in the system player. Path-gated to the assets dir
/// (canonicalized) so a crafted name can't open a file outside the graph.
#[tauri::command]
pub(crate) fn open_asset(name: String, state: GraphContext<'_>) -> Result<(), String> {
    let target = with_filesystem_graph(&state, |g| {
        g.asset_file_for_read(&name).map_err(|e| e.to_string())
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
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    // Mobile: opening an asset in an external app uses a platform intent; stub for now (M1).
    #[cfg(not(desktop))]
    {
        let _ = (&name, &target);
        Err("open asset externally is not supported on this platform".into())
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
) -> Result<(), String> {
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
                .ok_or_else(|| "no-page".to_string())?;
                slot.with_filesystem_graph(|graph| {
                    graph
                        .page_source_file(&page.name, page.kind, Some(&page.path))
                        .map_err(|error| error.to_string())
                })
            }
            None => slot
                .legacy_graph()?
                .page_source_file(&name, kind, path.as_deref())
                .map_err(|error| error.to_string()),
        }
    })
    .await
    .map_err(|error| error.to_string())??;
    #[cfg(desktop)]
    {
        if reveal {
            reveal_page_source(&target)
        } else {
            open_page_source(&target)
        }
    }
    #[cfg(not(desktop))]
    {
        let _ = (target, reveal);
        Err("page file actions are available on desktop only".into())
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
) -> Result<(), String> {
    let target = with_filesystem_graph(&state, |g| {
        g.asset_file_for_read(&name).map_err(|e| e.to_string())
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
                .map_err(|e| e.to_string())?;
            return Ok(());
        }
        let (prog, args) = build_editor_argv(trimmed, &target_str)?;
        diag(format!("edit_asset_external: {name} -> {prog} {args:?}"));
        opener_command(&prog)
            .args(&args)
            .spawn()
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    #[cfg(not(desktop))]
    {
        let _ = (&name, &command, &target);
        Err("editing an asset externally is not supported on this platform".into())
    }
}

/// Best-effort autodetect of an installed external editor's launch command, by
/// PROBING known install locations on disk — never executing anything (so a
/// Flatpak wrapper can't leak its bundled env into the probe). Returns a command
/// template suitable for `edit_asset_external`, or an empty string if not found
/// (the caller then leaves the setting empty = OS opener). Currently knows
/// `drawio`; other ids return empty.
#[tauri::command]
pub(crate) fn detect_media_editor(id: String) -> Result<String, String> {
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
fn build_editor_argv(command: &str, target: &str) -> Result<(String, Vec<String>), String> {
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
        return Err("unclosed double quote in editor command".to_string());
    }
    if token_started {
        tokens.push(token);
    }

    let (prog, rest) = tokens
        .split_first()
        .ok_or_else(|| "empty editor command".to_string())?;
    if prog.is_empty() {
        return Err("editor command program is empty".to_string());
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
            build_editor_argv("   ", "/g/x.svg").unwrap_err(),
            "empty editor command"
        );
        assert_eq!(
            build_editor_argv(r#""C:\Program Files\draw.io\draw.io.exe {}"#, "/g/x.svg")
                .unwrap_err(),
            "unclosed double quote in editor command"
        );
        assert_eq!(
            build_editor_argv(r#""" {}"#, "/g/x.svg").unwrap_err(),
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
) -> Result<Vec<tine_core::model::AssetInfo>, String> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        match sparse_application_handle(&slot)? {
            Some(handle) => {
                match sparse_navigation(handle, SyncApplicationNavigationRequest::OrphanAssets)? {
                    SyncApplicationNavigationReply::OrphanAssets(assets) => Ok(assets),
                    _ => Err("managed navigation returned the wrong reply".into()),
                }
            }
            None => Ok(slot.legacy_graph()?.orphan_assets()),
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Move an orphaned asset to the recoverable trash.
#[tauri::command]
pub(crate) fn trash_asset(name: String, state: GraphContext<'_>) -> Result<(), String> {
    with_trash_graph(&state, |g| g.trash_asset(&name).map_err(|e| e.to_string()))
}

/// Count + total bytes in the recoverable asset trash.
#[tauri::command]
pub(crate) fn asset_trash_stats(
    state: GraphContext<'_>,
) -> Result<tine_core::model::TrashStats, String> {
    with_filesystem_graph(&state, |g| Ok(g.asset_trash_stats()))
}

/// Permanently delete everything in the asset trash; returns files removed.
#[tauri::command]
pub(crate) fn empty_asset_trash(state: GraphContext<'_>) -> Result<u64, String> {
    with_trash_graph(&state, |g| g.empty_asset_trash().map_err(|e| e.to_string()))
}

/// Journal days that resolve to more than one file (e.g. a date-stem file plus a
/// title-named one) — for the user to reconcile.
#[tauri::command]
pub(crate) fn list_journal_conflicts(
    state: GraphContext<'_>,
) -> Result<Vec<tine_core::model::JournalConflict>, String> {
    with_filesystem_graph(&state, |g| Ok(g.journal_conflicts()))
}

/// Sync-tool conflict copies (Syncthing/Dropbox) sitting in the graph — for the
/// user to review + reconcile instead of them showing as garbage pages.
#[tauri::command]
pub(crate) fn list_sync_conflicts(
    state: GraphContext<'_>,
) -> Result<Vec<tine_core::model::SyncConflict>, String> {
    with_filesystem_graph(&state, |g| Ok(g.list_sync_conflicts()))
}

/// Block-level diff of a sync-conflict copy against its winner (both graph-root-
/// relative paths) — the data behind the two-column merge UI. Read-only.
#[tauri::command]
pub(crate) fn sync_conflict_diff(
    winner: String,
    conflict: String,
    state: GraphContext<'_>,
) -> Result<Option<tine_core::sync_diff::SyncConflictDiff>, String> {
    with_filesystem_graph(&state, |g| {
        g.sync_conflict_diff(&winner, &conflict)
            .map_err(|e| e.to_string())
    })
}

/// Resolve a sync-conflict copy: merge it into its winner per the user's per-row
/// `decisions` (row id → "mine"/"theirs"/"both") via the normal save path, then
/// trash the conflict copy. `base_rev` guards against the winner changing under
/// the merge; returns "conflict" if it did. `pre_choice`: "mine"/"theirs"/"union".
#[tauri::command]
pub(crate) async fn resolve_sync_conflict(
    winner: String,
    conflict: String,
    decisions: std::collections::HashMap<String, String>,
    base_rev: String,
    conflict_rev: String,
    pre_choice: Option<String>,
    state: GraphContext<'_>,
) -> Result<(), String> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        match sparse_application_handle(&slot)? {
            Some(handle) => map_managed_sync_conflict_resolution(handle.mutate_application_graph(
                SyncApplicationGraphMutationRequest::ResolveSyncConflict {
                    winner_path: winner,
                    conflict_path: conflict,
                    decisions,
                    base_revision: base_rev,
                    conflict_revision: conflict_rev,
                    pre_choice: pre_choice.unwrap_or_else(|| "union".into()),
                },
            )),
            None => slot
                .legacy_graph()?
                .resolve_sync_conflict(
                    &winner,
                    &conflict,
                    &decisions,
                    &base_rev,
                    &conflict_rev,
                    pre_choice.as_deref().unwrap_or("union"),
                )
                .map_err(|error| {
                    if error.kind() == std::io::ErrorKind::AlreadyExists {
                        "conflict".to_string()
                    } else {
                        error.to_string()
                    }
                }),
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Discard a sync-conflict copy without merging (move it to the recoverable
/// trash). Refuses anything that isn't a conflict copy.
#[tauri::command]
pub(crate) fn trash_sync_conflict(conflict: String, state: GraphContext<'_>) -> Result<(), String> {
    with_trash_graph(&state, |g| {
        g.trash_sync_conflict(&conflict).map_err(|e| e.to_string())
    })
}

/// Move one journal file (by exact filename) to the recoverable trash.
#[tauri::command]
pub(crate) async fn trash_journal_file(
    name: String,
    state: GraphContext<'_>,
) -> Result<(), String> {
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
                .map_err(|error| error.to_string()),
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Raw contents of one journal file (by exact filename) — for inspecting a
/// duplicate day's files before reconciling.
#[tauri::command]
pub(crate) fn read_journal_file(name: String, state: GraphContext<'_>) -> Result<String, String> {
    with_filesystem_graph(&state, |g| {
        g.read_journal_file(&name).map_err(|e| e.to_string())
    })
}

/// Load a page from a SPECIFIC file by its graph-root-relative path — lets the UI
/// navigate to a duplicate-day stray that shares a (kind,name) with the canonical
/// file and so is unreachable by name (#21).
#[tauri::command]
pub(crate) async fn get_page_by_path(
    path: String,
    state: GraphContext<'_>,
) -> Result<Option<PageDto>, String> {
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
                .map_err(|error| error.to_string()),
        }
    })
    .await
    .map_err(|error| error.to_string())?
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
        assert!(ambiguous.contains("could not identify this page"));
        assert!(ambiguous.contains("Reload it"));
        let deferred = map_sparse_page_load(SyncApplicationPageLoadOutcome::Deferred {
            state: SyncEditorDeferred::RetryableExternalWork,
        })
        .unwrap_err();
        assert!(deferred.contains("updating this page"));
        assert!(deferred.contains("Try again"));
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
            |request| -> Result<SyncApplicationPageSaveOutcome, &'static str> {
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
            |request| -> Result<SyncApplicationPageSaveOutcome, &'static str> {
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
            |_request| -> Result<SyncApplicationPageSaveOutcome, &'static str> {
                unreachable!("an unobserved replacement must fail before actor invocation")
            },
        )
        .unwrap_err();
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
        let feed = collect_journal_feed_page(inventory, 1, None, 20260729, |entry| {
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
    state: GraphContext<'_>,
) -> Result<(), String> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        match sparse_application_handle(&slot)? {
            Some(handle) => map_managed_graph_mutation(handle.mutate_application_graph(
                SyncApplicationGraphMutationRequest::MergePages {
                    source_path: src,
                    destination_path: dst,
                },
            )),
            None => slot
                .legacy_graph()?
                .merge_pages(&src, &dst)
                .map_err(|e| e.to_string()),
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Rescue a duplicate-day stray by moving it to a uniquely-named page
/// (`pages/<new_name>`), so it stops colliding and becomes normally navigable (#21).
#[tauri::command]
pub(crate) async fn rename_file_to_page(
    path: String,
    new_name: String,
    state: GraphContext<'_>,
) -> Result<(), String> {
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
                .map_err(|e| e.to_string()),
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) fn save_asset(
    name: String,
    bytes_b64: String,
    state: GraphContext<'_>,
) -> Result<String, String> {
    let bytes = decode_asset_b64(&bytes_b64)?;
    with_filesystem_graph(&state, |g| {
        g.save_asset(&name, &bytes).map_err(|e| e.to_string())
    })
}

#[tauri::command]
pub(crate) fn read_highlights(
    pdf: String,
    state: GraphContext<'_>,
) -> Result<Vec<tine_core::pdf::Highlight>, String> {
    with_filesystem_graph(&state, |g| Ok(g.read_highlights(&pdf)))
}

#[tauri::command]
pub(crate) async fn open_pdf(
    pdf: String,
    label: String,
    state: GraphContext<'_>,
) -> Result<tine_core::pdf::PdfState, String> {
    let (app, window_label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &window_label, Some(binding_generation))?;
        match sparse_application_handle(&slot)? {
            Some(handle) => match handle
                .open_application_pdf(pdf, label)
                .map_err(|error| error.to_string())?
            {
                SyncApplicationPdfOpenOutcome::Ready { state } => Ok(state),
                SyncApplicationPdfOpenOutcome::Deferred { .. } => Err(
                    "Tine-managed storage is updating PDF notes. Try again when it finishes."
                        .into(),
                ),
            },
            None => slot
                .legacy_graph()?
                .open_pdf(&pdf, &label)
                .map_err(|error| error.to_string()),
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn write_highlights(
    pdf: String,
    label: String,
    highlights: Vec<tine_core::pdf::Highlight>,
    base_ids: Vec<String>,
    state: GraphContext<'_>,
) -> Result<(), String> {
    let (app, window_label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &window_label, Some(binding_generation))?;
        match sparse_application_handle(&slot)? {
            Some(handle) => match handle
                .write_application_pdf_highlights(pdf, label, highlights, base_ids)
                .map_err(|error| error.to_string())?
            {
                SyncApplicationUnitOutcome::Applied => Ok(()),
                SyncApplicationUnitOutcome::Deferred { .. } => Err(
                    "Tine-managed storage is updating PDF notes. Try again when it finishes."
                        .into(),
                ),
            },
            None => slot
                .legacy_graph()?
                .write_highlights(&pdf, &label, &highlights, &base_ids)
                .map_err(|error| error.to_string()),
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn write_pdf_view_state(
    pdf: String,
    page: i64,
    scale: f64,
    state: GraphContext<'_>,
) -> Result<(), String> {
    let (app, label, binding_generation) = owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = slot_for_bound_window(&state, &label, Some(binding_generation))?;
        match sparse_application_handle(&slot)? {
            Some(handle) => match handle
                .write_application_pdf_view_state(pdf, page, scale)
                .map_err(|error| error.to_string())?
            {
                SyncApplicationUnitOutcome::Applied => Ok(()),
                SyncApplicationUnitOutcome::Deferred { .. } => Err(
                    "Tine-managed storage is updating PDF state. Try again when it finishes."
                        .into(),
                ),
            },
            None => slot
                .legacy_graph()?
                .write_pdf_view_state(&pdf, page, scale)
                .map_err(|error| error.to_string()),
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) fn save_pdf_area_image(
    pdf: String,
    page: i64,
    id: String,
    stamp: i64,
    bytes_b64: String,
    state: GraphContext<'_>,
) -> Result<String, String> {
    let bytes = decode_asset_b64(&bytes_b64)?;
    with_filesystem_graph(&state, |g| {
        g.write_pdf_area_image(&pdf, page, &id, stamp, &bytes)
            .map_err(|e| e.to_string())
    })
}

#[cfg(test)]
mod direct_save_error_tests {
    use super::direct_save_error_message;
    use std::io;

    /// The frontend puts up a conflict prompt ("Keep mine" / "Use disk version")
    /// for exactly one message, and a page it marks conflicted stops saving until
    /// the user resolves it. So the set of failures that produce that message is
    /// a contract, not a formatting detail: anything in it that the two buttons
    /// cannot resolve strands the page.
    #[test]
    fn only_a_real_base_revision_conflict_raises_the_conflict_prompt() {
        assert_eq!(
            direct_save_error_message(io::Error::new(io::ErrorKind::AlreadyExists, "conflict")),
            "conflict"
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
            let reported =
                direct_save_error_message(io::Error::new(io::ErrorKind::AlreadyExists, message));
            assert!(
                reported.starts_with(expected_code),
                "{message} should report as {expected_code}, got {reported}"
            );
            assert_ne!(
                reported, "conflict",
                "{message} cannot be resolved by keep-mine or use-disk, so it must not \
                 raise the conflict prompt"
            );
        }
    }

    /// The counterpart: a page whose file moved between load and save IS a
    /// content conflict, and since `011658a9` "keep mine" can actually resolve
    /// it. It must reach the prompt.
    #[test]
    fn an_unobserved_external_change_still_raises_the_conflict_prompt() {
        assert_eq!(
            direct_save_error_message(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "path-pinned page does not match its captured exact owner",
            )),
            "conflict"
        );
    }

    #[test]
    fn every_minted_site_and_no_tokenless_site_reaches_the_banner() {
        for message in [
            "editor conflict: save baseline present",
            "editor conflict: save baseline absent",
            "editor conflict: commit recheck",
            "editor conflict: replace pre-retirement",
            "editor conflict: retired mismatch",
            "editor conflict: publication collision",
            "editor conflict: create publication collision",
            "editor conflict: final reread absent",
            "editor conflict: final reread present",
            "editor conflict: post-publication validation",
        ] {
            assert_eq!(
                direct_save_error_message(io::Error::new(io::ErrorKind::AlreadyExists, message,)),
                "conflict",
                "minted authority at {message} must reach the two-arm banner"
            );
        }
        for message in [
            "tokenless editor conflict: commit recheck: continued churn",
            "tokenless editor conflict: replace pre-retirement: transient I/O",
            "tokenless editor conflict: final reread present: transient I/O",
        ] {
            let reported =
                direct_save_error_message(io::Error::new(io::ErrorKind::WouldBlock, message));
            assert!(reported.starts_with("conflict_retry."), "{reported}");
            assert_ne!(reported, "conflict");
        }
    }
}
