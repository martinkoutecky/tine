//! Reviewed query and live publication. Selection and query answers come from
//! one `WholeGraph` view. Query output goes to Store-owned published-queries
//! leaves; whole-graph live/CLI exports remain create-only outside the graph. Store stages,
//! syncs, preserves replaced output and installs without clobbering a winner.
//! This module never writes source pages or runs a second query evaluator.
//!
//! `plan_query` costs O(P + selected page bytes + query evaluation) and returns
//! a fingerprint over the reviewed membership and held source documents. `publish_query`
//! repeats that work and refuses a changed plan. Every output projects those
//! held documents, so later external edits cannot mix unreviewed content in. `publish_live` costs O(P + B)
//! and exports public pages, or all pages on explicit request. Query exports
//! suppress nested-query counts of outside results. Observable
//! failures are parser/selection refusal, output budget, stale plan and I/O;
//! callers show them. For query exports, an asset-budget refusal offers Settings,
//! missing assets are visible warnings, and replacement reports retained output.

use crate::render::{self, RenderGraph, SheetExport, SheetIndex};
use crate::render_query_cache::substitute_current_page;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io;
use std::path::Path;
use tine_core::model::{GraphMeta, PageKind};
use tine_core::query::ir::{ExecutionContext, QueryResult, QueryRows};
use tine_core::query::macro_text::query_macro_extents;
use tine_core::query::wire_parse::{
    anchored_view, parse_query_pair, ParsedQuery, QueryTextDialect,
};
use tine_core::refs::page_key;
use tine_store::Resolved;
use tine_store::{IrAnswer, IrRequest, Store, WholeGraph};

const MAX_PAGES: usize = 20_000;
const MAX_EXPORT_BYTES: usize = 128 * 1024 * 1024;

/// The export clock in seconds: `SOURCE_DATE_EPOCH` when set (reproducible
/// builds), otherwise now. A CLI caller reads it in its own process and
/// passes it to [`publish_query_at`], so a running owner never substitutes
/// its own environment.
pub fn export_epoch() -> io::Result<i64> {
    match std::env::var("SOURCE_DATE_EPOCH") {
        Ok(raw) => raw
            .parse::<i64>()
            .map_err(|_| refusal("invalid SOURCE_DATE_EPOCH")),
        Err(_) => Ok(std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_secs() as i64),
    }
}

fn export_time(epoch: i64) -> io::Result<String> {
    time::OffsetDateTime::from_unix_timestamp(epoch)
        .map_err(io::Error::other)?
        .format(&time::format_description::well_known::Rfc3339)
        .map_err(io::Error::other)
}

pub use crate::publication_options::QueryExportRequest;

/// Resolve one persisted block UUID to the dialog's query publication input.
/// O(log I + candidate-page blocks) over one held graph view, no writes. The parser owns query
/// recognition and raw arguments; publication still uses `plan_query`/`publish_query`.
pub fn query_request_by_uuid(
    store: &Store,
    uuid: &str,
    name: Option<&str>,
    replace: bool,
) -> io::Result<QueryExportRequest> {
    let graph = store
        .whole_graph()
        .map_err(|e| io::Error::other(format!("graph load failed: {e:?}")))?;
    let corpus = graph.corpus(tine_store::CorpusScope::BlockIdCandidates(uuid), true);
    let mut found = None;
    for page in &corpus.pages {
        let mut stack: Vec<_> = page.document.roots.iter().collect();
        while let Some(block) = stack.pop() {
            #[cfg(feature = "test-faults")]
            tine_store::cost_counters::query_uuid_block(uuid);
            stack.extend(block.children.iter());
            if block
                .property("id")
                .is_some_and(|id| id.eq_ignore_ascii_case(uuid))
            {
                if found.is_some() {
                    return Err(refusal(format!("ambiguous block UUID: {uuid}")));
                }
                found = Some((page, block));
            }
        }
    }
    let (page, block) = found.ok_or_else(|| refusal(format!("unknown block UUID: {uuid}")))?;
    let (argument, dialect, title) =
        render::query_for_export(block.visible_text(), block.is_org()).map_err(refusal)?;
    let properties = block
        .properties()
        .into_iter()
        .filter(|(key, _)| query_parse_property(key) || display_only_property(key))
        .collect();
    Ok(QueryExportRequest {
        argument,
        dialect,
        properties,
        current_page: Some(page.name.clone()),
        name: name
            .map(str::to_owned)
            .or(title)
            .unwrap_or_else(|| format!("Query {uuid}")),
        host_block_id: Some(block.uuid.to_string()),
        folder: None,
        replace,
        asset_budget_bytes: None,
        include_query_text: false,
        include_comments: None,
    })
}

pub use crate::publication_options::{ExportPage, ExportReceipt, LeftOutPage, QueryExportPlan};

/// Default cumulative budget for a query export's copied assets, in bytes.
pub const QUERY_EXPORT_DEFAULT_ASSET_BUDGET_BYTES: u64 = 1024 * 1024 * 1024;

/// Typed refusal: assets would exceed the device limit. No output was committed.
#[derive(Debug)]
pub struct AssetBudgetExceeded {
    pub limit: u64,
    pub len: u64,
}
impl std::fmt::Display for AssetBudgetExceeded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Export stopped: copied assets exceed the {} MiB limit (next asset: {} bytes). Raise \"Query export size limit\" in Settings → Graph, or remove the asset from the exported pages.", self.limit as f64 / 1048576.0, self.len)
    }
}
impl std::error::Error for AssetBudgetExceeded {}

pub(crate) fn refusal(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

fn slug(name: &str) -> String {
    let mut out = String::new();
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
        if out.len() >= 64 {
            break;
        }
    }
    let trimmed = out.trim_end_matches('-');
    if trimmed.is_empty() {
        "export".into()
    } else {
        trimmed.into()
    }
}

fn ir_registry(
    graph: &WholeGraph,
) -> io::Result<std::sync::Arc<tine_core::query::registry::Registry>> {
    match graph
        .query_ir(IrRequest::Registry, &tine_store::Cancel::default())
        .map(|answer| answer.expect("never cancelled"))
        .map_err(|e| io::Error::other(e.to_string()))?
    {
        IrAnswer::Registry(registry) => Ok(registry),
        _ => Err(io::Error::other("query registry returned another answer")),
    }
}

