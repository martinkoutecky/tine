#[cfg(desktop)]
use crate::debug::diag;
#[cfg(desktop)]
use crate::platform::{open_page_source, opener_command, reveal_page_source};
use crate::state::{
    capture_quick_switch_slot, slot_for_context, AppState, GraphContext, GraphSlot,
};
use serde::Serialize;
use std::sync::Arc;
use tauri::{State, WebviewWindow};
use tine_core::model::{
    BacklinkFilterContext, BacklinkFilterTarget, PageDto, PageEntry, PageKind, RefGroup,
};
#[cfg(test)]
use tine_store::SaveBase;
use tine_store::{
    Budget, FacetPolicy, PageId, QueryError, Resolved, SaveOutcome, StoreError, WholeGraph,
};

fn feature_asset_error(error: std::io::Error, slot: &GraphSlot) -> String {
    tine_graph_features::assets::error_for_user(&slot.store, error)
}
fn feature_pdf_error(error: std::io::Error) -> String {
    error.to_string()
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
        StoreError::InvalidTarget(_) => "invalid page path".into(),
        StoreError::Undecodable => "stream did not contain valid UTF-8".into(),
        StoreError::Unparseable(reason) => reason,
        StoreError::TooLarge { .. } => "asset-too-large".into(),
        StoreError::Io(error) => error.to_string(),
        StoreError::Closed => "store closed".into(),
    }
}

fn save_store_error(error: StoreError) -> String {
    match error {
        StoreError::NotFound => "deleted".into(),
        StoreError::InvalidTarget(_) | StoreError::Undecodable | StoreError::Unparseable(_) => {
            "invalid-target".into()
        }
        StoreError::TooLarge { .. } => "asset-too-large".into(),
        StoreError::Io(error) => format!("io:{:?}", error.kind()),
        StoreError::Closed => "closed".into(),
    }
}

fn asset_error(error: StoreError) -> String {
    match error {
        StoreError::NotFound => std::io::Error::from_raw_os_error(2).to_string(),
        StoreError::InvalidTarget(_) => "invalid asset".into(),
        StoreError::TooLarge { .. } => "asset-too-large".into(),
        other => store_error(other),
    }
}

fn sync_conflict_error(error: std::io::Error) -> String {
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
            capture_graph: std::sync::Mutex::new(None),
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

fn whole_graph(state: &GraphContext<'_>) -> Result<WholeGraph, String> {
    slot_for_context(state)?
        .store
        .whole_graph()
        .map_err(|e| format!("graph load failed: {e:?}"))
}

fn query_error(error: QueryError) -> String {
    match error {
        QueryError::Cancelled => "cancelled".into(),
        QueryError::Parse(reason) => reason,
        QueryError::InvalidTarget(_) => "invalid page path".into(),
        QueryError::RequestTooLarge { what: Budget::BacklinkFilterRoots, count, limit } =>
            format!("too many backlink filter roots: {count} (limit: {limit})"),
        QueryError::RequestTooLarge { what, count, limit } =>
            format!("request-too-large: {count} {} (limit: {limit})", budget_text(what)),
        QueryError::ExportRequestTooLarge { macros, bytes, macro_limit, byte_limit, processing_cap } =>
            format!("query-export-request-too-large: {macros} macros / {bytes} bytes (request limits: {macro_limit} macros / {byte_limit} bytes; processing cap: {processing_cap} macros)"),
        QueryError::ResultTooLarge { what: Budget::PropertyFacets, .. } =>
            "result-too-large: property facets exceed the construction budget".into(),
        QueryError::ResultTooLarge { what: Budget::ResolvedBlockRows, count, .. } =>
            format!("result-too-large: {count} resolved block-reference rows exceed the construction budget"),
        QueryError::ResultTooLarge { what: Budget::RequestedBlockRefs, count, limit, .. } =>
            format!("result-too-large: {count} requested block references (limit: {limit})"),
        QueryError::ResultTooLarge { what: Budget::ExportBytes, count, limit, .. } =>
            format!("query-export-result-too-large: ~{count} bytes (limit: {limit} bytes)"),
        QueryError::ResultTooLarge { what: Budget::BridgeMatchingBlocks, count, limit, bytes: Some(bytes), byte_limit } =>
            format!("result-too-large: {count} matching blocks (~{bytes} bytes); narrow the query or add (sample N) (limits: {limit} blocks / {byte_limit} bytes)"),
        QueryError::ResultTooLarge { what: Budget::MatchingBlocks, count, limit, byte_limit, .. } =>
            format!("result-too-large: {count} matching blocks; narrow the query or add (sample N) (construction limits: {limit} blocks / {byte_limit} bytes)"),
        QueryError::ResultTooLarge { what: Budget::AdvancedQueryMatches, count, .. } =>
            format!("result-too-large: {count} advanced-query matches; narrow the query"),
        QueryError::ResultTooLarge { what: Budget::SearchHits, count, limit, bytes: Some(bytes), byte_limit } =>
            format!("result-too-large: {count} search hits (~{bytes} bytes); narrow the search (limits: {limit} hits / {byte_limit} bytes)"),
        QueryError::ResultTooLarge { what, count, limit, .. } =>
            format!("result-too-large: {count} {} (limit: {limit})", budget_text(what)),
    }
}

fn budget_text(what: Budget) -> &'static str {
    match what {
        Budget::BacklinkFilterRoots => "backlink filter roots",
        Budget::MatchingBlocks => "matching blocks",
        Budget::BridgeMatchingBlocks => "bridge matching blocks",
        Budget::RequestedBlockRefs => "requested block references",
        Budget::ResolvedBlockRows => "resolved block-reference rows",
        Budget::ExportBytes => "query export bytes",
        Budget::PropertyFacets => "property facets",
        Budget::AdvancedQueryMatches => "advanced-query matches",
        Budget::SearchHits => "search hits",
    }
}

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

