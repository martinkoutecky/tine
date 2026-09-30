#[cfg(desktop)]
use crate::debug::diag_private;
#[cfg(desktop)]
use crate::platform::{open_page_source, opener_command, reveal_page_source};
use crate::state::{
    capture_quick_switch_slot, slot_for_context, AppState, GraphContext, GraphSlot,
};
use serde::Serialize;
use std::sync::Arc;
use tauri::{State, WebviewWindow};
use tine_core::model::{
    AssetInfo, BacklinkFilterContext, BacklinkFilterTarget, PageDto, PageEntry, PageKind, RefGroup,
};
use tine_graph_features::journals::{self, JournalFilenameMigration};
use tine_graph_features::{config, IncompleteTransaction as IncompleteTx};
use tine_store::{FacetPolicy, PageId, Resolved, StoreError, WholeGraph};
#[cfg(test)]
use tine_store::{SaveOutcome, SavePagesOutcome};
mod discovery;
mod save_wire;
use discovery::{page_inventory_wire, resolve_name, PageInventoryWire};
use save_wire::SavePagesWire;
#[cfg(test)]
use save_wire::{save_outcome_to_wire, save_pages_outcome_to_wire};

fn feature_asset_error(error: std::io::Error, slot: &GraphSlot) -> String {
    tine_graph_features::assets::error_for_user(&slot.store, error)
}
fn feature_pdf_error(error: std::io::Error) -> String {
    if error.kind() == std::io::ErrorKind::WouldBlock {
        "conflict".to_string()
    } else {
        error.to_string()
    }
}

#[derive(Serialize)]
pub(crate) struct PageWire {
    id: String,
    #[serde(flatten)]
    doc: PageDto,
}

impl std::ops::Deref for PageWire {
    type Target = PageDto;

    fn deref(&self) -> &PageDto {
        &self.doc
    }
}

fn page_dto(read: tine_store::PageRead) -> PageWire {
    let mut doc = read.doc;
    doc.rev = Some(read.rev.into());
    doc.read_only = read.read_only.is_some();
    PageWire {
        id: read.id.into(),
        doc,
    }
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum ResolvedWire {
    Existing { id: String, others: Vec<String> },
    Alias { owners: Vec<String> },
    Absent { id: String },
}

impl From<Resolved> for ResolvedWire {
    fn from(value: Resolved) -> Self {
        Self::from(&value)
    }
}

impl From<&Resolved> for ResolvedWire {
    fn from(value: &Resolved) -> Self {
        let id = |id: &PageId| id.as_str().to_owned();
        match value {
            Resolved::Existing { id: first, others } => Self::Existing {
                id: id(first),
                others: others.iter().map(id).collect(),
            },
            Resolved::Alias { owners } => Self::Alias {
                owners: owners.iter().map(id).collect(),
            },
            Resolved::Absent { id: absent } => Self::Absent { id: id(absent) },
        }
    }
}

fn store_error(error: StoreError) -> String {
    match error {
        StoreError::NotFound => std::io::ErrorKind::NotFound.to_string(),
        StoreError::InvalidTarget(_) | StoreError::PageSource(_) | StoreError::StreamSymlink(_) => {
            "invalid page path".into()
        }
        StoreError::Undecodable => "stream did not contain valid UTF-8".into(),
        StoreError::Unparseable(reason) => reason,
        StoreError::TooLarge { .. } => "asset-too-large".into(),
        StoreError::Io(error) => error.to_string(),
        StoreError::Closed => "store closed".into(),
    }
}

fn asset_error(error: StoreError) -> String {
    match error {
        StoreError::NotFound => std::io::Error::from_raw_os_error(2).to_string(),
        StoreError::InvalidTarget(_) | StoreError::PageSource(_) | StoreError::StreamSymlink(_) => {
            "invalid asset".into()
        }
        StoreError::TooLarge { .. } => "asset-too-large".into(),
        other => store_error(other),
    }
}

pub(crate) fn sync_conflict_error(error: std::io::Error) -> String {
    if error.get_ref().is_some_and(|e| e.is::<IncompleteTx>()) {
        return error.to_string();
    }
    if error.kind() == std::io::ErrorKind::AlreadyExists {
        "conflict".into()
    } else {
        format!("io:{:?}", error.kind())
    }
}

fn feature_asset_access_error(error: tine_graph_features::assets::AssetAccessError) -> String {
    match error {
        tine_graph_features::assets::AssetAccessError::BadName => "bad asset name".into(),
        tine_graph_features::assets::AssetAccessError::StreamSymlink => {
            "asset symlinks cannot be streamed".into()
        }
        tine_graph_features::assets::AssetAccessError::Store(error) => asset_error(error),
    }
}

fn asset_handoff_target(slot: &GraphSlot, name: &str) -> Result<std::path::PathBuf, String> {
    tine_graph_features::assets::path_for_os_handoff(&slot.store, name)
        .map_err(feature_asset_access_error)
}

fn feature_page_read_error(error: tine_graph_features::pages::PageReadError) -> String {
    match error {
        tine_graph_features::pages::PageReadError::Load(error) => format!("{error:?}"),
        tine_graph_features::pages::PageReadError::Source(reason) => reason,
        tine_graph_features::pages::PageReadError::Store(error) => store_error(error),
        tine_graph_features::pages::PageReadError::EmptyAlias => "alias has no owner".into(),
    }
}

#[cfg(test)]
mod device_read_tests {
    use super::*;

    #[test]
    fn read_text_file_refuses_bound_graph_csv() {
        let temp = tempfile::tempdir().unwrap();
        let graph = temp.path().join("graph");
        std::fs::create_dir_all(graph.join("pages")).unwrap();
        let csv = graph.join("private.csv");
        std::fs::write(&csv, "secret").unwrap();
        let state = test_bound_state(&graph);
        assert!(read_text_file_from_path(&csv, &state).is_err());
        assert!(read_text_file_from_path(&graph.join("pages/../private.csv"), &state).is_err());
        #[cfg(unix)]
        {
            let alias = temp.path().join("alias.csv");
            std::os::unix::fs::symlink(&csv, &alias).unwrap();
            assert!(read_text_file_from_path(&alias, &state).is_err());
        }
    }

    #[test]
    fn read_local_image_requires_a_bound_root_check_before_metadata() {
        let source = include_str!("commands.rs");
        let image = source
            .rsplit("pub(crate) fn read_local_image(")
            .next()
            .unwrap();
        let image = image.split("pub(crate) fn import_asset(").next().unwrap();
        assert!(image.contains("refuse_bound_graph_path"));
    }

    fn test_bound_state(root: &std::path::Path) -> AppState {
        let (store, _, _) =
            tine_store::Store::open(root, tine_store::OpenOptions::default()).unwrap();
        let mut graphs = crate::state::GraphRegistry::default();
        graphs
            .bind(
                "main".into(),
                Arc::new(GraphSlot::new(store, root.to_path_buf())),
            )
            .unwrap();
        AppState {
            graphs: std::sync::RwLock::new(graphs),
            graph_load: std::sync::Mutex::new(()),
            last_focused: std::sync::Mutex::new(None),
            capture_graph: std::sync::Mutex::new(Default::default()),
            #[cfg(desktop)]
            next_window: std::sync::atomic::AtomicU64::new(1),
        }
    }

    #[test]
    fn read_local_image_refuses_bound_graph_through_alias_and_parent() {
        let temp = tempfile::tempdir().unwrap();
        let graph = temp.path().join("graph");
        std::fs::create_dir_all(graph.join("pages")).unwrap();
        let image = graph.join("pages/private.png");
        std::fs::write(&image, b"private").unwrap();
        let state = test_bound_state(&graph);
        assert!(
            refuse_bound_graph_path(&graph.join("pages/../pages/private.png"), &state).is_err()
        );
        #[cfg(unix)]
        {
            let alias = temp.path().join("alias.png");
            std::os::unix::fs::symlink(&image, &alias).unwrap();
            assert!(refuse_bound_graph_path(&alias, &state).is_err());
        }
    }
}

fn refuse_bound_graph_path(
    path: &std::path::Path,
    state: &AppState,
) -> Result<std::path::PathBuf, String> {
    let resolved = std::fs::canonicalize(path).map_err(|error| error.to_string())?;
    if state
        .graphs
        .read()
        .unwrap()
        .entries()
        .iter()
        .any(|(_, slot)| resolved.starts_with(&slot.root_key))
    {
        return Err("device read of a bound graph file is forbidden; use tine-store".into());
    }
    Ok(resolved)
}

mod query_error_wire;
use query_error_wire::{query_error, reference_error};

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
) -> Result<crate::settings::WorkspaceSaveOutcome, String> {
    crate::settings::save_workspaces(data, app, state)
}