fn parse_and_run(
    graph: &WholeGraph,
    argument: &str,
    dialect: QueryTextDialect,
    properties: &[(String, String)],
    context: &ExecutionContext,
    require_supported: bool,
) -> io::Result<(ParsedQuery, QueryResult)> {
    if !tine_core::query::query_source_within_limit(argument)
        || !tine_core::query::query_nesting_within_limit(argument)
    {
        return Err(refusal("query exceeds the export parser limit"));
    }
    let registry = ir_registry(graph)?;
    let parsed = parse_query_pair(argument, dialect, properties, &registry);
    let anchor = parsed.query.anchor;
    let view = anchored_view(&parsed, anchor);
    let result = match graph
        .query_ir(
            IrRequest::Run {
                query: &parsed.query,
                view: &view,
                context,
            },
            &tine_store::Cancel::default(),
        )
        .map(|answer| answer.expect("never cancelled"))
        .map_err(|e| io::Error::other(e.to_string()))?
    {
        IrAnswer::Result(result) => *result,
        _ => return Err(io::Error::other("query run returned another answer")),
    };
    if require_supported && (result.exceeded || result.total > MAX_PAGES * 32) {
        return Err(refusal("query export exceeds the bounded result limit"));
    }
    if require_supported
        && (!result.report.supported || result.diagnostics.iter().any(|d| !d.disabled))
    {
        return Err(refusal("this query is not fully supported for export"));
    }
    Ok((parsed, result))
}

pub(crate) struct Planned {
    pub(crate) plan: QueryExportPlan,
    pub(crate) parsed: ParsedQuery,
    pub(crate) result: QueryResult,
    pub(crate) selected: tine_core::Corpus,
}

pub(crate) fn resolve_plan(
    store: &Store,
    graph: &WholeGraph,
    request: &QueryExportRequest,
) -> io::Result<Planned> {
    resolve_plan_review(store, graph, request, None)
}

fn resolve_plan_review(
    store: &Store,
    graph: &WholeGraph,
    request: &QueryExportRequest,
    snapshot_byte_limit: Option<u64>,
) -> io::Result<Planned> {
    if request.name.trim().is_empty() {
        return Err(refusal("give the export a name"));
    }
    if request.name.len() > 256
        || request.properties.len() > 128
        || request
            .current_page
            .as_ref()
            .is_some_and(|page| page.len() > 512)
        || request
            .host_block_id
            .as_ref()
            .is_some_and(|id| id.len() > 128)
    {
        return Err(refusal("query export metadata exceeds its limit"));
    }
    let context = ExecutionContext {
        current_page: request.current_page.clone(),
    };
    let (parsed, mut result) = parse_and_run(
        graph,
        &request.argument,
        request.dialect,
        &request.properties,
        &context,
        true,
    )?;
    if let (Some(host), QueryRows::Block { groups }) = (&request.host_block_id, &mut result.rows) {
        for group in groups.iter_mut() {
            group.blocks.retain(|block| &block.id != host);
        }
        groups.retain(|group| !group.blocks.is_empty());
        result.total = groups.iter().map(|group| group.blocks.len()).sum();
        result.matched_total = Some(result.total);
    }
    let corpus = graph.corpus(None, true);
    // Owner files by (kind, page_key), built once rather than per group. Page
    // identity is `page_key` everywhere here (census Q1), as in the reader
    // (`publishedBackend.ts pageByName`): two files claiming one key are an
    // ambiguous owner for every name, not only an ASCII one.
    let ambiguous = render::ambiguous_source_keys(&graph);
    let mut owners: HashMap<_, Vec<&str>> = HashMap::new();
    for page in &corpus.pages {
        (owners.entry((page.kind, page_key(&page.name))).or_default()).push(page.id.as_str());
    }
    let mut paths = HashSet::new();
    let anchor = match &result.rows {
        QueryRows::Page { pages } => {
            for page in pages {
                paths.insert(page.path.clone());
            }
            "page"
        }
        QueryRows::Block { groups } => {
            for group in groups {
                match owners.get(&(group.kind, page_key(&group.page))) {
                    Some(files)
                        if files.len() == 1 && !ambiguous.contains(&page_key(&group.page)) =>
                    {
                        paths.insert(files[0].to_owned())
                    }
                    _ => return Err(refusal("query result has an ambiguous page owner")),
                };
            }
            "block"
        }
    };
    if paths.len() > MAX_PAGES {
        return Err(refusal("query selects too many pages"));
    }
    let mut chosen: Vec<_> = corpus
        .pages
        .into_iter()
        .filter(|page| paths.contains(page.id.as_str()))
        .collect();
    if chosen.len() != paths.len() {
        return Err(refusal("query result owner is missing"));
    }
    chosen.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));
    // WYSIWYG selection minus the two rules every export applies
    // (SHARE-DIALOG §1–2): private pages and the exports page stay out, and the
    // plan names them so the review shows what was left out.
    let mut left_out = Vec::new();
    chosen.retain(|page| match render::export_left_out(page) {
        Some(reason) => {
            left_out.push(LeftOutPage {
                name: page.name.clone(),
                path: page.id.as_str().into(),
                reason,
            });
            false
        }
        None => true,
    });
    let include_comments =
        crate::output_comments::included(request.include_comments, &request.properties);
    crate::output_comments::project(&mut chosen, include_comments);
    if !include_comments {
        crate::output_comments::close_result(&mut result, &chosen);
    }
    if chosen.is_empty() && !left_out.is_empty() {
        return Err(refusal(
            "every page this query selects is private or the exports page",
        ));
    }
    let selects_host_page = request.current_page.as_deref().is_some_and(|host| {
        chosen
            .iter()
            .any(|page| page_key(&page.name) == page_key(host))
    });
    let selected = tine_core::Corpus { pages: chosen };
    // The answer is closed to the exported pages too, so neither a ticked
    // "Include the query text" home nor the row count carries a left-out row.
    if !left_out.is_empty() {
        let paths: HashSet<&str> = selected.pages.iter().map(|p| p.id.as_str()).collect();
        let names: HashSet<_> = selected
            .pages
            .iter()
            .map(|p| (p.kind, page_key(&p.name)))
            .collect();
        result = close_query_result(result, &paths, &names);
    }
    let pages: Vec<ExportPage> = selected
        .pages
        .iter()
        .map(|p| ExportPage {
            name: p.name.clone(),
            path: p.id.as_str().into(),
            journal: p.kind == PageKind::Journal,
        })
        .collect();
    let mut hasher = Sha256::new();
    // Selection/content fingerprint stays stable across an explicit destination
    // choice or device budget change; none can admit unreviewed source content.
    let mut reviewed = request.clone();
    reviewed.folder = None;
    reviewed.replace = false;
    reviewed.asset_budget_bytes = None;
    reviewed.include_query_text = false;
    reviewed.include_comments = Some(include_comments);
    hasher.update(serde_json::to_vec(&reviewed).map_err(io::Error::other)?);
    hasher.update(serde_json::to_vec(&result.rows).map_err(io::Error::other)?);
    for page in &selected.pages {
        hasher.update(page.id.as_str().as_bytes());
        hasher.update(serde_json::to_vec(page.document.as_ref()).map_err(io::Error::other)?);
    }
    let fingerprint = format!("{:x}", hasher.finalize());
    let folder = request
        .folder
        .clone()
        .unwrap_or_else(|| slug(&request.name));
    let (path, exists, suggested_folder, previous_snapshot) =
        tine_store::publish::query_publication_destination(store, &folder, snapshot_byte_limit)?;
    Ok(Planned {
        plan: QueryExportPlan {
            include_comments,
            anchor: anchor.into(),
            row_count: result.total,
            pages,
            left_out,
            selects_host_page,
            folder,
            path,
            exists,
            suggested_folder,
            fingerprint,
            previous_pages: previous_membership(previous_snapshot),
        },
        parsed,
        result,
        selected,
    })
}