#[cfg(test)]
fn enforce_result_bridge_budget(groups: &[RefGroup]) -> Result<(), String> {
    let rows = groups.iter().map(|group| group.blocks.len()).sum::<usize>();
    let bytes = tine_core::model::ref_groups_estimated_bytes(groups);
    if let Some(error) = QueryError::bridge_matching_blocks(rows, bytes) {
        return Err(query_error(error));
    }
    Ok(())
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
    use super::{enforce_result_bridge_budget, query_error};
    use tine_core::{BlockDto, PageKind, RefGroup};
    use tine_store::{Budget, QueryError};

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
        let groups = [group(vec![BlockDto::default(); 20_001])];
        assert!(enforce_result_bridge_budget(&groups)
            .unwrap_err()
            .starts_with("result-too-large:"));
    }

    #[test]
    fn rejects_oversized_result_bytes_before_ipc() {
        let mut block = BlockDto::default();
        block.raw = "x".repeat(33_554_433);
        assert!(enforce_result_bridge_budget(&[group(vec![block])])
            .unwrap_err()
            .starts_with("result-too-large:"));
    }

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

#[derive(Serialize)]
pub(crate) struct PageInventoryWire {
    /// The `GraphRev` the inventory was read at. The frontend drops a response
    /// older than one it already holds.
    rev: String,
    entries: Vec<PageInventoryEntryWire>,
}

#[derive(Serialize)]
pub(crate) struct PageInventoryEntryWire {
    /// `refs::page_key(name)`: the frontend looks names up by this key only.
    key: String,
    name: String,
    is_journal: bool,
    day: Option<i64>,
    target: ResolvedWire,
}

/// The whole name inventory: physical pages and journals, aliases, and names
/// that are only referenced. A thin adapter over `WholeGraph::inventory`; the
/// frontend caches it in `pageIndex.ts` and keeps no other name map.
///
/// Graph-wide, and it waits for the initial load: it runs on the blocking pool,
/// never on the main thread (the load wait alone held it ~240 ms at bind on the
/// anonymized graph).
#[tauri::command]
pub(crate) async fn page_inventory(state: GraphContext<'_>) -> Result<PageInventoryWire, String> {
    let slot = slot_for_context(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        slot.store
            .whole_graph()
            .map(|view| page_inventory_wire(&view))
            .map_err(|e| format!("graph load failed: {e:?}"))
    })
    .await
    .map_err(|error| error.to_string())?
}

