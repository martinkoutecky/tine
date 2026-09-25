#[cfg(desktop)]
use crate::debug::diag;
#[cfg(desktop)]
use crate::platform::{open_page_source, opener_command, reveal_page_source};
use crate::state::{
    capture_quick_switch_slot, refresh_graph, slot_for_context, with_graph, AppState, GraphContext,
    GraphSlot,
};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{State, WebviewWindow};
use tine_core::date::JournalDate;
use tine_core::model::{
    BacklinkFilterContext, BacklinkFilterTarget, PageDto, PageEntry, PageKind, RefGroup,
};
use tine_store::{
    Area, Budget, Cancel, FacetPolicy, Inventory, PageId, QueryDialect, QueryError, QueryResult,
    Resolved, SaveBase, SaveOutcome, SearchRequest, StoreError, TrashKind, WholeGraph,
};

fn page_dto(read: tine_store::PageRead) -> PageDto {
    let mut doc = read.doc;
    doc.rev = Some(read.rev.into());
    doc.read_only = read.read_only.is_some();
    doc.path = Some(read.id);
    doc
}

fn store_error(error: StoreError) -> String {
    match error {
        StoreError::NotFound => std::io::ErrorKind::NotFound.to_string(),
        StoreError::InvalidTarget(_) => "invalid page path".into(),
        StoreError::Undecodable => "stream did not contain valid UTF-8".into(),
        StoreError::Unparseable(reason) => reason,
        StoreError::TooLarge { limit, .. } => format!("asset exceeds {limit} byte limit"),
        StoreError::Io(error) => error.to_string(),
        StoreError::Closed => "store closed".into(),
    }
}

fn asset_error(error: StoreError) -> String {
    match error {
        StoreError::NotFound => std::io::Error::from_raw_os_error(2).to_string(),
        StoreError::InvalidTarget(_) => "invalid asset".into(),
        StoreError::TooLarge { limit, .. } => format!("asset exceeds {limit} byte limit"),
        other => store_error(other),
    }
}

fn stream_asset_error(error: StoreError) -> String {
    match error {
        StoreError::InvalidTarget(reason) if reason.starts_with("symlink:") => {
            "asset symlinks cannot be streamed".into()
        }
        other => asset_error(other),
    }
}

fn asset_handoff_target(slot: &GraphSlot, name: &str) -> Result<std::path::PathBuf, String> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.contains('\\') {
        return Err("bad asset name".into());
    }
    let id = slot
        .store
        .file_id(Area::Assets, name)
        .map_err(asset_error)?;
    let target = slot.store.path_for_os_handoff(&id).map_err(asset_error)?;
    let target = std::fs::canonicalize(target).map_err(|error| error.to_string())?;
    let assets =
        std::fs::canonicalize(slot.graph.assets_path()).map_err(|error| error.to_string())?;
    if !target.starts_with(&assets) || !target.is_file() {
        return Err("invalid asset".into());
    }
    Ok(target)
}

fn page_handoff_target(slot: &GraphSlot, id: &PageId) -> Result<std::path::PathBuf, String> {
    if slot.store.as_page(&id.file()).is_none() {
        return Err("invalid page path".into());
    }
    let lexical = slot.graph.root.join(id.as_str());
    let target = match slot.store.path_for_os_handoff(&id.file()) {
        Ok(target) => target,
        // The old page opener admitted symlinks between pages/ and journals/
        // and reported its own escape error for symlinks outside both.
        Err(StoreError::InvalidTarget(_)) => lexical,
        Err(error) => return Err(store_error(error)),
    };
    let target = std::fs::canonicalize(target).map_err(|error| error.to_string())?;
    if !target.is_file() {
        return Err("page source is not a file".into());
    }
    let pages =
        std::fs::canonicalize(slot.graph.pages_path()).map_err(|error| error.to_string())?;
    let journals =
        std::fs::canonicalize(slot.graph.journals_path()).map_err(|error| error.to_string())?;
    if !target.starts_with(&pages) && !target.starts_with(&journals) {
        return Err("page source escapes graph directories".into());
    }
    Ok(target)
}

#[cfg(test)]
mod handoff_tests {
    use super::*;
    use tine_store::model::Graph;