/// Resolve one query through og's in-memory answerer and return the complete
/// owner-page set with a source-revision fingerprint. No output is written.
pub fn plan_query(store: &Store, request: &QueryExportRequest) -> io::Result<QueryExportPlan> {
    let graph = store
        .whole_graph()
        .map_err(|e| io::Error::other(format!("graph load failed: {e:?}")))?;
    // Match the existing export byte ceiling. Only the read-only review asks
    // for membership evidence; confirmation and quick-share preparation do not.
    Ok(resolve_plan_review(store, &graph, request, Some(MAX_EXPORT_BYTES as u64))?.plan)
}

fn previous_membership(bytes: Option<Vec<u8>>) -> Option<Vec<String>> {
    #[derive(Deserialize)]
    struct MembershipPage {
        name: String,
        path: String,
    }
    #[derive(Deserialize)]
    struct MembershipSnapshot {
        pages: Vec<MembershipPage>,
    }
    let snapshot: MembershipSnapshot = serde_json::from_slice(&bytes?).ok()?;
    // A query export's generated home has an empty path; it is not an owner page.
    Some(
        snapshot
            .pages
            .into_iter()
            .filter(|page| !page.path.is_empty())
            .map(|page| page.name)
            .collect(),
    )
}

fn collect_static(
    store: &Store,
    graph: &WholeGraph,
    corpus: &tine_core::Corpus,
    sheets: &SheetIndex,
    query_export: bool,
    asset_budget: u64,
    warnings: &mut Vec<String>,
) -> io::Result<Vec<(String, Vec<u8>)>> {
    let config = store.config();
    let mut render_graph = RenderGraph::new(corpus, graph, store, Some(sheets));
    render_graph.query_export = query_export;
    let mut files = Vec::new();
    let mut used = 0usize;
    render::publish_graph(
        &render_graph,
        render::PageSelection::Preselected,
        &config.favorites,
        &mut |name, bytes| {
            used = used
                .checked_add(bytes.len())
                .ok_or_else(|| refusal("export byte budget exceeded"))?;
            if !query_export && used > MAX_EXPORT_BYTES {
                return Err(refusal("export byte budget exceeded"));
            }
            let bytes = if name.ends_with(".html") {
                String::from_utf8(bytes.to_vec())
                    .map_err(|_| refusal("rendered HTML is not UTF-8"))?
                    .replace("../assets/", "assets/")
                    .into_bytes()
            } else {
                bytes.to_vec()
            };
            files.push((name.to_owned(), bytes));
            Ok(())
        },
    )?;
    files.extend(
        publication_assets(store, corpus, asset_budget, warnings).map_err(|error| match error {
            tine_store::StoreError::TooLarge { len, .. } if query_export => io::Error::new(
                io::ErrorKind::InvalidInput,
                AssetBudgetExceeded {
                    limit: asset_budget,
                    len,
                },
            ),
            other => crate::store_error(other),
        })?,
    );
    Ok(files)
}

fn close_query_result(
    mut result: QueryResult,
    paths: &HashSet<&str>,
    names: &HashSet<(PageKind, String)>,
) -> QueryResult {
    match &mut result.rows {
        QueryRows::Page { pages } => pages.retain(|page| paths.contains(page.path.as_str())),
        QueryRows::Block { groups } => {
            groups.retain(|group| names.contains(&(group.kind, page_key(&group.page))))
        }
    }
    result.total = match &result.rows {
        QueryRows::Page { pages } => pages.len(),
        QueryRows::Block { groups } => groups.iter().map(|group| group.blocks.len()).sum(),
    };
    result.matched_total = Some(result.total);
    result.statistics = None;
    result
}

/// Match QueryMacro's parser request. Legacy columns and sort stay in the
/// block text for the renderer; only query-table affects the parsed reading.
/// Publication owns baking this exact key, rather than making the reader
/// guess which differently configured query's answer it can reuse.
/// Properties that are part of a query's parse key: what the renderer's
/// `hostProperties` sends to the engine (`src/components/Macro.tsx`).
fn query_parse_property(key: &str) -> bool {
    (key.starts_with("tine.") || key == "query-table") && !display_only_property(key)
}

/// Renderer-only properties: not part of the parse key, but they travel to a
/// generated query home so the export looks like the source block.
/// `tine.breadcrumbs` is presentation only (GH #881).
fn display_only_property(key: &str) -> bool {
    matches!(
        key,
        "query-properties" | "query-sort-by" | "query-sort-desc" | "tine.breadcrumbs"
    )
}