fn page_inventory_wire(view: &WholeGraph) -> PageInventoryWire {
    let rev = view.rev();
    PageInventoryWire {
        rev: rev.into(),
        entries: view
            .inventory()
            .0
            .iter()
            .map(|entry| PageInventoryEntryWire {
                key: tine_core::refs::page_key(&entry.name),
                name: entry.name.clone(),
                is_journal: entry.is_journal,
                day: entry.day.map(|day| day.0),
                target: ResolvedWire::from(&entry.target),
            })
            .collect(),
    }
}

#[cfg(test)]
mod inventory_adapter_tests {
    use super::*;

    fn temp_root(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "tine-inventory-adapter-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn open(root: &std::path::Path, files: &[(&str, &str)]) -> tine_store::Store {
        for (path, body) in files {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, body).unwrap();
        }
        tine_store::Store::open(root, tine_store::OpenOptions::default())
            .unwrap()
            .0
    }

    fn rows(wire: &PageInventoryWire) -> Vec<String> {
        wire.entries
            .iter()
            .map(|entry| serde_json::to_string(entry).unwrap())
            .collect()
    }

    /// Ported from `inventory_adapters_match_legacy_fixture` (the deleted
    /// `list_pages` / `page_aliases` / `referenced_page_names` adapters): the
    /// same fixture, now asserted on the one wire that replaces all three. The
    /// frontend views over it are pinned in `src/pageIndex.test.ts`.
    #[test]
    fn page_inventory_wire_matches_legacy_fixture() {
        let root = temp_root("legacy");
        let store = open(
            &root,
            &[
                (
                    "pages/Alpha.md",
                    "title:: Display Alpha\nalias:: Shared\n- [[Only Linked]]\n",
                ),
                (
                    "pages/Beta.org",
                    "#+TITLE: Display Beta\nalias:: Shared\n* [[Only Linked]]\n",
                ),
                ("pages/nested/Alpha.org", "* nested twin\n"),
                ("pages/Team%2FChild.md", "- [[Another Ref]]\n"),
                ("journals/2026_06_26.md", "- canonical\n"),
                ("journals/Jun 26th, 2026.org", "* duplicate day\n"),
            ],
        );
        let view = store.whole_graph().unwrap();
        let wire = page_inventory_wire(&view);
        assert_eq!(wire.rev, String::from(view.rev()));
        assert_eq!(
            rows(&wire),
            vec![
                r#"{"key":"alpha","name":"Alpha","is_journal":false,"day":null,"target":{"kind":"existing","id":"pages/Alpha.md","others":["pages/nested/Alpha.org"]}}"#,
                r#"{"key":"another ref","name":"Another Ref","is_journal":false,"day":null,"target":{"kind":"absent","id":"pages/Another Ref.md"}}"#,
                r#"{"key":"beta","name":"Beta","is_journal":false,"day":null,"target":{"kind":"existing","id":"pages/Beta.org","others":[]}}"#,
                r#"{"key":"jun 26th, 2026","name":"Jun 26th, 2026","is_journal":true,"day":20260626,"target":{"kind":"existing","id":"journals/2026_06_26.md","others":["journals/Jun 26th, 2026.org"]}}"#,
                r#"{"key":"only linked","name":"Only Linked","is_journal":false,"day":null,"target":{"kind":"absent","id":"pages/Only Linked.md"}}"#,
                r#"{"key":"shared","name":"Shared","is_journal":false,"day":null,"target":{"kind":"alias","owners":["pages/Alpha.md","pages/Beta.org"]}}"#,
                r#"{"key":"team/child","name":"Team/Child","is_journal":false,"day":null,"target":{"kind":"existing","id":"pages/Team%2FChild.md","others":[]}}"#,
            ]
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Cost probe for batch-1 B16a, not a gate: `page_inventory` on a real
    /// graph. Run on a COPY: `TINE_INVENTORY_PROBE=<dir> cargo test --release
    /// -p tine page_inventory_cost_probe -- --ignored --nocapture`. Reports the
    /// JSON payload size and the median command time over 20 runs, both warm
    /// and after an ordinary content save (the refresh a save triggers).
    #[test]
    #[ignore]
    fn page_inventory_cost_probe() {
        let root = std::path::PathBuf::from(
            std::env::var("TINE_INVENTORY_PROBE").expect("TINE_INVENTORY_PROBE=<graph copy>"),
        );
        let (store, _, _) =
            tine_store::Store::open(&root, tine_store::OpenOptions::default()).unwrap();
        let time = |store: &tine_store::Store| {
            let start = std::time::Instant::now();
            let bytes =
                serde_json::to_vec(&page_inventory_wire(&store.whole_graph().unwrap())).unwrap();
            (start.elapsed(), bytes.len())
        };
        let median = |mut runs: Vec<std::time::Duration>| {
            runs.sort();
            runs[runs.len() / 2]
        };
        let (first, bytes) = time(&store);
        let wire = page_inventory_wire(&store.whole_graph().unwrap());
        let warm: Vec<_> = (0..20).map(|_| time(&store).0).collect();
        let target = wire
            .entries
            .iter()
            .find_map(|entry| match &entry.target {
                ResolvedWire::Existing { id, .. } if !entry.is_journal && id.ends_with(".md") => {
                    let id = PageId::from(id.clone());
                    let read = store.page(&id).ok()?;
                    (read.read_only.is_none() && !read.doc.blocks.is_empty()).then_some(id)
                }
                _ => None,
            })
            .expect("a Markdown page");
        let mut after_save = Vec::new();
        for i in 0..20 {
            let read = store.page(&target).unwrap();
            let mut doc = read.doc;
            doc.blocks[0].raw.push_str(&format!(" probe{i}"));
            let outcome = store.save(&target, SaveBase::Existing(read.rev), &doc);
            assert!(matches!(outcome, SaveOutcome::Saved(_)), "probe save");
            after_save.push(time(&store).0);
        }
        println!(
            "page_inventory: entries={} payload_bytes={bytes} first={first:?} warm_median={:?} after_save_median={:?} after_save_max={:?}",
            wire.entries.len(),
            median(warm),
            median(after_save.clone()),
            after_save.iter().max().unwrap(),
        );
    }

    /// Existing files beat a colliding alias (v0.6.5 `load_named`): the wire has
    /// one entry for the name, and its target is the file, as `resolve` says.
    #[test]
    fn page_inventory_a_file_beats_a_colliding_alias() {
        let root = temp_root("collide");
        let store = open(
            &root,
            &[
                ("pages/Owner.md", "alias:: Real, Cafe\u{301}\n- body\n"),
                ("pages/Real.md", "- a real page\n"),
                ("pages/Café.md", "- NFC file\n"),
            ],
        );
        let view = store.whole_graph().unwrap();
        let wire = page_inventory_wire(&view);
        for key in ["real", "café"] {
            let entries: Vec<_> = wire.entries.iter().filter(|e| e.key == key).collect();
            assert_eq!(entries.len(), 1, "one entry for {key}");
            let resolved = ResolvedWire::from(view.resolve(&entries[0].name, false));
            assert_eq!(
                serde_json::to_string(&entries[0].target).unwrap(),
                serde_json::to_string(&resolved).unwrap(),
            );
            assert!(matches!(entries[0].target, ResolvedWire::Existing { .. }));
        }
        std::fs::remove_dir_all(root).unwrap();
    }
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
        let feed = tine_graph_features::journals::feed_page(&slot.store, limit, before_day)?;
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
            .map(|view| view.resolve(&name, kind == PageKind::Journal).into())
            .map_err(|e| format!("graph load failed: {e:?}"))
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

#[tauri::command]
pub(crate) fn save_page(
    id: String,
    page: PageDto,
    base_rev: Option<String>,
    force: Option<bool>,
    state: GraphContext<'_>,
) -> Result<String, String> {
    let slot = slot_for_context(&state)?;
    let id = PageId::from(id);
    let outcome = tine_graph_features::pages::save_page(
        &slot.store,
        &id,
        &page,
        base_rev,
        force.unwrap_or(false),
    )
    .map_err(save_store_error)?;
    save_outcome_to_wire(outcome, &page)
}

fn save_outcome_to_wire(outcome: SaveOutcome, _page: &PageDto) -> Result<String, String> {
    match outcome {
        SaveOutcome::Saved(rev) | SaveOutcome::Unchanged(rev) => Ok(rev.into()),
        SaveOutcome::Conflict { .. } => Err("conflict".into()),
        SaveOutcome::Deleted => Err("deleted".into()),
        SaveOutcome::ReadOnly(_) => Err("read-only".into()),
        SaveOutcome::InvalidTarget(_) => Err("invalid-target".into()),
        SaveOutcome::Twin { .. } => Err("twin".into()),
        SaveOutcome::Io(error) => Err(format!("io:{:?}", error.kind())),
        SaveOutcome::Closed => Err("closed".into()),
        SaveOutcome::GuideEphemeral => Ok("guide-ephemeral".into()),
    }
}

#[cfg(test)]
mod save_wire_tests {
    use super::*;

    #[test]
    fn save_wire_families_are_distinct() {
        const RULE: &str = "I-9: wire failures use fixed families and omit page names/paths; exemplar commands::save_outcome_to_wire";
        let page: PageDto = serde_json::from_value(serde_json::json!({
            "name": "a conflict in my title", "kind": "page", "title": "a conflict in my title",
            "pre_block": null, "blocks": []
        }))
        .unwrap();
        assert_eq!(
            save_outcome_to_wire(
                SaveOutcome::Conflict {
                    disk: String::from("rev").into()
                },
                &page
            ),
            Err("conflict".into()),
            "{RULE}"
        );
        assert_eq!(
            save_outcome_to_wire(SaveOutcome::Deleted, &page),
            Err("deleted".into()),
            "{RULE}"
        );
        assert_eq!(
            save_outcome_to_wire(
                SaveOutcome::Twin {
                    existing: PageId::from("pages/secret.md".to_string())
                },
                &page
            ),
            Err("twin".into()),
            "{RULE}"
        );
        assert_eq!(
            save_outcome_to_wire(
                SaveOutcome::Io(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "/secret/path"
                )),
                &page
            ),
            Err("io:PermissionDenied".into()),
            "{RULE}"
        );
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
}

#[tauri::command]
pub(crate) fn guide_pages() -> Vec<tine_core::guide::GuidePage> {
    tine_core::guide::bundled_guide_pages()
}

#[tauri::command]
pub(crate) fn copy_guide_into_graph(
    title: String,
    state: GraphContext<'_>,
) -> Result<tine_graph_features::guide::GuideCopyResult, String> {
    let slot = slot_for_context(&state)?;
    tine_graph_features::guide::copy_guide_into_graph(&slot.store, &title)
        .map_err(|error| error.to_string())
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
        view.backlinks(&name).map_err(query_error)
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
        view.unlinked_references(&name).map_err(query_error)
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
pub(crate) fn delete_page(
    name: String,
    kind: PageKind,
    expected_path: Option<String>,
    state: GraphContext<'_>,
) -> Result<(), String> {
    let slot = slot_for_context(&state)?;
    tine_graph_features::pages::delete_page_expected(
        &slot.store,
        &name,
        kind,
        expected_path.as_deref(),
        None,
    )
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn rename_page(
    old: String,
    new: String,
    expected_path: Option<String>,
    state: GraphContext<'_>,
) -> Result<(), String> {
    let slot = slot_for_context(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        tine_graph_features::pages::rename_page_expected(
            &slot.store,
            &old,
            &new,
            expected_path.as_deref(),
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
        for name in ["get_backlinks", "get_unlinked_refs", "rename_page"] {
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
pub(crate) fn page_print_html(
    name: String,
    opts: tine_graph_features::print::PrintOpts,
    state: GraphContext<'_>,
) -> Result<String, String> {
    let slot = slot_for_context(&state)?;
    tine_graph_features::print::page_print_html(&slot.store, &name, opts)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "no-page".to_string())
}

#[tauri::command]
pub(crate) async fn run_query(
    query: String,
    state: GraphContext<'_>,
) -> Result<Arc<Vec<RefGroup>>, String> {
    let slot = slot_for_context(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        tine_graph_features::search::run_query(&slot.store, &query).map_err(feature_search_error)
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
        )
        .map_err(feature_search_error)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn run_advanced_query(
    query: String,
    current_page: Option<String>,
    state: GraphContext<'_>,
) -> Result<tine_core::query::AdvancedResult, String> {
    let slot = slot_for_context(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        tine_graph_features::search::run_advanced_query(
            &slot.store,
            &query,
            current_page.as_deref(),
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
pub(crate) fn set_favorites(names: Vec<String>, state: GraphContext<'_>) -> Result<(), String> {
    with_config_store(&state, |store| {
        tine_graph_features::config::set_favorites(store, &names).map_err(|e| e.to_string())
    })
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

/// Set the graph's `:journal/page-title-format` (journal display-title format,
/// e.g. "MMM do, yyyy"). Display-only — does not rename journal files.
#[tauri::command]
pub(crate) fn set_journal_title_format(
    format: String,
    state: GraphContext<'_>,
) -> Result<(), String> {
    let slot = slot_for_context(&state)?;
    tine_graph_features::config::set_journal_page_title_format_and_migrate(&slot.store, &format)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) fn read_custom_css(state: GraphContext<'_>) -> Result<String, String> {
    with_config_store(&state, |store| {
        Ok(tine_graph_features::config::custom_css(store))
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
            capture_graph: Mutex::new(None),
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
pub(crate) fn list_templates(
    state: GraphContext<'_>,
) -> Result<Vec<tine_core::model::TemplateDto>, String> {
    Ok(whole_graph(&state)?.templates())
}

#[tauri::command]
pub(crate) fn journal_content_days(state: GraphContext<'_>) -> Result<Vec<i64>, String> {
    Ok(whole_graph(&state)?
        .journal_content_days()
        .into_iter()
        .map(|day| day.0)
        .collect())
}

#[tauri::command]
pub(crate) fn resolve_block(
    uuid: String,
    state: GraphContext<'_>,
) -> Result<Option<RefGroup>, String> {
    Ok(whole_graph(&state)?
        .blocks(&[uuid])
        .map_err(query_error)?
        .pop()
        .flatten())
}

#[tauri::command]
pub(crate) fn resolve_blocks(
    uuids: Vec<String>,
    state: GraphContext<'_>,
) -> Result<Vec<Option<RefGroup>>, String> {
    whole_graph(&state)?.blocks(&uuids).map_err(query_error)
}

/// Explicit, bounded subtree resolution for hover previews. Ordinary
/// `resolve_block(s)` stays shallow so a page containing nested references
/// cannot multiply the same descendants across the IPC bridge.
#[tauri::command]
pub(crate) fn preview_block(
    uuid: String,
    max_nodes: usize,
    state: GraphContext<'_>,
) -> Result<Option<tine_core::BlockPreview>, String> {
    whole_graph(&state)?
        .preview_block(&uuid, max_nodes)
        .map_err(query_error)
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
    let p = refuse_bound_graph_path(p, &state)?;
    let meta = std::fs::metadata(&p).map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err("not a file".into());
    }
    const MAX_BYTES: u64 = 64 * 1024 * 1024;
    if meta.len() > MAX_BYTES {
        return Err("image too large".into());
    }
    std::fs::read(&p)
        .map(tauri::ipc::Response::new)
        .map_err(|e| e.to_string())
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
    let slot = slot_for_context(&state)?;
    let stored = tine_graph_features::assets::import_asset_file(
        &slot.store,
        &name,
        tine_store::Content::Stream {
            source: capture.into_std(),
            max_bytes,
        },
    )
    .map_err(|error| feature_asset_error(error, &slot))?;
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
    let slot = slot_for_context(&state)?;
    let target = asset_handoff_target(&slot, &name)?;
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
pub(crate) fn open_page_file(
    name: String,
    kind: PageKind,
    path: Option<String>,
    reveal: bool,
    state: GraphContext<'_>,
) -> Result<(), String> {
    let slot = slot_for_context(&state)?;
    let target = tine_graph_features::pages::source_path_for_os_handoff(
        &slot.store,
        &name,
        kind,
        path.as_deref(),
    )
    .map_err(feature_page_read_error)?;
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
    let slot = slot_for_context(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        tine_graph_features::assets::orphan_assets(&slot.store)
    })
    .await
    .map_err(|error| error.to_string())
}

/// Move an orphaned asset to the recoverable trash.
#[tauri::command]
pub(crate) fn trash_asset(name: String, state: GraphContext<'_>) -> Result<(), String> {
    let slot = slot_for_context(&state)?;
    tine_graph_features::assets::trash_asset(&slot.store, &name)
        .map_err(|error| feature_asset_error(error, &slot))
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
    })
    .await
    .map_err(|error| error.to_string())
}

/// Sync-tool conflict copies (Syncthing/Dropbox) sitting in the graph — for the
/// user to review + reconcile instead of them showing as garbage pages.
#[tauri::command]
pub(crate) async fn list_sync_conflicts(
    state: GraphContext<'_>,
) -> Result<Vec<tine_core::model::SyncConflict>, String> {
    let slot = slot_for_context(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        tine_graph_features::conflicts::list_sync_conflicts(&slot.store)
    })
    .await
    .map_err(|error| error.to_string())
}

/// Block-level diff of a sync-conflict copy against its winner (both graph-root-
/// relative paths) — the data behind the two-column merge UI. Read-only.
#[tauri::command]
pub(crate) fn sync_conflict_diff(
    winner: String,
    conflict: String,
    state: GraphContext<'_>,
) -> Result<Option<tine_core::sync_diff::SyncConflictDiff>, String> {
    let slot = slot_for_context(&state)?;
    tine_graph_features::conflicts::sync_conflict_diff(&slot.store, &winner, &conflict)
        .map_err(|e| e.to_string())
}

/// Resolve a sync-conflict copy: merge it into its winner per the user's per-row
/// `decisions` (row id → "mine"/"theirs"/"both") via the normal save path, then
/// trash the conflict copy. `base_rev` guards against the winner changing under
/// the merge; returns "conflict" if it did. `pre_choice`: "mine"/"theirs"/"union".
#[tauri::command]
pub(crate) fn resolve_sync_conflict(
    winner: String,
    conflict: String,
    decisions: std::collections::HashMap<String, String>,
    base_rev: String,
    conflict_rev: String,
    pre_choice: Option<String>,
    state: GraphContext<'_>,
) -> Result<(), String> {
    let slot = slot_for_context(&state)?;
    tine_graph_features::conflicts::resolve_sync_conflict(
        &slot.store,
        &winner,
        &conflict,
        &decisions,
        &base_rev,
        &conflict_rev,
        pre_choice.as_deref().unwrap_or("union"),
    )
    .map_err(sync_conflict_error)
}

/// Discard a sync-conflict copy without merging (move it to the recoverable
/// trash). Refuses anything that isn't a conflict copy.
#[tauri::command]
pub(crate) fn trash_sync_conflict(conflict: String, state: GraphContext<'_>) -> Result<(), String> {
    let slot = slot_for_context(&state)?;
    tine_graph_features::conflicts::trash_sync_conflict(&slot.store, &conflict)
        .map_err(|e| e.to_string())
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
pub(crate) fn merge_pages(src: String, dst: String, state: GraphContext<'_>) -> Result<(), String> {
    let slot = slot_for_context(&state)?;
    tine_graph_features::pages::merge_pages(&slot.store, &src, &dst)
        .map_err(graph_write_error_to_wire)
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
pub(crate) fn rename_file_to_page(
    path: String,
    new_name: String,
    state: GraphContext<'_>,
) -> Result<(), String> {
    let slot = slot_for_context(&state)?;
    tine_graph_features::pages::rename_file_to_page(&slot.store, &path, &new_name)
        .map_err(|error| error.to_string())
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

#[tauri::command]
pub(crate) fn read_highlights(
    pdf: String,
    state: GraphContext<'_>,
) -> Result<Vec<tine_core::pdf::Highlight>, String> {
    let slot = slot_for_context(&state)?;
    Ok(tine_graph_features::pdf::read_highlights(&slot.store, &pdf))
}

#[tauri::command]
pub(crate) fn open_pdf(
    pdf: String,
    label: String,
    state: GraphContext<'_>,
) -> Result<tine_core::pdf::PdfState, String> {
    let slot = slot_for_context(&state)?;
    tine_graph_features::pdf::open_pdf(&slot.store, &pdf, &label).map_err(feature_pdf_error)
}

#[tauri::command]
pub(crate) fn write_highlights(
    pdf: String,
    label: String,
    highlights: Vec<tine_core::pdf::Highlight>,
    base_ids: Vec<String>,
    state: GraphContext<'_>,
) -> Result<(), String> {
    let slot = slot_for_context(&state)?;
    tine_graph_features::pdf::write_highlights(&slot.store, &pdf, &label, &highlights, &base_ids)
        .map_err(feature_pdf_error)
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
