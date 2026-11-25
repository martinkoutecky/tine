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

use crate::publication_snapshot::{self, HomePage, Snapshot};
use crate::render::{self, RenderGraph, SheetExport, SheetIndex};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::io;
use std::path::Path;
use tine_core::model::PageKind;
use tine_core::query::ir::{ExecutionContext, QueryResult, QueryRows};
use tine_core::query::wire_parse::{
    anchored_view, bind_current_page, parse_query_pair, QueryTextDialect,
};
use tine_core::refs::page_key;
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

mod source;
pub use source::{query_request_by_uuid, query_request_inline, query_request_on_page};

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
    request: &QueryExportRequest,
) -> io::Result<(QueryResult, bool)> {
    let argument = &request.argument;
    if !tine_core::query::query_source_within_limit(argument)
        || !tine_core::query::query_nesting_within_limit(argument)
    {
        return Err(refusal("query exceeds the export parser limit"));
    }
    let registry = ir_registry(graph)?;
    let parse =
        |source: &str| parse_query_pair(source, request.dialect, &request.properties, &registry);
    let mut parsed = parse(argument);
    let uses_current_page = parsed.uses_current_page();
    if let Some(page) = request
        .textual_current_page
        .as_deref()
        .filter(|_| !parsed.current_page_tokens.is_empty())
    {
        let execution = bind_current_page(argument, page);
        if crate::query_wire::validate_query_source(&execution).is_err() {
            return Err(refusal("query exceeds the export parser limit"));
        }
        parsed = parse(&execution);
    }
    let anchor = parsed.query.anchor;
    let view = anchored_view(&parsed, anchor);
    let result = match graph
        .query_ir(
            IrRequest::Run {
                query: &parsed.query,
                view: &view,
                context: &ExecutionContext {
                    current_page: request.current_page.clone(),
                },
            },
            &tine_store::Cancel::default(),
        )
        .map(|answer| answer.expect("never cancelled"))
        .map_err(|e| io::Error::other(e.to_string()))?
    {
        IrAnswer::Result(result) => *result,
        _ => return Err(io::Error::other("query run returned another answer")),
    };
    if result.exceeded || result.total > MAX_PAGES * 32 {
        return Err(refusal("query export exceeds the bounded result limit"));
    }
    if !result.report.supported || result.diagnostics.iter().any(|d| !d.disabled) {
        return Err(refusal("this query is not fully supported for export"));
    }
    Ok((result, uses_current_page))
}