fn feature_search_error(error: tine_graph_features::search::SearchError) -> String {
    match error {
        tine_graph_features::search::SearchError::Load(error) => {
            format!("graph load failed: {error:?}")
        }
        tine_graph_features::search::SearchError::Query(error) => query_error(error),
    }
}

#[cfg(test)]
mod result_bridge_budget_tests {
    use super::query_error;
    use tine_store::{Budget, QueryError};

    #[test]
    fn moved_read_errors_keep_the_existing_wire_text() {
        assert_eq!(
            query_error(QueryError::RequestTooLarge {
                what: Budget::BacklinkFilterRoots,
                count: 20_001,
                limit: 20_000,
            }),
            "too many backlink filter roots: 20001 (limit: 20000)"
        );
        assert_eq!(
            query_error(QueryError::ExportRequestTooLarge {
                macros: 1_025,
                bytes: 12,
                macro_limit: 1_024,
                byte_limit: 65_536,
                processing_cap: 64,
            }),
            "query-export-request-too-large: 1025 macros / 12 bytes (request limits: 1024 macros / 65536 bytes; processing cap: 64 macros)"
        );
        assert_eq!(
            query_error(QueryError::ResultTooLarge {
                what: Budget::BridgeMatchingBlocks,
                count: 5,
                limit: 20_000,
                bytes: Some(33_554_433),
                byte_limit: 33_554_432,
            }),
            "result-too-large: 5 matching blocks (~33554433 bytes); narrow the query or add (sample N) (limits: 20000 blocks / 33554432 bytes)"
        );
        assert_eq!(
            query_error(QueryError::bridge_search_hits(20_001, 10).unwrap()),
            "result-too-large: 20001 search hits (~10 bytes); narrow the search (limits: 20000 hits / 33554432 bytes)"
        );
    }
}

mod asset_ingress;
pub(crate) use asset_ingress::decode_asset_b64;

/// The whole name inventory: physical pages and journals, aliases, and names
/// that are only referenced. A thin adapter over `WholeGraph::inventory`; the
/// frontend caches it in `pageIndex.ts` and keeps no other name map.
///
/// Graph-wide off-thread; unreadable files are listed, never fail the inventory.
#[tauri::command]
pub(crate) async fn page_inventory(state: GraphContext<'_>) -> Result<PageInventoryWire, String> {
    let slot = slot_for_context(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        slot.store
            .whole_graph()
            .map_err(|e| format!("graph load failed: {e:?}"))
            .map(|view| page_inventory_wire(&view))
    })
    .await
    .map_err(|error| error.to_string())?
}

#[derive(Serialize)]
pub(crate) struct JournalFeedPage {
    pages: Vec<PageWire>,
    next_before_day: Option<i64>,
    done: bool,
    as_of_day: i64,
}