/// One record for each query macro the reader can show: those in blocks, with
/// their host block's `tine.*` properties, and those in a page's properties
/// (the page header, which the reader renders with no host block and so no
/// properties).
fn baked_queries(graph: &WholeGraph, corpus: &tine_core::Corpus) -> io::Result<Vec<Value>> {
    let paths: HashSet<_> = corpus.pages.iter().map(|page| page.id.as_str()).collect();
    let names: HashSet<_> = corpus
        .pages
        .iter()
        .map(|page| (page.kind, page_key(&page.name)))
        .collect();
    let mut records = Vec::new();
    let registry = ir_registry(graph)?;
    for page in &corpus.pages {
        let mut bake = |raw: &str, properties: Vec<(String, String)>| -> io::Result<()> {
            // The raw-extent reader is quadratic with many malformed starts;
            // bound its input before calling it (I-22).
            if raw.len() > tine_core::query::QUERY_SOURCE_MAX_BYTES {
                return Ok(());
            }
            for extent in query_macro_extents(raw) {
                if records.len() >= 256 {
                    return Err(refusal("too many queries in published pages"));
                }
                let dialect = if extent.name.eq_ignore_ascii_case("tine-query") {
                    QueryTextDialect::MacroTql
                } else {
                    QueryTextDialect::MacroQuery
                };
                let properties: Vec<_> = properties
                    .iter()
                    .filter(|(key, _)| query_parse_property(key))
                    .cloned()
                    .collect();
                let context = ExecutionContext {
                    current_page: Some(page.name.clone()),
                };
                let parsed = parse_query_pair(&extent.argument, dialect, &properties, &registry);
                let execution = substitute_current_page(&extent.argument, &page.name);
                let executed = execution.as_deref().unwrap_or(&extent.argument);
                let (run_parsed, result) =
                    parse_and_run(graph, executed, dialect, &properties, &context, false)?;
                let view = anchored_view(&run_parsed, run_parsed.query.anchor);
                let result = close_query_result(result, &paths, &names);
                let mut record = json!({ "host": page.name, "argument": extent.argument,
                    "dialect": dialect, "properties": properties, "parsed": parsed,
                    "context": context, "executed_context": context,
                    "view": view, "result": result });
                if let Some(argument) = execution {
                    record["execution"] = json!({ "argument": argument, "parsed": run_parsed });
                }
                records.push(record);
            }
            Ok(())
        };
        if let Some(pre) = &page.document.pre_block {
            let is_org = tine_core::page_properties::page_document_is_org(&page.document);
            for (_, value) in tine_core::page_properties::page_property_lines(pre, is_org) {
                bake(&value, Vec::new())?;
            }
        }
        let mut stack: Vec<_> = page.document.roots.iter().collect();
        while let Some(block) = stack.pop() {
            stack.extend(block.children.iter());
            bake(block.raw(), block.properties())?;
        }
    }
    Ok(records)
}

fn snapshot(
    graph: &WholeGraph,
    meta: &GraphMeta,
    corpus: &tine_core::Corpus,
    name: &str,
    home: &str,
    home_query: Option<(&QueryExportRequest, &ParsedQuery, &QueryResult)>,
    epoch: i64,
) -> io::Result<Vec<u8>> {
    snapshot_at(
        graph,
        meta,
        corpus,
        name,
        home,
        home_query,
        &export_time(epoch)?,
    )
}

/// [`snapshot`] stamped with a given `exported_at` (quick share stamps its
/// share document's `created_at`). `graph` is the private build graph of
/// `corpus` (`build_graph.rs`) and `meta` its metadata: every answer below,
/// `answers` included, is then closed by the exported pages.
pub(crate) fn snapshot_at(
    graph: &WholeGraph,
    meta: &GraphMeta,
    corpus: &tine_core::Corpus,
    name: &str,
    home: &str,
    home_query: Option<(&QueryExportRequest, &ParsedQuery, &QueryResult)>,
    exported_at: &str,
) -> io::Result<Vec<u8>> {
    let mut pages: Vec<Value> = Vec::new();
    let mut entries: Vec<Value> = Vec::new();
    let selected: HashSet<_> = corpus
        .pages
        .iter()
        .map(|p| (p.kind, page_key(&p.name)))
        .collect();
    let inventory = graph.inventory(tine_store::InventoryScope::All);
    // One pass over the inventory, not one per page (I-15).
    let mut days = HashMap::new();
    for entry in &inventory.0 {
        if let (Resolved::Existing { id, .. }, Some(day)) = (&entry.target, entry.day) {
            days.entry(id.as_str()).or_insert(day.0);
        }
    }
    for page in &corpus.pages {
        let blocks: Vec<_> = page
            .document
            .roots
            .iter()
            .map(tine_core::projection::block_to_dto)
            .collect();
        let mut value = json!({ "name": page.name, "kind": page.kind, "title": page.name,
            "pre_block": page.document.pre_block, "blocks": blocks,
            "format": tine_core::model::Format::from_path(Path::new(page.id.as_str())),
            "read_only": true, "guide": false });
        value["path"] = json!(page.id.as_str());
        value["rev"] = Value::Null;
        value["activation"] = Value::Null;
        pages.push(value);
        let day = days.get(page.id.as_str());
        entries.push(json!({ "name": page.name, "kind": page.kind, "date_key": day, "path": page.id.as_str() }));
    }
    let mut queries = baked_queries(graph, corpus)?;
    if let Some((request, parsed, result)) = home_query {
        let macro_name = match request.dialect {
            QueryTextDialect::MacroTql | QueryTextDialect::Tql => "tine-query",
            _ => "query",
        };
        let mut raw = format!("{{{{{macro_name} {}}}}}", request.argument.trim());
        for (key, value) in &request.properties {
            if (query_parse_property(key) || display_only_property(key))
                && !key.contains('\n')
                && !value.contains('\n')
            {
                raw.push_str(&format!("\n  {key}:: {}", value.trim()));
            }
        }
        let block_id = format!("published-query:{}", slug(home));
        let mut query_block = json!({ "id": block_id, "raw": raw,
            "collapsed": false, "children": [], "breadcrumb": [] });
        if !request.properties.is_empty() {
            query_block["properties"] = json!(request.properties);
        }
        // Without "Include the query text" the home is only the page list: no
        // macro, no parsed query, no answer record (PK3, SHARE-DIALOG §1).
        let with_query = request.include_query_text;
        let mut home_blocks = if with_query {
            vec![query_block]
        } else {
            Vec::new()
        };
        for (index, page) in corpus.pages.iter().enumerate() {
            home_blocks.push(json!({ "id": format!("published-page:{index}"),
                "raw": format!("[[{}]]", page.name), "collapsed": false,
                "children": [], "breadcrumb": [] }));
        }
        pages.insert(
            0,
            json!({ "name": home, "kind": "page", "title": home,
            "pre_block": null, "blocks": home_blocks, "guide": false,
            "read_only": true, "format": "md", "path": "", "activation": null, "rev": null }),
        );
        entries.insert(
            0,
            json!({ "name": home, "kind": "page", "date_key": null, "path": "" }),
        );
        let context = ExecutionContext {
            current_page: Some(home.to_owned()),
        };
        // The page the query ran on, named only when it is an exported page
        // (its canonical name): an unexported host is omitted, never named
        // (closure, `docs/contracts/quick-share.md`).
        let executed_context = ExecutionContext {
            current_page: request.current_page.as_deref().and_then(|host| {
                let key = page_key(host);
                corpus
                    .pages
                    .iter()
                    .find(|page| page_key(&page.name) == key)
                    .map(|page| page.name.clone())
            }),
        };
        if with_query {
            let view = anchored_view(parsed, parsed.query.anchor);
            let properties: Vec<_> = request
                .properties
                .iter()
                .filter(|(key, _)| query_parse_property(key))
                .collect();
            queries.push(
                json!({ "host": home, "argument": request.argument, "dialect": request.dialect,
                "properties": properties, "parsed": parsed, "context": context,
                "executed_context": executed_context, "view": view, "result": result }),
            );
        }
    }
    // Only selected sources may cross into a published answer. The whole-graph
    // backlink answerer supplies rows; this projection closes them by owner.
    let mut backlinks = BTreeMap::new();
    for page in &corpus.pages {
        let groups = graph
            .backlinks(&page.name)
            .map_err(|e| io::Error::other(format!("{e:?}")))?;
        let closed: Vec<_> = groups
            .iter()
            .filter(|g| selected.contains(&(g.kind, page_key(&g.page))))
            .cloned()
            .collect();
        if !closed.is_empty() {
            backlinks.insert(page.name.clone(), closed);
        }
    }
    let names: Vec<_> = corpus.pages.iter().map(|p| p.name.clone()).collect();
    let icons = graph.page_icons(&names);
    let mut owner_names = HashMap::new();
    for page in &corpus.pages {
        owner_names.entry(page.id.as_str()).or_insert(&page.name);
    }
    let aliases: Vec<_> = inventory
        .0
        .iter()
        .filter_map(|entry| match &entry.target {
            Resolved::Alias { owners } if owners.len() == 1 => owner_names
                .get(owners[0].as_str())
                .map(|owner| (entry.name.clone(), (*owner).clone())),
            _ => None,
        })
        .collect();
    let block_ref_counts = corpus.block_ref_counts();
    let answers = crate::published_answers::answers(
        graph,
        meta,
        corpus,
        &inventory,
        home_query.map(|_| home),
    )?;
    let snapshot = json!({ "schema": 1, "name": name, "exported_at": exported_at,
        "home": home, "pages": pages, "entries": entries, "backlinks": backlinks,
        "block_ref_counts": block_ref_counts, "aliases": aliases, "icons": icons, "queries": queries,
        "answers": answers });
    let bytes = serde_json::to_vec(&snapshot).map_err(io::Error::other)?;
    if home_query.is_none() && bytes.len() > MAX_EXPORT_BYTES {
        return Err(refusal("snapshot byte budget exceeded"));
    }
    Ok(bytes)
}