pub(crate) struct Planned {
    pub(crate) plan: QueryExportPlan,
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
            .into_iter()
            .chain(request.textual_current_page.as_ref())
            .any(|page| page.len() > 512)
        || request
            .host_block_id
            .as_ref()
            .is_some_and(|id| id.len() > 128)
    {
        return Err(refusal("query export metadata exceeds its limit"));
    }
    let (mut result, uses_current_page) = parse_and_run(graph, request)?;
    if let (Some(host), QueryRows::Block { groups }) = (&request.host_block_id, &mut result.rows) {
        for group in groups.iter_mut() {
            group.blocks.retain(|block| &block.id != host);
        }
        groups.retain(|group| !group.blocks.is_empty());
        result.total = groups.iter().map(|group| group.blocks.len()).sum();
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
    reviewed.include_comments = Some(include_comments);
    hasher.update(serde_json::to_vec(&reviewed).map_err(io::Error::other)?);
    hasher
        .update(serde_json::to_vec(&(result.total, anchor, &left_out)).map_err(io::Error::other)?);
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
            uses_current_page,
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
    #[serde(untagged)]
    enum MembershipSnapshot<'a> {
        Sources(#[serde(borrow)] Snapshot<'a>),
        Pages {
            schema: Option<u32>,
            pages: Vec<MembershipPage>,
        },
    }
    match serde_json::from_slice(&bytes?).ok()? {
        MembershipSnapshot::Sources(snapshot) => snapshot.query_membership().ok(),
        MembershipSnapshot::Pages {
            schema: None | Some(1),
            pages,
        } => Some(
            pages
                .into_iter()
                .filter(|page| !page.path.is_empty())
                .map(|page| page.name)
                .collect(),
        ),
        MembershipSnapshot::Pages { .. } => None,
    }
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
    let config = &graph.config().config;
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

fn query_publication_property(key: &str) -> bool {
    key.starts_with("tine.")
        || matches!(
            key,
            "query-table" | "query-properties" | "query-sort-by" | "query-sort-desc"
        )
}

fn home_query(request: &QueryExportRequest) -> String {
    let macro_name = match request.dialect {
        QueryTextDialect::MacroTql | QueryTextDialect::Tql => "tine-query",
        _ => "query",
    };
    let argument = request.argument.trim().replace('\n', "\n  ");
    let mut raw = format!("{{{{{macro_name} {argument}}}}}");
    for (key, value) in &request.properties {
        if query_publication_property(key)
            && !key.contains(['\n', '\r'])
            && !value.contains(['\n', '\r'])
        {
            raw.push_str(&format!("\n  {key}:: {}", value.trim()));
        }
    }
    raw
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
        "<meta http-equiv=\"Content-Security-Policy\" content=\"worker-src 'self'\">\
         <meta name=\"tine-published\" content=\"snapshot.json\">",
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

/// The asset names publication copies: the collector's targets as the owner
/// named them (never decoded again), without a folder that another copied
/// name lies under. Pure; O(names × path depth).
pub(crate) fn publication_asset_names(mut names: HashSet<String>) -> Vec<String> {
    // A link to a folder (`assets/sub/`) names no file. Publication copies
    // files, so drop a name that another copied name lies under.
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
    names
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
    let names = publication_asset_names(corpus.asset_refs());
    let mut out = Vec::new();
    let mut remaining = budget;
    for name in names {
        // A name the owner gives may still be no file the store can name
        // (a `..` step after the `assets` folder). Such a name is simply not
        // an asset: skip it like a missing one instead of failing the whole
        // publication. A real read failure of an asset still fails.
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
    let epoch = export_epoch()?;
    publish_at(store, request, fingerprint, bundle, sheets, epoch)
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
    publish_at(store, request, fingerprint, bundle, Vec::new(), epoch)
}

fn publish_at(
    store: &Store,
    request: &QueryExportRequest,
    fingerprint: &str,
    bundle: &[(String, Vec<u8>)],
    sheets: Vec<SheetExport>,
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
    let mut warnings = Vec::new();
    // Every emitted file (the static HTML fallback and the snapshot) is
    // rendered from the private build graph: an unexported page can change no
    // byte of it (closure, `docs/contracts/quick-share.md`). Asset bytes are
    // still read from the real store.
    let (mut files, snap) = crate::build_graph::with_build_graph_text(
        &graph,
        &planned.selected.pages,
        BUILD_GRAPH_REFUSED,
        |build, corpus, sources| {
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
            let home = match HomePage::new(build, &corpus.pages, request.name.trim()) {
                Ok(home) => home,
                Err(name) => {
                    let mut candidate = format!("{name} (export)");
                    loop {
                        match HomePage::new(build, &corpus.pages, &candidate) {
                            Ok(home) => break home,
                            Err(name) => candidate = format!("{name} 2"),
                        }
                    }
                }
            };
            let query = request.include_query_text.then(|| home_query(request));
            let snap = publication_snapshot::snapshot(
                &graph,
                &corpus,
                sources,
                &request.name,
                &export_time(epoch)?,
                &home.name,
                Some(home.source(&corpus.pages, query.as_deref())?),
                false,
                &mut warnings,
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
    live(
        store,
        parent,
        name,
        all_pages,
        include_comments,
        home,
        bundle,
        Vec::new(),
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
    live(
        store,
        parent,
        name,
        all_pages,
        include_comments,
        None,
        bundle,
        sheets,
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
            None => (graph.config().config.default_home)
                .as_deref()
                .and_then(|configured| find(configured))
                .or_else(|| find("Welcome to Tine"))
                .or_else(|| corpus.pages.first()),
        }
        .map(|p| p.name.clone())
        .unwrap_or_default();
    // The static fallback and the snapshot both come from the build graph
    // (see `publish_at`).
    let mut warnings = Vec::new();
    let (mut files, snap) = crate::build_graph::with_build_graph_text(
        &graph,
        &corpus.pages,
        BUILD_GRAPH_REFUSED,
        |build, corpus, sources| {
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
                publication_snapshot::snapshot(
                    &graph,
                    &corpus,
                    sources,
                    name,
                    &export_time(export_epoch()?)?,
                    &home,
                    None,
                    false,
                    &mut warnings,
                )?,
            ))
        },
    )?;
    app_files(&mut files, bundle, snap, name)?;
    let mut receipt = commit(store, parent, &slug(name), files, corpus.pages.len())?;
    receipt.warnings.extend(warnings);
    Ok(receipt)
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
    // `publish_at`).
    let files = crate::build_graph::with_build_graph(
        &graph,
        &corpus.pages,
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

    // Prose after `assets/` keeps files alive (the collector's loose mentions,
    // so orphan detection cannot miss a reference) but names no link, so
    // publication copies only the linked file and never fails on the prose
    // (the Guide's own `assets/` sentence once did).
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
        std::fs::create_dir_all(temp.path().join("logseq")).unwrap();
        let config = temp.path().join("logseq/config.edn");
        std::fs::write(&config, "{:start-of-week 1 :favorites [\"Private\"]}").unwrap();
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
            textual_current_page: None,
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
        std::fs::write(config, "{:start-of-week 4}").unwrap();
        store.refresh(tine_store::Depth::Stamps).unwrap();
        let bytes = crate::build_graph::with_build_graph_text(
            &graph,
            &reviewed.selected.pages,
            BUILD_GRAPH_REFUSED,
            |_, corpus, sources| {
                publication_snapshot::snapshot(
                    &graph,
                    &corpus,
                    sources,
                    "Export",
                    &export_time(0)?,
                    "Public",
                    None,
                    false,
                    &mut Vec::new(),
                )
            },
        )
        .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            value["files"][0]["text"], "public:: true\n\n- TODO reviewed\n",
            "I-20: publication uses held reviewed sources"
        );
        let reduced = value["files"][1]["text"].as_str().unwrap();
        assert_eq!(tine_core::Config::parse(reduced).start_of_week, 1);
        assert!(!reduced.contains("Private"));
        let repeated = resolve_plan(&store, &graph, &request).unwrap();
        assert_eq!(
            reviewed.plan.fingerprint, repeated.plan.fingerprint,
            "a held plan must fingerprint its held sources, not live disk bytes"
        );
    }
}

#[cfg(test)]
#[path = "publish_query_table_tests.rs"]
mod table_tests;