/// Feed-only pagination by ordinal day.
#[tauri::command]
pub(crate) async fn journal_feed_page(
    limit: usize,
    before_day: Option<i64>,
    state: GraphContext<'_>,
) -> Result<JournalFeedPage, String> {
    let slot = slot_for_context(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let feed = tine_graph_features::journals::feed_page(&slot.store, limit, before_day)
            .map_err(|error| error.to_string())?;
        Ok(JournalFeedPage {
            pages: feed.pages.into_iter().map(page_dto).collect(),
            next_before_day: feed.next_before_day,
            done: feed.done,
            as_of_day: feed.as_of_day,
        })
    })
    .await
    .map_err(|error| error.to_string())?
}
#[tauri::command]
pub(crate) async fn get_page(
    name: String,
    kind: PageKind,
    state: GraphContext<'_>,
) -> Result<Option<PageWire>, String> {
    let slot = slot_for_context(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        tine_graph_features::pages::get_page(&slot.store, &name, kind)
            .map(|read| read.map(page_dto))
            .map_err(feature_page_read_error)
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Waits for the initial load, so it runs on the blocking pool.
#[tauri::command]
pub(crate) async fn resolve_page(
    name: String,
    kind: PageKind,
    state: GraphContext<'_>,
) -> Result<ResolvedWire, String> {
    let slot = slot_for_context(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        slot.store
            .whole_graph()
            .map_err(|e| format!("graph load failed: {e:?}"))
            .map(|view| resolve_name(view, &name, kind == PageKind::Journal))
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Raw text of every Markdown/Org file in the open graph (`pages/`, plus
/// `journals/` when `include_journals`), for the "Help improve Tine" diff panel.
/// Mirrors `lsdoc/tools/graph-check.mjs`'s file scan: skips files over 8 MB, tags
/// format by extension, returns graph-root-relative paths sorted for stable
/// output. Read-only and local — the panel makes no network calls.
#[tauri::command]
pub(crate) async fn graph_source_files(
    include_journals: bool,
    state: GraphContext<'_>,
) -> Result<Vec<tine_graph_features::sources::GraphSourceFile>, String> {
    let slot = slot_for_context(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        tine_graph_features::sources::graph_source_files(&slot.store, include_journals)
    })
    .await
    .map_err(|error| error.to_string())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SavePageEntry {
    id: String,
    page: PageDto,
    base_rev: Option<String>,
    #[serde(default)]
    force: bool,
    kinds: Vec<tine_store::EditKind>,
}

fn log_save_kinds(entries: &[SavePageEntry]) {
    if crate::debug::debug_enabled() {
        for entry in entries {
            crate::debug::diag_private(
                "edit-kinds",
                format!("{}: {:?}", entry.page.name, entry.kinds),
            );
        }
    }
}

/// Require a current graph binding and nonempty edit kinds, then prepare
/// bases and run one ordered guarded page transaction. A missing/stale
/// binding or empty kinds returns command Err; preparation and transaction
/// failures return a Failed wire value. Force reads current UTF-8 bytes for
/// each affected base. Empty input returns Failed at placeholder index 0.
/// A failed or slow call is recorded as a fixed-shape `direct.save` event.
#[tauri::command]
pub(crate) fn save_pages(
    entries: Vec<SavePageEntry>,
    state: GraphContext<'_>,
) -> Result<SavePagesWire, String> {
    let slot = slot_for_context(&state)?;
    if entries.iter().any(|entry| entry.kinds.is_empty()) {
        return Err("OG-RULES Rule 8: every page write declares a non-empty edit kind list; exemplar src/document/save/engine.ts".into());
    }
    log_save_kinds(&entries);
    let entries: Vec<_> = entries
        .into_iter()
        .map(|entry| {
            (
                PageId::from(entry.id),
                entry.page,
                entry.base_rev,
                entry.force,
                entry.kinds,
            )
        })
        .collect();
    Ok(save_wire::save_pages_wire(
        &slot.store,
        &entries,
        tine_graph_features::pages::save_pages,
    ))
}

#[cfg(test)]
mod save_wire_tests {
    use super::*;

    #[test]
    fn save_wire_families_are_distinct() {
        const RULE: &str = "I-9: save failure families stay fixed while recovery locations remain explicit; exemplar commands::save_outcome_to_wire";
        assert_eq!(
            save_outcome_to_wire(SaveOutcome::Conflict {
                disk: String::from("rev").into()
            }),
            Err("conflict".into()),
            "{RULE}"
        );
        assert_eq!(
            save_outcome_to_wire(SaveOutcome::Deleted),
            Err("deleted".into()),
            "{RULE}"
        );
        assert_eq!(
            save_outcome_to_wire(SaveOutcome::Twin {
                existing: PageId::from("pages/secret.md".to_string())
            }),
            Err("twin".into()),
            "{RULE}"
        );
        assert_eq!(
            save_outcome_to_wire(SaveOutcome::Io(
                std::io::Error::new(std::io::ErrorKind::PermissionDenied, "/secret/path").into()
            )),
            Err("io:PermissionDenied".into()),
            "{RULE}"
        );
        let wire = save_pages_outcome_to_wire(SavePagesOutcome::Failed {
            index: 2,
            outcome: SaveOutcome::Repeated,
            undo_failed: vec![tine_store::FileId::from("pages/A.md".to_string())],
            publication_errors: Vec::new(),
        });
        let encoded = serde_json::to_string(&wire).unwrap();
        assert_eq!(
            encoded, r#"{"failed":{"index":2,"family":"repeated","undoFailed":["pages/A.md"]}}"#,
            "{RULE}"
        );
        let incomplete = save_pages_outcome_to_wire(SavePagesOutcome::Failed {
            index: 0,
            outcome: SaveOutcome::Io(std::io::Error::other("publication failed").into()),
            undo_failed: Vec::new(),
            publication_errors: vec![tine_store::FileId::from("pages/A.md".to_string())],
        });
        assert_eq!(
            serde_json::to_string(&incomplete).unwrap(),
            r#"{"failed":{"index":0,"family":"publication-incomplete","undoFailed":[],"publicationErrors":["pages/A.md"]}}"#,
            "{RULE}"
        );
        let families = [
            (
                SaveOutcome::Conflict {
                    disk: "private-rev".to_string().into(),
                },
                "conflict",
            ),
            (SaveOutcome::Deleted, "deleted"),
            (
                SaveOutcome::Twin {
                    existing: PageId::from("pages/private-title.md"),
                },
                "twin",
            ),
            (SaveOutcome::Repeated, "repeated"),
            (SaveOutcome::ReadOnly("private title".into()), "read-only"),
            (
                SaveOutcome::InvalidTarget("/private/path".into()),
                "invalid-target",
            ),
            (SaveOutcome::Closed, "closed"),
            (
                SaveOutcome::Io(
                    std::io::Error::new(std::io::ErrorKind::PermissionDenied, "/private/path")
                        .into(),
                ),
                "io:PermissionDenied",
            ),
        ];
        let mut seen = std::collections::HashSet::new();
        for (outcome, family) in families {
            let encoded =
                serde_json::to_string(&save_pages_outcome_to_wire(SavePagesOutcome::Failed {
                    index: 1,
                    outcome,
                    undo_failed: vec![tine_store::FileId::from("pages/A.md".to_string())],
                    publication_errors: Vec::new(),
                }))
                .unwrap();
            let rev_field = if family == "conflict" {
                r#","diskRev":"private-rev""#
            } else {
                ""
            };
            assert_eq!(
                encoded,
                format!(
                    r#"{{"failed":{{"index":1,"family":"{family}"{rev_field},"undoFailed":["pages/A.md"]}}}}"#
                ),
                "{RULE}"
            );
            assert!(seen.insert(family), "{RULE}");
        }
        assert_eq!(
            asset_error(StoreError::TooLarge { limit: 12, len: 13 }),
            "asset-too-large",
            "{RULE}"
        );
        assert_eq!(
            sync_conflict_error(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "secret"
            )),
            "conflict",
            "{RULE}"
        );
        assert_eq!(
            sync_conflict_error(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "/secret/path"
            )),
            "io:PermissionDenied",
            "{RULE}"
        );
    }
    #[test]
    fn fail_read_concord_adapter_keeps_recovery_evidence() {
        let token = sync_conflict_error(std::io::Error::other(
            tine_graph_features::IncompleteTransaction::Rollback(
                "recovery: logseq/.tine-trash/a.md".into(),
            ),
        ));
        assert!(
            token.contains("rollback-incomplete"),
            "I-2/I-9: an adapter must retain incomplete transaction evidence"
        );
        assert!(token.contains("logseq/.tine-trash/a.md"));
    }
}

#[tauri::command]
pub(crate) fn guide_pages() -> Vec<tine_core::guide::GuidePage> {
    tine_core::guide::bundled_guide_pages()
}

#[tauri::command]
pub(crate) async fn copy_guide_into_graph(
    title: String,
    state: GraphContext<'_>,
) -> Result<tine_graph_features::guide::GuideCopyResult, String> {
    let slot = slot_for_context(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        tine_graph_features::guide::copy_guide_into_graph(&slot.store, &title)
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn get_backlinks(
    name: String,
    state: GraphContext<'_>,
) -> Result<Arc<Vec<RefGroup>>, String> {
    let slot = slot_for_context(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let view = slot
            .store
            .whole_graph()
            .map_err(|e| format!("graph load failed: {e:?}"))?;
        view.backlinks(&name).map_err(reference_error)
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
    let slot = slot_for_context(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let view = slot
            .store
            .whole_graph()
            .map_err(|e| format!("graph load failed: {e:?}"))?;
        view.backlink_filter_context(&name, &targets)
            .map_err(query_error)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn get_unlinked_refs(
    name: String,
    state: GraphContext<'_>,
) -> Result<Arc<Vec<RefGroup>>, String> {
    let slot = slot_for_context(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let view = slot
            .store
            .whole_graph()
            .map_err(|e| format!("graph load failed: {e:?}"))?;
        view.unlinked_references(&name).map_err(reference_error)
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
    let slot = slot_for_context(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        slot.store
            .whole_graph()
            .map(|view| view.block_ref_counts())
            .map_err(|e| format!("graph load failed: {e:?}"))
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
    let slot = slot_for_context(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let view = slot
            .store
            .whole_graph()
            .map_err(|e| format!("graph load failed: {e:?}"))?;
        view.block_referrers(&uuid).map_err(query_error)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn delete_page(
    name: String,
    kind: PageKind,
    expected_path: Option<String>,
    state: GraphContext<'_>,
) -> Result<(), String> {
    let slot = slot_for_context(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        tine_graph_features::pages::delete_page_expected(
            &slot.store,
            &name,
            kind,
            expected_path.as_deref(),
            None,
        )
        .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn rename_page(
    old: String,
    new: String,
    expected_path: Option<String>,
    merge_into: Option<String>,
    unsaved_paths: Option<Vec<String>>,
    state: GraphContext<'_>,
) -> Result<tine_graph_features::pages::RenameReport, String> {
    let slot = slot_for_context(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        tine_graph_features::pages::rename_or_merge_page(
            &slot.store,
            &old,
            &new,
            expected_path.as_deref(),
            merge_into.as_deref(),
            unsaved_paths.as_deref().unwrap_or_default(),
        )
        .map_err(|e| e.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[cfg(test)]
mod graph_wide_command_boundary_tests {
    #[test]
    fn expensive_reference_and_rename_commands_cross_the_blocking_pool() {
        let source = include_str!("commands.rs");
        for name in [
            "get_backlinks",
            "get_unlinked_refs",
            "rename_page",
            "delete_page",
            "merge_pages",
            "rename_file_to_page",
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

#[tauri::command]
pub(crate) async fn publish_html(state: GraphContext<'_>) -> Result<(String, usize), String> {
    let slot = slot_for_context(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        tine_graph_features::publish::publish_html(&slot.store).map_err(|error| error.to_string())
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
    opts: tine_graph_features::print::PrintOpts,
    sheets: Vec<tine_graph_features::SheetExport>,
    state: GraphContext<'_>,
) -> Result<String, String> {
    let slot = slot_for_context(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        tine_graph_features::print::page_print_html_with_sheets(&slot.store, &name, opts, sheets)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "no-page".to_string())
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
    let slot = slot_for_context(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let view = slot
            .store
            .whole_graph()
            .map_err(|e| format!("graph load failed: {e:?}"))?;
        view.export_query_subtrees(&specs).map_err(query_error)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QueryPageScope {
    name: String,
    page_kind: PageKind,
    #[serde(default)]
    path: Option<String>,
}

#[tauri::command]
pub(crate) async fn run_graph_search(
    source: String,
    page_limit: usize,
    block_limit: usize,
    lane: Option<String>,
    explain: bool,
    scope: Option<QueryPageScope>,
    page_match_scope: Option<tine_core::query::ir::FriendlyPageMatchScope>,
    page_view: Option<tine_core::query::ir::ViewSettings>,
    block_view: Option<tine_core::query::ir::ViewSettings>,
    state: GraphContext<'_>,
) -> Result<tine_core::query_plan::QueryExecution, String> {
    let slot = slot_for_context(&state)?;
    let scope = scope.map(|scope| tine_graph_features::search::Scope {
        name: scope.name,
        kind: scope.page_kind,
        path: scope.path,
    });
    tauri::async_runtime::spawn_blocking(move || {
        tine_graph_features::search::run_graph_search(
            &slot.store,
            &slot.block_search_lanes,
            source,
            page_limit,
            block_limit,
            lane.as_deref(),
            explain,
            scope,
            page_match_scope,
            page_view,
            block_view,
        )
        .map_err(feature_search_error)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn query_facets(
    state: GraphContext<'_>,
    autocomplete: Option<bool>,
) -> Result<Vec<(String, Vec<String>)>, String> {
    let policy = if autocomplete.unwrap_or(false) {
        FacetPolicy::Truncated
    } else {
        FacetPolicy::Budgeted
    };
    let slot = slot_for_context(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let view = slot
            .store
            .whole_graph()
            .map_err(|e| format!("graph load failed: {e:?}"))?;
        view.property_facets(policy).map_err(query_error)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn page_icons(
    names: Vec<String>,
    state: GraphContext<'_>,
) -> Result<std::collections::HashMap<String, String>, String> {
    let slot = slot_for_context(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        slot.store
            .whole_graph()
            .map(|view| view.page_icons(&names))
            .map_err(|e| format!("graph load failed: {e:?}"))
    })
    .await
    .map_err(|error| error.to_string())?
}

fn with_config_store<T>(
    state: &GraphContext<'_>,
    f: impl FnOnce(&tine_store::Store) -> Result<T, String>,
) -> Result<T, String> {
    let slot = slot_for_context(state)?;
    f(&slot.store)
}

#[tauri::command]
pub(crate) fn set_preferred_workflow(
    workflow: String,
    state: GraphContext<'_>,
) -> Result<(), String> {
    with_config_store(&state, |store| {
        tine_graph_features::config::set_preferred_workflow(store, &workflow)
            .map_err(|e| e.to_string())
    })
}

#[tauri::command]
pub(crate) fn set_timetracking_enabled(
    enabled: bool,
    state: GraphContext<'_>,
) -> Result<(), String> {
    with_config_store(&state, |store| {
        tine_graph_features::config::set_timetracking_enabled(store, enabled)
            .map_err(|e| e.to_string())
    })?;
    Ok(())
}

#[tauri::command]
pub(crate) fn set_show_brackets(enabled: bool, state: GraphContext<'_>) -> Result<(), String> {
    with_config_store(&state, |store| {
        tine_graph_features::config::set_show_brackets(store, enabled).map_err(|e| e.to_string())
    })?;
    Ok(())
}

#[tauri::command]
pub(crate) fn set_doc_mode_enter_for_new_block(
    enabled: bool,
    state: GraphContext<'_>,
) -> Result<(), String> {
    with_config_store(&state, |store| {
        tine_graph_features::config::set_doc_mode_enter_for_new_block(store, enabled)
            .map_err(|e| e.to_string())
    })?;
    Ok(())
}

#[tauri::command]
pub(crate) fn set_logical_outdenting(enabled: bool, state: GraphContext<'_>) -> Result<(), String> {
    with_config_store(&state, |store| {
        tine_graph_features::config::set_logical_outdenting(store, enabled)
            .map_err(|e| e.to_string())
    })?;
    Ok(())
}

#[tauri::command]
pub(crate) fn set_guide_announced(announced: bool, state: GraphContext<'_>) -> Result<(), String> {
    with_config_store(&state, |store| {
        tine_graph_features::config::set_guide_announced(store, announced)
            .map_err(|e| e.to_string())
    })?;
    Ok(())
}

#[tauri::command]
pub(crate) fn set_default_journal_template(
    name: Option<String>,
    state: GraphContext<'_>,
) -> Result<(), String> {
    with_config_store(&state, |store| {
        tine_graph_features::config::set_default_journal_template(store, name.as_deref())
            .map_err(|e| e.to_string())
    })
}

#[tauri::command]
pub(crate) fn set_start_of_week(n: u32, state: GraphContext<'_>) -> Result<(), String> {
    with_config_store(&state, |store| {
        tine_graph_features::config::set_start_of_week(store, n).map_err(|e| e.to_string())
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
    with_config_store(&state, |store| {
        tine_graph_features::config::set_preferred_format(store, fmt).map_err(|e| e.to_string())
    })?;
    Ok(())
}

/// Set `:journal/page-title-format` (e.g. "MMM do, yyyy") in config.edn,
/// unvalidated. Renames no files: title-named journals are only proposed and
/// applied through the journal filename commands (master e6f9b6e1ceae).
#[tauri::command]
pub(crate) fn set_journal_title_format(
    format: String,
    state: GraphContext<'_>,
) -> Result<(), String> {
    let slot = slot_for_context(&state)?;
    tine_graph_features::config::set_journal_page_title_format(&slot.store, &format)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) fn read_custom_css(state: GraphContext<'_>) -> Result<String, String> {
    with_config_store(&state, |store| {
        config::custom_css(store).map_err(|error| error.to_string())
    })
}

#[tauri::command]
pub(crate) async fn search(
    query: String,
    limit: usize,
    lane: Option<String>,
    state: GraphContext<'_>,
) -> Result<Vec<RefGroup>, String> {
    let slot = slot_for_context(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        tine_graph_features::search::find_blocks(
            &slot.store,
            &slot.block_search_lanes,
            &query,
            limit,
            lane.as_deref(),
        )
        .map_err(feature_search_error)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn quick_switch(
    query: String,
    limit: usize,
    state: GraphContext<'_>,
) -> Result<Vec<PageEntry>, String> {
    let slot = slot_for_context(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        slot.store
            .whole_graph()
            .map(|view| view.complete_page_names(&query, limit))
            .map_err(|e| format!("graph load failed: {e:?}"))
    })
    .await
    .map_err(|error| error.to_string())?
}

#[cfg(test)]
fn capture_quick_switch_for(
    state: &AppState,
    caller: &str,
    binding_generation: Option<u64>,
    query: &str,
    limit: usize,
) -> Result<Vec<PageEntry>, String> {
    let slot = capture_quick_switch_slot(state, caller, binding_generation)?;
    let view = slot
        .store
        .whole_graph()
        .map_err(|e| format!("graph load failed: {e:?}"))?;
    Ok(view.complete_page_names(query, limit.min(8)))
}

/// The sole graph-backed capability exposed to Quick Capture. It is deliberately
/// not a `GraphContext` command: capture may ask for bounded page/tag candidates
/// but cannot save, delete, trash, or invoke any other graph command.
#[tauri::command]
pub(crate) async fn capture_quick_switch(
    query: String,
    limit: usize,
    binding_generation: Option<u64>,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<Vec<PageEntry>, String> {
    let slot = capture_quick_switch_slot(&state, window.label(), binding_generation)?;
    tauri::async_runtime::spawn_blocking(move || {
        let view = slot
            .store
            .whole_graph()
            .map_err(|e| format!("graph load failed: {e:?}"))?;
        Ok(view.complete_page_names(&query, limit.min(8)))
    })
    .await
    .map_err(|error| error.to_string())?
}

#[cfg(test)]
mod capture_quick_switch_tests {
    use super::*;
    use crate::state::{slot_for_bound_window, GraphRegistry, GraphSlot};
    use std::path::PathBuf;
    use std::sync::atomic::AtomicU64;
    use std::sync::{Mutex, RwLock};

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
            last_focused: Mutex::new(Some("main".into())),
            capture_graph: Mutex::new(Default::default()),
            #[cfg(desktop)]
            next_window: AtomicU64::new(2),
        };
        let (selected_store, _, _) =
            tine_store::Store::open(&selected, tine_store::OpenOptions::default()).unwrap();
        let selected_slot = Arc::new(GraphSlot::new(selected_store, selected.clone()));
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
                Arc::new(GraphSlot::new(
                    tine_store::Store::open(&other, tine_store::OpenOptions::default())
                        .unwrap()
                        .0,
                    other,
                )),
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
        // `save_pages` and other mutations resolve through GraphContext, which
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

/// Run a whole-graph read on the blocking pool. `Store::whole_graph` waits for
/// the initial parse (seconds on a 10k-page graph); a synchronous Tauri command
/// runs on the main thread and would freeze the webview for that whole wait.
async fn off_ui_graph_read<T: Send + 'static>(
    state: &GraphContext<'_>,
    read: impl FnOnce(WholeGraph) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let slot = slot_for_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let view = slot
            .store
            .whole_graph()
            .map_err(|e| format!("graph load failed: {e:?}"))?;
        read(view)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn list_templates(
    state: GraphContext<'_>,
) -> Result<Vec<tine_core::model::TemplateDto>, String> {
    off_ui_graph_read(&state, |view| Ok(view.templates())).await
}

#[tauri::command]
pub(crate) async fn journal_content_days(state: GraphContext<'_>) -> Result<Vec<i64>, String> {
    off_ui_graph_read(&state, |view| {
        Ok(view
            .journal_content_days()
            .into_iter()
            .map(|day| day.0)
            .collect())
    })
    .await
}

#[tauri::command]
pub(crate) async fn resolve_block(
    uuid: String,
    state: GraphContext<'_>,
) -> Result<Option<RefGroup>, String> {
    off_ui_graph_read(&state, move |view| {
        Ok(view.blocks(&[uuid]).map_err(query_error)?.pop().flatten())
    })
    .await
}

#[tauri::command]
pub(crate) async fn resolve_blocks(
    uuids: Vec<String>,
    state: GraphContext<'_>,
) -> Result<Vec<Option<RefGroup>>, String> {
    off_ui_graph_read(&state, move |view| view.blocks(&uuids).map_err(query_error)).await
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
    off_ui_graph_read(&state, move |view| {
        view.preview_block(&uuid, max_nodes).map_err(query_error)
    })
    .await
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
    let slot = slot_for_context(&state)?;
    tine_graph_features::assets::read_asset(&slot.store, &name, max_bytes)
        .map(tauri::ipc::Response::new)
        .map_err(feature_asset_access_error)
}

/// Validate one graph media file and return its top-level asset name for the
/// range-aware `tine-media:` protocol. The protocol revalidates against the
/// requesting window's current graph on every request.
#[tauri::command]
pub(crate) fn stream_asset_path(name: String, state: GraphContext<'_>) -> Result<String, String> {
    let slot = slot_for_context(&state)?;
    tine_graph_features::assets::validate_stream_asset(&slot.store, &name)
        .map_err(feature_asset_access_error)?;
    Ok(format!("{}/{}", slot.binding_generation, name))
}

/// Quit the app cleanly. On Linux, first SIGKILL WebKitGTK's helper subprocesses so
/// they don't run their buggy GL-driver atexit teardown and dump a SIGABRT core on
/// exit (GH #28). The JS close handler calls this only AFTER `flushAll()`/
/// `flushSession()` have resolved, so tearing the web process down hard loses no
/// edits. Then hand off to Tauri's normal exit (the main process still tears down
/// the way it always has — no dump there). On non-Linux this is just `app.exit(0)`.
#[tauri::command]
pub(crate) fn tine_quit(app: tauri::AppHandle) {
    #[cfg(target_os = "linux")]
    crate::platform::kill_webkit_children();
    app.exit(0);
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

/// Read an opted-in local image by absolute path outside every bound graph.
/// Symlinks resolve before the graph-scope check; non-image extensions,
/// non-regular files and files over 64 MiB fail with a string error. The
/// bounded read also stops if a regular file grows after metadata was read.
#[tauri::command]
pub(crate) fn read_local_image(
    path: String,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<tauri::ipc::Response, String> {
    // Read an image from an ABSOLUTE path OUTSIDE the graph, for raw-HTML `<img>`
    // srcs the user has explicitly opted into (Settings → "Load local-file images").
    // OFF by default; gated here too (defense in depth — the frontend also checks),
    // restricted to image extensions + a size cap so an allowed note can't slurp an
    // arbitrary file. Returns RAW bytes like `read_asset`. See ADR 0019.
    if !crate::settings::device_bool(&app, "allow_local_file_images", false) {
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
    let p = refuse_bound_graph_path(p, &state)?;
    const MAX_BYTES: u64 = 64 * 1024 * 1024;
    crate::device_io::read_regular_file_bounded(&p, MAX_BYTES).map(tauri::ipc::Response::new)
}

#[tauri::command]
pub(crate) fn import_asset(
    path: String,
    name: Option<String>,
    state: GraphContext<'_>,
) -> Result<String, String> {
    let slot = slot_for_context(&state)?;
    crate::device_io::import_asset_from_path(&slot.store, &path, name.as_deref()).map_err(|error| {
        match error {
            crate::device_io::DeviceAssetImportError::Name(message) => message,
            crate::device_io::DeviceAssetImportError::Io(error) => {
                feature_asset_error(error, &slot)
            }
        }
    })
}

/// Import a bounded Android photo or voice memo by native cache-file capability.
/// Media never crosses Kotlin/WebView/Rust as base64; Rust streams the open file
/// into the graph and removes the temp only after the durable asset commit.
#[tauri::command]
pub(crate) fn import_native_capture(
    path: String,
    name: String,
    graph_root: Option<String>,
    app: tauri::AppHandle,
    state: GraphContext<'_>,
) -> Result<String, String> {
    use cap_std::{ambient_authority, fs::Dir};
    use tauri::Manager;
    let target = crate::capture_target::pick(slot_for_context(&state)?, graph_root, &app, &state)?;
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
    let stored = tine_graph_features::assets::import_asset_file(
        target.store(),
        &name,
        tine_store::Content::Stream {
            source: capture.into_std(),
            max_bytes,
        },
    )
    .map_err(|error| target.asset_error(error))?;
    // The graph asset is authoritative now. Cleanup failure is harmless cache
    // litter and must not make the frontend omit the already-durable reference.
    let _ = cache_dir.remove_file(filename);
    Ok(stored)
}

/// Read a dropped delimited-text file for the CSV/TSV → grid drop path.
/// Deliberately narrow: this caller-chosen path is restricted to the drop
/// feature's delimited text types. `read_local_image` has a separate image gate.
#[tauri::command]
pub(crate) fn read_text_file(path: String, state: State<'_, AppState>) -> Result<String, String> {
    read_text_file_from_path(std::path::Path::new(&path), &state)
}

fn read_text_file_from_path(p: &std::path::Path, state: &AppState) -> Result<String, String> {
    fn delimited_ext(p: &std::path::Path) -> bool {
        p.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("csv") || e.eq_ignore_ascii_case("tsv"))
            .unwrap_or(false)
    }
    if !delimited_ext(p) {
        return Err("unsupported file type".into());
    }
    // Re-check on the RESOLVED path too — a symlink named x.csv pointing at an
    // arbitrary file must not pass the extension gate (review finding).
    let resolved = refuse_bound_graph_path(p, state)?;
    if !delimited_ext(&resolved) {
        return Err("unsupported file type".into());
    }
    const MAX_BYTES: u64 = 10 * 1024 * 1024;
    let bytes = crate::device_io::read_regular_file_bounded(&resolved, MAX_BYTES)?;
    String::from_utf8(bytes).map_err(|e| e.to_string())
}

/// Open an `assets/`-relative file, directory, or (empty name) the assets root (GH #367)
/// in the OS default app / file manager. Gated to the canonical assets dir.
#[tauri::command]
pub(crate) fn open_asset(name: String, state: GraphContext<'_>) -> Result<(), String> {
    let slot = slot_for_context(&state)?;
    let target = tine_graph_features::assets::path_for_os_open(&slot.store, &name)
        .map_err(feature_asset_access_error)?;
    open_asset_with_os(&name, &target, false)
}

fn open_asset_with_os(name: &str, target: &std::path::Path, editing: bool) -> Result<(), String> {
    #[cfg(desktop)]
    {
        #[cfg(target_os = "linux")]
        let prog = "xdg-open";
        #[cfg(target_os = "macos")]
        let prog = "open";
        #[cfg(target_os = "windows")]
        let prog = "explorer";
        let (tag, action, q) = if editing {
            ("edit-asset", "edit_asset_external", "opener ")
        } else {
            ("open-asset", "open_asset", "")
        };
        let shown = target.display();
        diag_private(tag, format!("{action}: {name} -> {shown} ({q}{prog})"));
        opener_command(prog)
            .arg(&target)
            .spawn()
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    // Mobile: opening an asset in an external app uses a platform intent; stub for now (M1).
    #[cfg(not(desktop))]
    {
        let _ = (name, target, editing);
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
    let slot = slot_for_context(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let target = tine_graph_features::pages::source_path_for_os_handoff(
            &slot.store,
            &name,
            kind,
            path.as_deref(),
        )
        .map_err(feature_page_read_error)?;
        open_page_source_with_os(&target, reveal)
    })
    .await
    .map_err(|error| error.to_string())?
}

fn open_page_source_with_os(target: &std::path::Path, reveal: bool) -> Result<(), String> {
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
    let slot = slot_for_context(&state)?;
    let target = asset_handoff_target(&slot, &name)?;
    edit_asset_with_os(&name, &command, &target)
}

fn edit_asset_with_os(name: &str, command: &str, target: &std::path::Path) -> Result<(), String> {
    #[cfg(desktop)]
    {
        let target_str = target.to_string_lossy().to_string();
        if command.trim().is_empty() {
            return open_asset_with_os(name, target, true);
        }
        let (prog, args) = build_editor_argv(command.trim(), &target_str)?;
        diag_private(
            "edit-asset",
            format!("edit_asset_external: {name} -> {prog} {args:?}"),
        );
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
pub(crate) async fn list_orphan_assets(state: GraphContext<'_>) -> Result<Vec<AssetInfo>, String> {
    let slot = slot_for_context(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        tine_graph_features::assets::orphan_assets(&slot.store).map_err(sync_conflict_error)
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Move an orphaned asset to the recoverable trash.
#[tauri::command]
pub(crate) async fn trash_asset(name: String, state: GraphContext<'_>) -> Result<(), String> {
    let slot = slot_for_context(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        tine_graph_features::assets::trash_asset(&slot.store, &name)
            .map_err(|error| feature_asset_error(error, &slot))
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Count + total bytes in the recoverable asset trash.
#[tauri::command]
pub(crate) async fn asset_trash_stats(
    state: GraphContext<'_>,
) -> Result<tine_core::model::TrashStats, String> {
    let slot = slot_for_context(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        tine_graph_features::assets::asset_trash_stats(&slot.store).map_err(store_error)
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Permanently delete everything in the asset trash; returns files removed.
#[tauri::command]
pub(crate) fn empty_asset_trash(state: GraphContext<'_>) -> Result<u64, String> {
    slot_for_context(&state)?
        .store
        .purge_asset_trash()
        .map(|(count, _)| count)
        .map_err(|(error, count, bytes)| {
            format!(
                "{} ({count} entries, {bytes} bytes already removed)",
                store_error(error)
            )
        })
}

/// Journal days that resolve to more than one file (e.g. a date-stem file plus a
/// title-named one) — for the user to reconcile.
#[tauri::command]
pub(crate) async fn list_journal_conflicts(
    state: GraphContext<'_>,
) -> Result<Vec<tine_core::model::JournalConflict>, String> {
    let slot = slot_for_context(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        tine_graph_features::journals::journal_conflicts(&slot.store)
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Proposed journal date-name renames; opening a graph never performs them.
#[tauri::command]
pub(crate) async fn list_journal_filename_migrations(
    state: GraphContext<'_>,
) -> Result<Vec<JournalFilenameMigration>, String> {
    let slot = slot_for_context(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        journals::journal_filename_migrations(&slot.store).map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Snapshot (O(graph bytes)), then rename only still-valid confirmed proposals, one file each.
#[tauri::command]
pub(crate) async fn apply_journal_filename_migrations(
    app: tauri::AppHandle,
    state: GraphContext<'_>,
    migrations: Vec<JournalFilenameMigration>,
) -> Result<journals::MigrationResult, String> {
    let slot = slot_for_context(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        crate::backup::snapshot_before_rewrite(&app, &slot, "pre-journal-rename")?;
        let result = journals::migrate_journal_filenames(&slot.store, &migrations)
            .map_err(|error| error.to_string())?;
        Ok(result)
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Move one journal file (by exact filename) to the recoverable trash.
#[tauri::command]
pub(crate) fn trash_journal_file(name: String, state: GraphContext<'_>) -> Result<(), String> {
    let slot = slot_for_context(&state)?;
    tine_graph_features::journals::trash_journal_file(&slot.store, &name).map_err(|e| e.to_string())
}

/// Raw contents of one journal file (by exact filename) — for inspecting a
/// duplicate day's files before reconciling.
#[tauri::command]
pub(crate) fn read_journal_file(name: String, state: GraphContext<'_>) -> Result<String, String> {
    let slot = slot_for_context(&state)?;
    tine_graph_features::journals::read_journal_file(&slot.store, &name).map_err(|e| e.to_string())
}

/// Load a page from a SPECIFIC file by its graph-root-relative path — lets the UI
/// navigate to a duplicate-day stray that shares a (kind,name) with the canonical
/// file and so is unreachable by name (#21).
#[tauri::command]
pub(crate) fn get_page_by_path(
    path: String,
    state: GraphContext<'_>,
) -> Result<Option<PageWire>, String> {
    let slot = slot_for_context(&state)?;
    match slot.store.page(&PageId::from(path)) {
        Ok(read) => Ok(Some(page_dto(read))),
        Err(StoreError::NotFound | StoreError::InvalidTarget(_)) => Ok(None),
        Err(error) => Err(store_error(error)),
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
    let slot = slot_for_context(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        tine_graph_features::pages::merge_pages(&slot.store, &src, &dst)
            .map_err(graph_write_error_to_wire)
    })
    .await
    .map_err(|error| error.to_string())?
}

fn graph_write_error_to_wire(error: std::io::Error) -> String {
    error.to_string()
}

#[cfg(test)]
#[test]
fn graph_write_wire_keeps_rollback_incomplete_family() {
    let error = std::io::Error::other(
        "rollback-incomplete: undo failed for pages/A.md; recovery: logseq/.tine-trash/r/A.md",
    );
    let wire = graph_write_error_to_wire(error);
    assert!(
        wire.starts_with("rollback-incomplete:"),
        "I-9: graph command wire must preserve rollback-incomplete; exemplar merge_pages: {wire}"
    );
    assert!(
        wire.contains(".tine-trash/r/A.md"),
        "I-9: graph command wire must preserve recovery location; exemplar merge_pages: {wire}"
    );
}

/// Rescue a duplicate-day stray by moving it to a uniquely-named page
/// (`pages/<new_name>`), so it stops colliding and becomes normally navigable (#21).
#[tauri::command]
pub(crate) async fn rename_file_to_page(
    path: String,
    new_name: String,
    state: GraphContext<'_>,
) -> Result<(), String> {
    let slot = slot_for_context(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        tine_graph_features::pages::rename_file_to_page(&slot.store, &path, &new_name)
            .map_err(|error| error.to_string())
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
    let slot = slot_for_context(&state)?;
    tine_graph_features::assets::save_asset(&slot.store, &name, &bytes)
        .map_err(|error| feature_asset_error(error, &slot))
}

mod read_highlights_worker;
#[tauri::command]
/// Bound-graph read; missing sidecar is empty, malformed EDN/I/O/join errors refuse.
pub(crate) async fn read_highlights(
    pdf: String,
    state: GraphContext<'_>,
) -> Result<Vec<tine_core::pdf::Highlight>, String> {
    let slot = slot_for_context(&state)?;
    read_highlights_worker::read(slot, pdf).await
}

#[tauri::command]
pub(crate) async fn open_pdf(
    pdf: String,
    label: String,
    state: GraphContext<'_>,
) -> Result<tine_core::pdf::PdfState, String> {
    let slot = slot_for_context(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        tine_graph_features::pdf::open_pdf(&slot.store, &pdf, &label).map_err(feature_pdf_error)
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Require a current graph binding; run the guarded merge on a blocking worker.
/// Dropping the caller does not cancel a started write. WouldBlock returns the
/// fixed `conflict` token; other I/O and join errors stringify. Retain edits.
#[tauri::command]
pub(crate) async fn write_highlights(
    pdf: String,
    label: String,
    highlights: Vec<tine_core::pdf::Highlight>,
    base_highlights: Vec<tine_core::pdf::Highlight>,
    state: GraphContext<'_>,
) -> Result<Vec<tine_core::pdf::Highlight>, String> {
    let slot = slot_for_context(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        tine_graph_features::pdf::write_highlights(
            &slot.store,
            &pdf,
            &label,
            &highlights,
            &base_highlights,
        )
        .map_err(feature_pdf_error)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) fn write_pdf_view_state(
    pdf: String,
    page: i64,
    scale: f64,
    state: GraphContext<'_>,
) -> Result<(), String> {
    let slot = slot_for_context(&state)?;
    tine_graph_features::pdf::write_pdf_view_state(&slot.store, &pdf, page, scale)
        .map_err(feature_pdf_error)
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
    let slot = slot_for_context(&state)?;
    tine_graph_features::pdf::write_pdf_area_image(&slot.store, &pdf, page, &id, stamp, &bytes)
        .map_err(feature_pdf_error)
}