fn app_files(
    files: &mut Vec<(String, Vec<u8>)>,
    bundle: &[(String, Vec<u8>)],
    snapshot: Vec<u8>,
    name: &str,
) -> io::Result<()> {
    let index = bundle
        .iter()
        .find(|(path, _)| path == "index.html")
        .ok_or_else(|| refusal("this build does not embed the frontend"))?;
    let html = std::str::from_utf8(&index.1).map_err(|_| refusal("frontend shell is not UTF-8"))?;
    let at = html
        .find("<head>")
        .ok_or_else(|| refusal("frontend shell has no head"))?
        + 6;
    let mut app_index = html.to_owned();
    app_index.insert_str(
        at,
        "<meta name=\"tine-published\" content=\"snapshot.json\">",
    );
    let title = name
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    app_index = app_index.replacen("<title>Tine</title>", &format!("<title>{title}</title>"), 1);
    files.push(("app/index.html".into(), app_index.into_bytes()));
    files.push(("app/snapshot.json".into(), snapshot));
    for (path, bytes) in bundle {
        if path == "index.html" {
            continue;
        }
        if !path.starts_with("assets/") || path.contains("..") || path.contains('\\') {
            continue;
        }
        files.push((format!("app/{path}"), bytes.clone()));
    }
    if let Some((_, index)) = files.iter_mut().find(|(path, _)| path == "index.html") {
        let text =
            String::from_utf8(index.clone()).map_err(|_| refusal("static index is not UTF-8"))?;
        *index = text.replacen("<head>", "<head><script src=\"app-redirect.js\"></script>", 1)
            .replacen("<main>", "<main><p class=\"publish-app-note\">Serve this folder over HTTP to open it as an app (add <code>?static</code> to stay on this page).</p>", 1)
            .into_bytes();
    }
    files.push(("app-redirect.js".into(), b"(function () {\ntry {\nvar p = location.protocol;\nif ((p === \"http:\" || p === \"https:\") && !/[?&]static(=|&|$)/.test(location.search)) {\nlocation.replace(\"app/\" + location.hash);\n}\n} catch (_) {}\n})();\n".to_vec()));
    Ok(())
}