    #[test]
    fn open_targets_require_existing_regular_files() {
        let root = std::env::temp_dir().join(format!(
            "tine-handoff-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        for area in ["pages", "journals", "assets"] {
            std::fs::create_dir_all(root.join(area)).unwrap();
        }
        std::fs::write(root.join("pages/Good.md"), "- good\n").unwrap();
        std::fs::write(root.join("assets/good.bin"), b"good").unwrap();
        std::fs::create_dir(root.join("pages/Directory.md")).unwrap();
        std::fs::create_dir(root.join("assets/directory.bin")).unwrap();
        let slot = GraphSlot::new(Graph::open(&root), root.clone());

        assert_eq!(
            page_handoff_target(&slot, &PageId::from("pages/Good.md")).unwrap(),
            root.join("pages/Good.md")
        );
        assert_eq!(
            asset_handoff_target(&slot, "good.bin").unwrap(),
            root.join("assets/good.bin")
        );
        assert!(page_handoff_target(&slot, &PageId::from("pages/Missing.md")).is_err());
        assert!(asset_handoff_target(&slot, "missing.bin").is_err());
        assert_eq!(
            page_handoff_target(&slot, &PageId::from("pages/Directory.md")).unwrap_err(),
            "page source is not a file"
        );
        assert_eq!(
            asset_handoff_target(&slot, "directory.bin").unwrap_err(),
            "invalid asset"
        );

        #[cfg(unix)]
        {
            let outside = root.with_extension("outside.md");
            std::fs::write(&outside, "- outside\n").unwrap();
            std::os::unix::fs::symlink(&outside, root.join("pages/Escape.md")).unwrap();
            std::os::unix::fs::symlink(&outside, root.join("assets/escape.bin")).unwrap();
            assert_eq!(
                page_handoff_target(&slot, &PageId::from("pages/Escape.md")).unwrap_err(),
                "page source escapes graph directories"
            );
            assert_eq!(
                asset_handoff_target(&slot, "escape.bin").unwrap_err(),
                "invalid asset"
            );
            std::fs::remove_file(outside).unwrap();
        }
        std::fs::remove_dir_all(root).unwrap();
    }
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

#[cfg(test)]
fn enforce_result_bridge_budget(groups: &[RefGroup]) -> Result<(), String> {
    let rows = groups.iter().map(|group| group.blocks.len()).sum::<usize>();
    let bytes = tine_core::model::ref_groups_estimated_bytes(groups);
    if let Some(error) = QueryError::bridge_matching_blocks(rows, bytes) {
        return Err(query_error(error));
    }
    Ok(())
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
                    + page.rel_path_str().len()
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
    if let Some(error) = QueryError::bridge_search_hits(execution.hits.len(), bytes) {
        return Err(query_error(error));
    }
    Ok(())
}

#[cfg(test)]
mod result_bridge_budget_tests {
    use super::{enforce_result_bridge_budget, query_error, validate_query_source};
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

#[tauri::command]
pub(crate) fn list_pages(state: GraphContext<'_>) -> Result<Vec<PageEntry>, String> {
    Ok(list_pages_from_inventory(&whole_graph(&state)?.inventory()))
}

#[tauri::command]
pub(crate) fn referenced_page_names(state: GraphContext<'_>) -> Result<Vec<String>, String> {
    Ok(referenced_names_from_inventory(
        &whole_graph(&state)?.inventory(),
    ))
}

fn list_pages_from_inventory(inventory: &Inventory) -> Vec<PageEntry> {
    let mut rows = Vec::new();
    for entry in &inventory.0 {
        if let Resolved::Existing { id, others } = &entry.target {
            let ids: Vec<&PageId> = if entry.is_journal {
                vec![id]
            } else {
                std::iter::once(id).chain(others).collect()
            };
            for id in ids {
                rows.push(PageEntry {
                    name: entry.name.clone(),
                    kind: if entry.is_journal {
                        PageKind::Journal
                    } else {
                        PageKind::Page
                    },
                    date_key: entry.day.map(|day| day.0),
                    rel_path: Some(id.clone()),
                    path: std::path::PathBuf::new(), // skipped by PageEntry's IPC serializer
                });
            }
        }
    }
    rows
}

fn referenced_names_from_inventory(inventory: &Inventory) -> Vec<String> {
    // pages.ts adds physical names first and drops case-folded collisions.
    // Alias property values counted as references in v0.6.5, so include aliases.
    // page_key also applies NFC, trim, and one boundary slash, so names at
    // those edges can differ from the previous frontend-visible union.
    inventory
        .0
        .iter()
        .filter_map(|entry| match &entry.target {
            Resolved::Absent { .. } | Resolved::Alias { .. } => Some(entry.name.clone()),
            _ => None,
        })
        .collect()
}

fn aliases_from_inventory(inventory: &Inventory) -> Vec<(String, String)> {
    let names: std::collections::HashMap<&str, &str> = inventory
        .0
        .iter()
        .filter_map(|entry| {
            if let Resolved::Existing { id, others } = &entry.target {
                Some(
                    std::iter::once(id)
                        .chain(others)
                        .map(|id| (id.as_str(), entry.name.as_str())),
                )
            } else {
                None
            }
        })
        .flatten()
        .collect();
    let mut rows = Vec::new();
    for entry in &inventory.0 {
        if let Resolved::Alias { owners } = &entry.target {
            for owner in owners {
                if let Some(name) = names.get(owner.as_str()) {
                    rows.push((owner.as_str(), entry.name.as_str(), *name));
                }
            }
        }
    }
    rows.sort_by(|a, b| {
        a.0.cmp(b.0)
            .then_with(|| tine_core::refs::page_key(a.1).cmp(&tine_core::refs::page_key(b.1)))
    });
    rows.into_iter()
        .map(|(_, alias, owner)| (tine_core::refs::page_key(alias), owner.to_owned()))
        .collect()
}

#[cfg(test)]
mod inventory_adapter_tests {
    use super::*;
    use tine_store::model::Graph;

    fn sorted_pages(pages: Vec<PageEntry>) -> Vec<String> {
        let mut rows: Vec<_> = pages
            .into_iter()
            .map(|page| serde_json::to_string(&page).unwrap())
            .collect();
        rows.sort();
        rows
    }

    fn visible_names(pages: Vec<PageEntry>, references: Vec<String>) -> Vec<String> {
        let mut seen = std::collections::HashSet::new();
        let mut names = Vec::new();
        for name in pages.into_iter().map(|page| page.name).chain(references) {
            if seen.insert(name.to_lowercase()) {
                names.push(name);
            }
        }
        names.sort();
        names
    }

    #[test]
    fn inventory_adapters_match_legacy_fixture() {
        let root = std::env::temp_dir().join(format!(
            "tine-inventory-adapter-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        for dir in ["pages/nested", "journals"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        for (path, body) in [
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
        ] {
            std::fs::write(root.join(path), body).unwrap();
        }
        let graph = Arc::new(Graph::open(&root));
        graph.warm_cache();
        let view = tine_store::Store::from_legacy(Arc::clone(&graph))
            .whole_graph()
            .unwrap();
        let inventory = view.inventory();

        let old_pages = graph.list_pages();
        let new_pages = list_pages_from_inventory(&inventory);
        let old_multiset = sorted_pages(old_pages.clone());
        let new_multiset = sorted_pages(new_pages.clone());
        assert_eq!(old_multiset, new_multiset);
        assert_eq!(graph.page_aliases(), aliases_from_inventory(&inventory));
        assert_eq!(
            visible_names(old_pages, graph.referenced_page_names()),
            visible_names(new_pages, referenced_names_from_inventory(&inventory))
        );
        std::fs::remove_dir_all(root).unwrap();
    }
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

/// Feed-only pagination. `before_day` is an ordinal-day cursor rather than a
/// mutable vector offset, so a file disappearing after inventory cannot make a
/// later day duplicate or disappear from the next request.
#[tauri::command]
pub(crate) fn journal_feed_page(
    limit: usize,
    before_day: Option<i64>,
    state: GraphContext<'_>,
) -> Result<JournalFeedPage, String> {
    let slot = slot_for_context(&state)?;
    let g = &slot.graph;
    {
        let as_of_day = JournalDate::today().ordinal_key();
        let entries = g.feed_journals_desc_through(JournalDate::from_ordinal(as_of_day));
        collect_journal_feed_page(entries, limit, before_day, as_of_day, |entry| {
            // A journal deleted from disk between inventory and load is skipped,
            // but its day still advances the cursor in the helper above.
            let id = entry.rel_path.as_ref().ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::InvalidInput, "missing journal path")
            })?;
            slot.store
                .page(id)
                .map(page_dto)
                .map_err(|error| match error {
                    StoreError::NotFound => std::io::Error::from(std::io::ErrorKind::NotFound),
                    StoreError::Io(error) => error,
                    other => std::io::Error::other(store_error(other)),
                })
        })
    }
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
            rel_path: None,
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
pub(crate) fn get_page(
    name: String,
    kind: PageKind,
    state: GraphContext<'_>,
) -> Result<Option<PageDto>, String> {
    let slot = slot_for_context(&state)?;
    let resolved = slot
        .store
        .whole_graph()
        .map_err(|e| format!("{e:?}"))?
        .resolve(&name, kind == PageKind::Journal);
    let id = match resolved {
        Resolved::Existing { id, .. } => id,
        Resolved::Alias { owners } => owners.into_iter().next().ok_or("alias has no owner")?,
        Resolved::Absent { .. } => return Ok(None),
    };
    match slot.store.page(&id) {
        Ok(read) => Ok(Some(page_dto(read))),
        Err(StoreError::NotFound) => Ok(None),
        Err(error) => Err(store_error(error)),
    }
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
    with_graph(&state, |g| {
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
    g: &tine_store::model::Graph,
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

#[tauri::command]
pub(crate) fn save_page(
    page: PageDto,
    base_rev: Option<String>,
    force: Option<bool>,
    state: GraphContext<'_>,
) -> Result<String, String> {
    let slot = slot_for_context(&state)?;
    if page.guide {
        return save_outcome_to_wire(SaveOutcome::GuideEphemeral, &page);
    }
    let id = match slot.store.target_for_save(&page) {
        Ok(id) => id,
        Err(outcome) => return save_outcome_to_wire(outcome, &page),
    };
    let base = if force.unwrap_or(false) {
        // Interim keep-mine: the banner shows no disk version yet. Read the
        // current one, then let BOTH save guards reject any later external edit.
        // Never turn unreadable or undecodable bytes into overwrite permission.
        match slot.store.read(&id.file(), None) {
            Ok((bytes, rev)) => {
                std::str::from_utf8(&bytes)
                    .map_err(|_| "stream did not contain valid UTF-8".to_string())?;
                SaveBase::Existing(rev)
            }
            Err(StoreError::NotFound) => SaveBase::CreateNew,
            Err(error) => return Err(store_error(error)),
        }
    } else {
        base_rev
            .map(|rev| SaveBase::Existing(rev.into()))
            .unwrap_or(SaveBase::CreateNew)
    };
    save_outcome_to_wire(slot.store.save(&id, base, &page), &page)
}

fn save_outcome_to_wire(outcome: SaveOutcome, page: &PageDto) -> Result<String, String> {
    match outcome {
        SaveOutcome::Saved(rev) | SaveOutcome::Unchanged(rev) => Ok(rev.into()),
        SaveOutcome::Conflict { .. } | SaveOutcome::Deleted => Err("conflict".into()),
        SaveOutcome::ReadOnly(reason) | SaveOutcome::InvalidTarget(reason) => Err(reason),
        SaveOutcome::Twin { .. } => Err(format!(
            "\"{}\" exists as both a .md and a .org file — remove one (e.g. in Logseq) to edit it in Tine",
            page.name
        )),
        SaveOutcome::Io(error) => Err(error.to_string()),
        SaveOutcome::Closed => Err("store closed".into()),
        SaveOutcome::GuideEphemeral => Ok("guide-ephemeral".into()),
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
) -> Result<tine_store::onboarding::GuideCopyResult, String> {
    let result: Result<tine_store::onboarding::GuideCopyResult, String> = with_graph(&state, |g| {
        tine_store::onboarding::copy_guide_into_graph(g, &title).map_err(|e| e.to_string())
    });
    result
}

#[tauri::command]
pub(crate) async fn get_backlinks(
    name: String,
    state: GraphContext<'_>,
) -> Result<Arc<Vec<RefGroup>>, String> {
    let view = whole_graph(&state)?;
    tauri::async_runtime::spawn_blocking(move || view.backlinks(&name).map_err(query_error))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn get_backlink_filter_context(
    name: String,
    targets: Vec<BacklinkFilterTarget>,
    state: GraphContext<'_>,
) -> Result<BacklinkFilterContext, String> {
    let view = whole_graph(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
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
    let view = whole_graph(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
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
    let view = whole_graph(&state)?;
    tauri::async_runtime::spawn_blocking(move || view.block_ref_counts())
        .await
        .map_err(|error| error.to_string())
}

/// The blocks that reference block `uuid`, grouped by page (the badge's referrers
/// panel). Lazy: called only when a badge is clicked open.
#[tauri::command]
pub(crate) fn block_referrers(
    uuid: String,
    state: GraphContext<'_>,
) -> Result<Arc<Vec<RefGroup>>, String> {
    whole_graph(&state)?
        .block_referrers(&uuid)
        .map_err(query_error)
}

#[tauri::command]
pub(crate) fn delete_page(
    name: String,
    kind: PageKind,
    expected_path: Option<String>,
    state: GraphContext<'_>,
) -> Result<(), String> {
    with_graph(&state, |g| {
        g.delete_page_expected(&name, kind, expected_path.as_deref())
            .map_err(|e| e.to_string())
    })
}

#[tauri::command]
pub(crate) async fn rename_page(
    old: String,
    new: String,
    expected_path: Option<String>,
    state: GraphContext<'_>,
) -> Result<(), String> {
    let graph = Arc::clone(&slot_for_context(&state)?.graph);
    tauri::async_runtime::spawn_blocking(move || {
        graph
            .rename_page_expected(&old, &new, expected_path.as_deref())
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
pub(crate) fn publish_html(state: GraphContext<'_>) -> Result<(String, usize), String> {
    with_graph(&state, |g| g.publish_html().map_err(|e| e.to_string()))
}

/// Render one page to a self-contained HTML document (assets inlined, no sidebar)
/// for the print-to-PDF export, with the dialog's options. `Err("no-page")` if the
/// page doesn't exist.
#[tauri::command]
pub(crate) fn page_print_html(
    name: String,
    opts: tine_store::publish::PrintOpts,
    state: GraphContext<'_>,
) -> Result<String, String> {
    with_graph(&state, |g| {
        g.page_print_html(&name, opts)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "no-page".to_string())
    })
}

#[tauri::command]
pub(crate) fn run_query(
    query: String,
    state: GraphContext<'_>,
) -> Result<Arc<Vec<RefGroup>>, String> {
    validate_query_source(&query)?;
    match whole_graph(&state)?
        .query(&query, QueryDialect::Simple, None)
        .map_err(query_error)?
    {
        QueryResult::Simple(groups) => Ok(groups),
        QueryResult::Advanced(_) => unreachable!(),
    }
}

/// Resolve every query macro in one Copy / Export session under one cumulative
/// construction budget. Unlike `get_page`, this returns only selected subtrees;
/// unrelated page content is never cloned across IPC or retained by the WebView.
#[tauri::command]
pub(crate) fn export_query_subtrees(
    specs: Vec<tine_core::query::QueryExportSpec>,
    state: GraphContext<'_>,
) -> Result<tine_core::query::QueryExportBatch, String> {
    whole_graph(&state)?
        .export_query_subtrees(&specs)
        .map_err(query_error)
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
    let view = slot
        .store
        .whole_graph()
        .map_err(|e| format!("graph load failed: {e:?}"))?;
    let flag = Arc::new(AtomicBool::new(false));
    if let Some(lane) = lane.as_ref() {
        if let Some(previous) = slot
            .block_search_lanes
            .lock()
            .unwrap()
            .insert(lane.clone(), Arc::clone(&flag))
        {
            previous.store(true, Ordering::Release);
        }
    }
    let within = scope.map(|scope| match scope.path {
        Some(path) => tine_store::PageId::from(path),
        None => match view.resolve(&scope.name, scope.page_kind == PageKind::Journal) {
            Resolved::Existing { id, .. } | Resolved::Absent { id } => id,
            Resolved::Alias { owners } => owners.into_iter().next().expect("alias has an owner"),
        },
    });
    let request = SearchRequest {
        text: source,
        within,
        page_limit,
        block_limit,
        explain,
    };
    // QueryExecution carries backward-defaulted per-category `has_more` bits;
    // returning it directly preserves those bits on the Tauri wire.
    let execution =
        tauri::async_runtime::spawn_blocking(move || view.search(&request, &Cancel(flag)))
            .await
            .map_err(|e| e.to_string())?;
    let execution = match execution {
        Ok(execution) => execution,
        Err(QueryError::Cancelled) => tine_core::query_plan::QueryExecution {
            hits: Vec::new(),
            diagnostics: Vec::new(),
            explanation: tine_core::query_plan::QueryExplanation {
                branches: Vec::new(),
            },
            has_more: tine_core::query_plan::QueryHasMore::default(),
            cancelled: true,
        },
        Err(error) => return Err(query_error(error)),
    };
    enforce_query_execution_budget(&execution)?;
    Ok(execution)
}

#[tauri::command]
pub(crate) fn run_advanced_query(
    query: String,
    current_page: Option<String>,
    state: GraphContext<'_>,
) -> Result<tine_core::query::AdvancedResult, String> {
    validate_query_source(&query)?;
    let view = whole_graph(&state)?;
    let current_id = current_page
        .as_deref()
        .map(|name| match view.resolve(name, false) {
            Resolved::Existing { id, .. } | Resolved::Absent { id } => id,
            Resolved::Alias { owners } => owners.into_iter().next().expect("alias has an owner"),
        });
    match view
        .query(&query, QueryDialect::Advanced, current_id.as_ref())
        .map_err(query_error)?
    {
        QueryResult::Advanced(result) => Ok(result),
        QueryResult::Simple(_) => unreachable!(),
    }
}

#[tauri::command]
pub(crate) fn query_facets(
    state: GraphContext<'_>,
    autocomplete: Option<bool>,
) -> Result<Vec<(String, Vec<String>)>, String> {
    let policy = if autocomplete.unwrap_or(false) {
        FacetPolicy::Truncated
    } else {
        FacetPolicy::Budgeted
    };
    whole_graph(&state)?
        .property_facets(policy)
        .map_err(query_error)
}

#[tauri::command]
pub(crate) fn page_aliases(state: GraphContext<'_>) -> Result<Vec<(String, String)>, String> {
    Ok(aliases_from_inventory(&whole_graph(&state)?.inventory()))
}

#[tauri::command]
pub(crate) fn page_icons(
    names: Vec<String>,
    state: GraphContext<'_>,
) -> Result<std::collections::HashMap<String, String>, String> {
    Ok(whole_graph(&state)?.page_icons(&names))
}

#[tauri::command]
pub(crate) fn set_favorites(names: Vec<String>, state: GraphContext<'_>) -> Result<(), String> {
    with_graph(&state, |g| {
        g.set_favorites(&names).map_err(|e| e.to_string())
    })
}

#[tauri::command]
pub(crate) fn set_preferred_workflow(
    workflow: String,
    state: GraphContext<'_>,
) -> Result<(), String> {
    with_graph(&state, |g| {
        g.set_preferred_workflow(&workflow)
            .map_err(|e| e.to_string())
    })
}

#[tauri::command]
pub(crate) fn set_timetracking_enabled(
    enabled: bool,
    state: GraphContext<'_>,
) -> Result<(), String> {
    with_graph(&state, |g| {
        g.set_timetracking_enabled(enabled)
            .map_err(|e| e.to_string())
    })?;
    refresh_graph(&state)?;
    Ok(())
}

#[tauri::command]
pub(crate) fn set_show_brackets(enabled: bool, state: GraphContext<'_>) -> Result<(), String> {
    with_graph(&state, |g| {
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
    with_graph(&state, |g| {
        g.set_doc_mode_enter_for_new_block(enabled)
            .map_err(|e| e.to_string())
    })?;
    refresh_graph(&state)?;
    Ok(())
}

#[tauri::command]
pub(crate) fn set_logical_outdenting(enabled: bool, state: GraphContext<'_>) -> Result<(), String> {
    with_graph(&state, |g| {
        g.set_logical_outdenting(enabled).map_err(|e| e.to_string())
    })?;
    refresh_graph(&state)?;
    Ok(())
}

#[tauri::command]
pub(crate) fn set_guide_announced(announced: bool, state: GraphContext<'_>) -> Result<(), String> {
    with_graph(&state, |g| {
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
    with_graph(&state, |g| {
        g.set_default_journal_template(name.as_deref())
            .map_err(|e| e.to_string())
    })
}

#[tauri::command]
pub(crate) fn set_start_of_week(n: u32, state: GraphContext<'_>) -> Result<(), String> {
    with_graph(&state, |g| {
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
    with_graph(&state, |g| {
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
    with_graph(&state, |g| {
        g.set_journal_page_title_format(&format)
            .map_err(|e| e.to_string())
    })?;
    refresh_graph(&state)?; // pick up the new format + migrate any title-named journals
    Ok(())
}

#[tauri::command]
pub(crate) fn read_custom_css(state: GraphContext<'_>) -> Result<String, String> {
    with_graph(&state, |g| Ok(g.custom_css()))
}

#[tauri::command]
pub(crate) async fn search(
    query: String,
    limit: usize,
    lane: Option<String>,
    state: GraphContext<'_>,
) -> Result<Vec<RefGroup>, String> {
    let slot = slot_for_context(&state)?;
    let view = slot
        .store
        .whole_graph()
        .map_err(|e| format!("graph load failed: {e:?}"))?;
    let flag = Arc::new(AtomicBool::new(false));
    if let Some(lane) = lane {
        if let Some(previous) = slot
            .block_search_lanes
            .lock()
            .unwrap()
            .insert(lane, Arc::clone(&flag))
        {
            previous.store(true, Ordering::Release);
        }
    }
    let groups = tauri::async_runtime::spawn_blocking(move || {
        view.find_blocks(&query, limit, &Cancel(flag))
            .map_err(query_error)
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(groups)
}

#[tauri::command]
pub(crate) fn quick_switch(
    query: String,
    limit: usize,
    state: GraphContext<'_>,
) -> Result<Vec<PageEntry>, String> {
    Ok(whole_graph(&state)?.complete_page_names(&query, limit))
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
    use tine_store::model::Graph;

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
    if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.contains('\\') {
        return Err("bad asset name".into());
    }
    let slot = slot_for_context(&state)?;
    let file = slot
        .store
        .file_id(Area::Assets, &name)
        .map_err(asset_error)?;
    slot.store
        .read(&file, max_bytes)
        .map(|(bytes, _)| tauri::ipc::Response::new(bytes))
        .map_err(asset_error)
}

/// Validate one graph media file and return its top-level asset name for the
/// range-aware `tine-media:` protocol. The protocol revalidates against the
/// requesting window's current graph on every request.
#[tauri::command]
pub(crate) fn stream_asset_path(name: String, state: GraphContext<'_>) -> Result<String, String> {
    let slot = slot_for_context(&state)?;
    if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.contains('\\') {
        return Err("bad asset name".into());
    }
    let id = slot
        .store
        .file_id(Area::Assets, &name)
        .map_err(asset_error)?;
    slot.store.open_read(&id).map_err(stream_asset_error)?;
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
    with_graph(&state, |g| {
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
    let stored = with_graph(&state, |graph| {
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
    let recorded_path = path.filter(|p| !p.trim().is_empty());
    let id = if let Some(path) = recorded_path {
        if slot.graph.resolve_rel(&path).is_none() {
            return Err("invalid page path".into());
        }
        PageId::from(path)
    } else {
        match slot
            .store
            .whole_graph()
            .map_err(|e| format!("{e:?}"))?
            .resolve(&name, kind == PageKind::Journal)
        {
            Resolved::Existing { id, .. } => id,
            Resolved::Alias { owners } => owners.into_iter().next().ok_or("alias has no owner")?,
            Resolved::Absent { id } => id,
        }
    };
    let target = page_handoff_target(&slot, &id)?;
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
pub(crate) fn list_orphan_assets(
    state: GraphContext<'_>,
) -> Result<Vec<tine_core::model::AssetInfo>, String> {
    with_graph(&state, |g| Ok(g.orphan_assets()))
}

/// Move an orphaned asset to the recoverable trash.
#[tauri::command]
pub(crate) fn trash_asset(name: String, state: GraphContext<'_>) -> Result<(), String> {
    with_graph(&state, |g| g.trash_asset(&name).map_err(|e| e.to_string()))
}

/// Count + total bytes in the recoverable asset trash.
#[tauri::command]
pub(crate) fn asset_trash_stats(
    state: GraphContext<'_>,
) -> Result<tine_core::model::TrashStats, String> {
    let mut stats = tine_core::model::TrashStats::default();
    for (kind, count, bytes) in slot_for_context(&state)?
        .store
        .trash_stats()
        .map_err(store_error)?
    {
        match kind {
            TrashKind::Asset => {
                stats.count = count;
                stats.bytes = bytes;
            }
            TrashKind::Page => stats.pages = count,
            TrashKind::Journal => stats.journals = count,
            TrashKind::Conflict => stats.conflicts = count,
            TrashKind::Legacy => stats.other = count,
        }
    }
    Ok(stats)
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
pub(crate) fn list_journal_conflicts(
    state: GraphContext<'_>,
) -> Result<Vec<tine_core::model::JournalConflict>, String> {
    with_graph(&state, |g| Ok(g.journal_conflicts()))
}

/// Sync-tool conflict copies (Syncthing/Dropbox) sitting in the graph — for the
/// user to review + reconcile instead of them showing as garbage pages.
#[tauri::command]
pub(crate) fn list_sync_conflicts(
    state: GraphContext<'_>,
) -> Result<Vec<tine_core::model::SyncConflict>, String> {
    with_graph(&state, |g| Ok(g.list_sync_conflicts()))
}

/// Block-level diff of a sync-conflict copy against its winner (both graph-root-
/// relative paths) — the data behind the two-column merge UI. Read-only.
#[tauri::command]
pub(crate) fn sync_conflict_diff(
    winner: String,
    conflict: String,
    state: GraphContext<'_>,
) -> Result<Option<tine_core::sync_diff::SyncConflictDiff>, String> {
    with_graph(&state, |g| {
        g.sync_conflict_diff(&winner, &conflict)
            .map_err(|e| e.to_string())
    })
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
    with_graph(&state, |g| {
        g.resolve_sync_conflict(
            &winner,
            &conflict,
            &decisions,
            &base_rev,
            &conflict_rev,
            pre_choice.as_deref().unwrap_or("union"),
        )
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                "conflict".to_string()
            } else {
                e.to_string()
            }
        })
    })
}

/// Discard a sync-conflict copy without merging (move it to the recoverable
/// trash). Refuses anything that isn't a conflict copy.
#[tauri::command]
pub(crate) fn trash_sync_conflict(conflict: String, state: GraphContext<'_>) -> Result<(), String> {
    with_graph(&state, |g| {
        g.trash_sync_conflict(&conflict).map_err(|e| e.to_string())
    })
}

/// Move one journal file (by exact filename) to the recoverable trash.
#[tauri::command]
pub(crate) fn trash_journal_file(name: String, state: GraphContext<'_>) -> Result<(), String> {
    with_graph(&state, |g| {
        g.trash_journal_file(&name).map_err(|e| e.to_string())
    })
}

/// Raw contents of one journal file (by exact filename) — for inspecting a
/// duplicate day's files before reconciling.
#[tauri::command]
pub(crate) fn read_journal_file(name: String, state: GraphContext<'_>) -> Result<String, String> {
    with_graph(&state, |g| {
        g.read_journal_file(&name).map_err(|e| e.to_string())
    })
}

/// Load a page from a SPECIFIC file by its graph-root-relative path — lets the UI
/// navigate to a duplicate-day stray that shares a (kind,name) with the canonical
/// file and so is unreachable by name (#21).
#[tauri::command]
pub(crate) fn get_page_by_path(
    path: String,
    state: GraphContext<'_>,
) -> Result<Option<PageDto>, String> {
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
    with_graph(&state, |g| {
        g.merge_pages(&src, &dst).map_err(|e| e.to_string())
    })
}

/// Rescue a duplicate-day stray by moving it to a uniquely-named page
/// (`pages/<new_name>`), so it stops colliding and becomes normally navigable (#21).
#[tauri::command]
pub(crate) fn rename_file_to_page(
    path: String,
    new_name: String,
    state: GraphContext<'_>,
) -> Result<(), String> {
    with_graph(&state, |g| {
        g.rename_file_to_page(&path, &new_name)
            .map_err(|e| e.to_string())
    })
}

#[tauri::command]
pub(crate) fn save_asset(
    name: String,
    bytes_b64: String,
    state: GraphContext<'_>,
) -> Result<String, String> {
    let bytes = decode_asset_b64(&bytes_b64)?;
    with_graph(&state, |g| {
        g.save_asset(&name, &bytes).map_err(|e| e.to_string())
    })
}

#[tauri::command]
pub(crate) fn read_highlights(
    pdf: String,
    state: GraphContext<'_>,
) -> Result<Vec<tine_core::pdf::Highlight>, String> {
    with_graph(&state, |g| Ok(g.read_highlights(&pdf)))
}

#[tauri::command]
pub(crate) fn open_pdf(
    pdf: String,
    label: String,
    state: GraphContext<'_>,
) -> Result<tine_core::pdf::PdfState, String> {
    with_graph(&state, |g| {
        g.open_pdf(&pdf, &label).map_err(|e| e.to_string())
    })
}

#[tauri::command]
pub(crate) fn write_highlights(
    pdf: String,
    label: String,
    highlights: Vec<tine_core::pdf::Highlight>,
    base_ids: Vec<String>,
    state: GraphContext<'_>,
) -> Result<(), String> {
    with_graph(&state, |g| {
        g.write_highlights(&pdf, &label, &highlights, &base_ids)
            .map_err(|e| e.to_string())
    })
}

#[tauri::command]
pub(crate) fn write_pdf_view_state(
    pdf: String,
    page: i64,
    scale: f64,
    state: GraphContext<'_>,
) -> Result<(), String> {
    with_graph(&state, |g| {
        g.write_pdf_view_state(&pdf, page, scale)
            .map_err(|e| e.to_string())
    })
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
    with_graph(&state, |g| {
        g.write_pdf_area_image(&pdf, page, &id, stamp, &bytes)
            .map_err(|e| e.to_string())
    })
}