/// Read the assets exactly the supplied parsed pages mention, through the
/// store's validated asset reader. Candidate names come from tine-core's one
/// asset-reference collector (`Corpus::asset_refs`, shared with orphan-asset
/// detection). Missing files and non-files warn; `TooLarge` refuses the
/// caller's cumulative byte budget before publication. Reads are bounded by
/// the remaining budget. Cost O(selected text + asset bytes); no graph writes.
fn publication_assets(
    store: &Store,
    corpus: &tine_core::Corpus,
    budget: u64,
    warnings: &mut Vec<String>,
) -> Result<Vec<(String, Vec<u8>)>, tine_store::StoreError> {
    let mut names = corpus.asset_refs();
    // The orphan scanner keeps raw and decoded URL spellings. A browser
    // decodes the URL, so copy only the decoded name when both were seen.
    let encoded: Vec<_> = names
        .iter()
        .filter(|name| name.contains('%'))
        .cloned()
        .collect();
    for name in encoded {
        if tine_core::asset_refs::percent_decode(&name) != name {
            names.remove(&name);
        }
    }
    // The shared orphan answerer also marks ancestors (PDF area-image
    // directories). Publication copies files, so remove those directory
    // markers while preserving the complete nested reference.
    let referenced: Vec<_> = names.iter().cloned().collect();
    for name in referenced {
        for parent in Path::new(&name).ancestors().skip(1) {
            if let Some(parent) = parent.to_str() {
                names.remove(parent);
            }
        }
    }
    let mut names: Vec<_> = names.into_iter().collect();
    names.sort();
    let mut out = Vec::new();
    let mut remaining = budget;
    for name in names {
        // The scanner over-collects on purpose (orphan detection must not
        // miss a reference), so a candidate may be prose after `assets/`
        // rather than a file name. One that cannot name a file is simply
        // not an asset: skip it like a missing one instead of failing the
        // whole publication. A real read failure of an asset still fails.
        let Ok(id) = store.file_id(tine_store::Area::Assets, &name) else {
            continue;
        };
        match store.read(&id, Some(remaining)) {
            Ok((bytes, _)) => {
                remaining = remaining.saturating_sub(bytes.len() as u64);
                out.push((format!("assets/{name}"), bytes));
            }
            Err(tine_store::StoreError::InvalidTarget(_)) => {
                warnings.push(format!("Asset {name} was omitted: not a regular file."));
            }
            Err(tine_store::StoreError::NotFound(_)) => {
                warnings.push(format!("Asset {name} was omitted: file not found."));
            }
            Err(tine_store::StoreError::Io(error))
                if matches!(
                    error.kind(),
                    io::ErrorKind::InvalidFilename | io::ErrorKind::NotADirectory
                ) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(out)
}

fn commit(
    store: &Store,
    parent: &Path,
    leaf: &str,
    files: Vec<(String, Vec<u8>)>,
    pages: usize,
) -> io::Result<ExportReceipt> {
    let total: usize = files.iter().map(|(_, data)| data.len()).sum();
    if total > MAX_EXPORT_BYTES {
        return Err(refusal("export byte budget exceeded"));
    }
    let receipt = store
        .publish(
            tine_store::PublishDest::External { parent, leaf },
            &mut |writer| {
                for (path, bytes) in &files {
                    writer.write(path, bytes)?;
                }
                Ok(())
            },
        )
        .map_err(|failure| io::Error::new(failure.cause.kind, failure.cause.message))?;
    Ok(ExportReceipt {
        path: receipt.site.display().to_string(),
        pages,
        files: receipt.files,
        retired: receipt.previous_kept.map(|p| p.display().to_string()),
        warnings: Vec::new(),
    })
}

/// The `page_key` of every page a query export would publish (its planned
/// selection), for the sheet inputs' publication boundary.
pub(crate) fn planned_page_keys(
    store: &Store,
    request: &QueryExportRequest,
) -> io::Result<HashSet<String>> {
    let graph = store
        .whole_graph()
        .map_err(|e| io::Error::other(format!("graph load failed: {e:?}")))?;
    let planned = resolve_plan(store, &graph, request)?;
    Ok(planned
        .selected
        .pages
        .iter()
        .map(|page| tine_core::refs::page_key(&page.name))
        .collect())
}

/// Commit a reviewed query to its in-graph leaf. Replace retains the old leaf
/// and reports recovery; create refuses a concurrent winner. Default asset
/// budget is 1 GiB; AssetBudgetExceeded is typed inside the io::Error. It re-runs the
/// plan and refuses changed membership or content before creating output.
/// The resulting folder contains static HTML and the read-only browser app.
pub fn publish_query(
    store: &Store,
    request: &QueryExportRequest,
    fingerprint: &str,
    bundle: &[(String, Vec<u8>)],
) -> io::Result<ExportReceipt> {
    publish_query_with_sheets(store, request, fingerprint, bundle, Vec::new())
}

/// `publish_query` with the app's computed sheets; a sheet block without one
/// keeps its plain outline.
pub fn publish_query_with_sheets(
    store: &Store,
    request: &QueryExportRequest,
    fingerprint: &str,
    bundle: &[(String, Vec<u8>)],
    sheets: Vec<SheetExport>,
) -> io::Result<ExportReceipt> {
    publish_query_with_sheets_in(
        store,
        request,
        fingerprint,
        bundle,
        sheets,
        &std::env::temp_dir(),
    )
}

/// `publish_query_with_sheets` building its private build graph
/// (`build_graph.rs`) under `scratch` (the app passes its cache directory).
pub fn publish_query_with_sheets_in(
    store: &Store,
    request: &QueryExportRequest,
    fingerprint: &str,
    bundle: &[(String, Vec<u8>)],
    sheets: Vec<SheetExport>,
    scratch: &Path,
) -> io::Result<ExportReceipt> {
    let epoch = export_epoch()?;
    publish_at(store, request, fingerprint, bundle, sheets, scratch, epoch)
}

/// `publish_query` stamped with a caller-supplied export clock
/// ([`export_epoch`] read in the caller's process).
pub fn publish_query_at(
    store: &Store,
    request: &QueryExportRequest,
    fingerprint: &str,
    bundle: &[(String, Vec<u8>)],
    epoch: i64,
) -> io::Result<ExportReceipt> {
    let scratch = std::env::temp_dir();
    publish_at(
        store,
        request,
        fingerprint,
        bundle,
        Vec::new(),
        &scratch,
        epoch,
    )
}

fn publish_at(
    store: &Store,
    request: &QueryExportRequest,
    fingerprint: &str,
    bundle: &[(String, Vec<u8>)],
    sheets: Vec<SheetExport>,
    scratch: &Path,
    epoch: i64,
) -> io::Result<ExportReceipt> {
    let graph = store
        .whole_graph()
        .map_err(|e| io::Error::other(format!("graph load failed: {e:?}")))?;
    let planned = resolve_plan(store, &graph, request)?;
    if planned.plan.fingerprint != fingerprint {
        return Err(refusal("query export changed; review it again"));
    }
    if planned.selected.pages.is_empty() {
        return Err(refusal("query has no pages to export"));
    }
    let mut result = planned.result;
    let mut warnings = Vec::new();
    // Every emitted file (the static HTML fallback and the snapshot) is
    // rendered from the private build graph: an unexported page can change no
    // byte of it (closure, `docs/contracts/quick-share.md`). Asset bytes are
    // still read from the real store.
    let (mut files, snap) = crate::build_graph::with_build_graph(
        store,
        &planned.selected.pages,
        scratch,
        BUILD_GRAPH_REFUSED,
        |build, meta, corpus| {
            if request.include_query_text && !planned.plan.include_comments {
                crate::output_comments::remap_result_ids(
                    &mut result,
                    &planned.selected.pages,
                    &corpus.pages,
                );
            }
            let files = collect_static(
                store,
                build,
                &corpus,
                &SheetIndex::new(sheets),
                true,
                request
                    .asset_budget_bytes
                    .unwrap_or(QUERY_EXPORT_DEFAULT_ASSET_BUDGET_BYTES),
                &mut warnings,
            )?;
            // The synthetic home must not shadow a name the build graph
            // answers (a page, journal or alias, single- or multi-owner):
            // rename it, as before, against the complete inventory.
            let inventory = build.inventory(tine_store::InventoryScope::All);
            let spellings =
                build.inventory(tine_store::InventoryScope::PublishedV1Spellings(&inventory));
            let taken = |name: &str| {
                crate::published_answers::home_name_taken(&inventory, &spellings, name)
            };
            let mut home = request.name.trim().to_owned();
            if taken(&home) {
                home.push_str(" (export)");
            }
            while taken(&home) {
                home.push_str(" 2");
            }
            let snap = snapshot(
                build,
                meta,
                &corpus,
                &request.name,
                &home,
                Some((request, &planned.parsed, &result)),
                epoch,
            )?;
            Ok((files, snap))
        },
    )?;
    app_files(&mut files, bundle, snap, &request.name)?;
    let receipt = store
        .publish(
            tine_store::PublishDest::QueryLeaf {
                folder: &planned.plan.folder,
                replace: request.replace,
            },
            &mut |writer| {
                for (path, bytes) in &files {
                    writer.write(path, bytes)?;
                }
                Ok(())
            },
        )
        .map_err(|failure| {
            let recovery = failure
                .previous_kept
                .map(|p| format!(" Previous export kept at {}.", p.display()))
                .unwrap_or_default();
            io::Error::new(
                failure.cause.kind,
                format!("{}{recovery}", failure.cause.message),
            )
        })?;
    Ok(ExportReceipt {
        path: receipt.site.display().to_string(),
        pages: planned.selected.pages.len(),
        files: receipt.files,
        retired: receipt.previous_kept.map(|p| p.display().to_string()),
        warnings,
    })
}

/// Publish a whole-graph read-only browser app, with the static site as a
/// fallback. The caller explicitly selects `all_pages`; otherwise only pages
/// with `public:: true` are included. Output is create-only outside the graph.
pub fn publish_live(
    store: &Store,
    parent: &Path,
    name: &str,
    all_pages: bool,
    include_comments: bool,
    bundle: &[(String, Vec<u8>)],
) -> io::Result<ExportReceipt> {
    publish_live_with_sheets(
        store,
        parent,
        name,
        all_pages,
        include_comments,
        bundle,
        Vec::new(),
    )
}

/// `publish_live` opening on `home` when given (it must be a selected page,
/// else the export is refused before anything is written), otherwise on the
/// graph's configured `:default-home` page when selected, "Welcome to Tine",
/// or the first page (port of master's `AppHome`; og I1f, #35).
pub fn publish_live_home(
    store: &Store,
    parent: &Path,
    name: &str,
    all_pages: bool,
    include_comments: bool,
    home: Option<&str>,
    bundle: &[(String, Vec<u8>)],
) -> io::Result<ExportReceipt> {
    let scratch = std::env::temp_dir();
    live(
        store,
        parent,
        name,
        all_pages,
        include_comments,
        home,
        bundle,
        Vec::new(),
        &scratch,
    )
}

/// `publish_live` with the app's computed sheets; a sheet block without one
/// keeps its plain outline (the CLI has no frontend and calls `publish_live`).
pub fn publish_live_with_sheets(
    store: &Store,
    parent: &Path,
    name: &str,
    all_pages: bool,
    include_comments: bool,
    bundle: &[(String, Vec<u8>)],
    sheets: Vec<SheetExport>,
) -> io::Result<ExportReceipt> {
    let scratch = std::env::temp_dir();
    live(
        store,
        parent,
        name,
        all_pages,
        include_comments,
        None,
        bundle,
        sheets,
        &scratch,
    )
}

/// `publish_live_with_sheets` building its private build graph
/// (`build_graph.rs`) under `scratch` (the app passes its cache directory).
pub fn publish_live_with_sheets_in(
    store: &Store,
    parent: &Path,
    name: &str,
    all_pages: bool,
    include_comments: bool,
    bundle: &[(String, Vec<u8>)],
    sheets: Vec<SheetExport>,
    scratch: &Path,
) -> io::Result<ExportReceipt> {
    live(
        store,
        parent,
        name,
        all_pages,
        include_comments,
        None,
        bundle,
        sheets,
        scratch,
    )
}

/// The view a whole-graph export (`live`, `publish_static`) publishes from. An
/// export must see every byte on disk, including a file a sync client or a
/// restore tool rewrote with its size and stamp put back, which a stamp diff
/// (`Depth::Stamps`, the poller) misses: hash every graph-text file once
/// (O(graph bytes), like the export itself), then take the view. The removed
/// `publish_html` did this; the live exports did not (HF9pAI).
fn export_view(store: &Store) -> io::Result<WholeGraph> {
    store
        .refresh(tine_store::Depth::Bytes)
        .map_err(|e| io::Error::other(format!("graph refresh failed: {e:?}")))?;
    store
        .whole_graph()
        .map_err(|e| io::Error::other(format!("graph load failed: {e:?}")))
}

/// Refusal when an exported page set does not read back as itself in its
/// build graph (threat: malformed input or a parser round-trip defect;
/// `build_graph::with_build_graph`).
const BUILD_GRAPH_REFUSED: &str =
    "the exported pages did not read back as the same pages; nothing was exported";

#[allow(clippy::too_many_arguments)]
fn live(
    store: &Store,
    parent: &Path,
    name: &str,
    all_pages: bool,
    include_comments: bool,
    requested_home: Option<&str>,
    bundle: &[(String, Vec<u8>)],
    sheets: Vec<SheetExport>,
    scratch: &Path,
) -> io::Result<ExportReceipt> {
    if name.trim().is_empty() || name.len() > 256 {
        return Err(refusal("live export name is invalid"));
    }
    let graph = export_view(store)?;
    let mut corpus = graph.corpus(None, true);
    let selection = render::PageSelection::every_page(all_pages);
    corpus.pages.retain(|page| selection.includes(page));
    crate::output_comments::project(&mut corpus.pages, include_comments);
    if corpus.pages.len() > MAX_PAGES {
        return Err(refusal("live export selects too many pages"));
    }
    corpus.pages.sort_by(|a, b| a.name.cmp(&b.name));
    let find = |wanted: &str| {
        let wanted = tine_core::refs::page_key(wanted);
        corpus
            .pages
            .iter()
            .find(|p| tine_core::refs::page_key(&p.name) == wanted)
    };
    let home =
        match requested_home {
            Some(requested) => Some(find(requested).ok_or_else(|| {
                refusal("the requested home page is not among the exported pages")
            })?),
            None => store
                .config()
                .config
                .default_home
                .as_deref()
                .and_then(|configured| find(configured))
                .or_else(|| find("Welcome to Tine"))
                .or_else(|| corpus.pages.first()),
        }
        .map(|p| p.name.clone())
        .unwrap_or_default();
    // The static fallback and the snapshot both come from the build graph
    // (see `publish_query_with_sheets_in`).
    let (mut files, snap) = crate::build_graph::with_build_graph(
        store,
        &corpus.pages,
        scratch,
        BUILD_GRAPH_REFUSED,
        |build, meta, corpus| {
            let files = collect_static(
                store,
                build,
                &corpus,
                &SheetIndex::new(sheets),
                false,
                32 * 1024 * 1024,
                &mut Vec::new(),
            )?;
            Ok((
                files,
                snapshot(build, meta, &corpus, name, &home, None, export_epoch()?)?,
            ))
        },
    )?;
    app_files(&mut files, bundle, snap, name)?;
    commit(store, parent, &slug(name), files, corpus.pages.len())
}

/// Publish a create-only static HTML site to an external, user-selected
/// directory. Public markers select pages unless all pages are explicitly
/// requested; source graph files are never written.
pub fn publish_static(
    store: &Store,
    parent: &Path,
    name: &str,
    all_pages: bool,
    include_comments: bool,
) -> io::Result<ExportReceipt> {
    if name.trim().is_empty() || name.len() > 256 {
        return Err(refusal("static export name is invalid"));
    }
    let graph = export_view(store)?;
    let mut corpus = graph.corpus(None, true);
    let selection = render::PageSelection::every_page(all_pages);
    corpus.pages.retain(|page| selection.includes(page));
    crate::output_comments::project(&mut corpus.pages, include_comments);
    if corpus.pages.len() > MAX_PAGES {
        return Err(refusal("static export selects too many pages"));
    }
    corpus.pages.sort_by(|a, b| a.name.cmp(&b.name));
    // No frontend computes sheets for a CLI export: every sheet block stays a
    // plain outline. Rendered from the private build graph (see
    // `publish_query_with_sheets_in`), under the system temp directory.
    let files = crate::build_graph::with_build_graph(
        store,
        &corpus.pages,
        &std::env::temp_dir(),
        BUILD_GRAPH_REFUSED,
        |build, _meta, corpus| {
            collect_static(
                store,
                build,
                &corpus,
                &SheetIndex::default(),
                false,
                32 * 1024 * 1024,
                &mut Vec::new(),
            )
        },
    )?;
    commit(store, parent, &slug(name), files, corpus.pages.len())
}

#[cfg(test)]
mod snapshot_consistency_tests {
    use super::*;

    // The asset-reference scanner over-collects (orphan detection must not miss a
    // reference), so prose after `assets/` becomes a candidate name. A candidate
    // longer than a file name may be must be skipped like a missing asset, not
    // fail the whole publication (the Guide's own `assets/` sentence did).
    #[test]
    fn prose_after_assets_prefix_does_not_fail_publication_assets() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::create_dir_all(root.join("journals")).unwrap();
        std::fs::create_dir_all(root.join("assets")).unwrap();
        std::fs::write(root.join("assets/pic.png"), b"png").unwrap();
        let prose = "x".repeat(300);
        std::fs::write(
            root.join("pages/a.md"),
            format!("- Files in `assets/ {prose}` are watched.\n- ![p](../assets/pic.png)\n"),
        )
        .unwrap();
        let (store, _, _) = Store::open(root, Default::default()).unwrap();
        let corpus = store.whole_graph().unwrap().corpus(None, true);
        let assets =
            publication_assets(&store, &corpus, 32 * 1024 * 1024, &mut Vec::new()).unwrap();
        let names: Vec<_> = assets.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(names, ["assets/pic.png"]);
        store.close();
    }
    #[test]
    fn snapshot_uses_held_reviewed_sources_after_an_external_edit() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join("pages")).unwrap();
        std::fs::write(
            temp.path().join("pages/Public.md"),
            "public:: true\n- TODO reviewed\n",
        )
        .unwrap();
        let store = Store::open(temp.path(), Default::default()).unwrap().0;
        let request = QueryExportRequest {
            argument: "(task TODO)".into(),
            dialect: QueryTextDialect::MacroQuery,
            properties: vec![],
            current_page: None,
            name: "Export".into(),
            host_block_id: None,
            folder: None,
            replace: false,
            asset_budget_bytes: None,
            include_query_text: false,
            include_comments: None,
        };
        let graph = store.whole_graph().unwrap();
        let reviewed = resolve_plan(&store, &graph, &request).unwrap();
        std::fs::write(
            temp.path().join("pages/Public.md"),
            "public:: true\n- TODO unreviewed\n",
        )
        .unwrap();
        store.refresh(tine_store::Depth::Stamps).unwrap();
        let bytes = crate::build_graph::with_build_graph(
            &store,
            &reviewed.selected.pages,
            temp.path(),
            BUILD_GRAPH_REFUSED,
            |build, meta, corpus| snapshot(build, meta, &corpus, "Export", "Public", None, 0),
        )
        .unwrap();
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["pages"][0]["blocks"][0]["raw"], "TODO reviewed", "I-20: all publication projections use the held reviewed corpus; exemplar publish_query::snapshot");
        let repeated = resolve_plan(&store, &graph, &request).unwrap();
        assert_eq!(
            reviewed.plan.fingerprint, repeated.plan.fingerprint,
            "a held plan must fingerprint its held sources, not live disk bytes"
        );
    }
}
