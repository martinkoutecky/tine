//! Reviewed query and live publication. Selection and query answers come from
//! one `WholeGraph` view. The caller chooses an external parent; `Store`
//! stages, fsyncs and installs one create-only leaf. The module never writes
//! source pages, knows the storage layout, or runs a second query evaluator.
//!
//! `plan_query` costs O(P + selected page bytes + query evaluation) and returns
//! a fingerprint over the reviewed membership and source revisions. `publish_query`
//! repeats that work and refuses a changed plan. `publish_live` costs O(P + B)
//! and exports public pages, or all pages on explicit request. Observable
//! failures are parser/selection refusal, output budget, stale plan and I/O;
//! callers show them and let the user pick a fresh destination.

use crate::render::{self, RenderGraph};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::io;
use std::path::Path;
use tine_core::model::PageKind;
use tine_core::query::ir::{ExecutionContext, QueryResult, QueryRows};
use tine_core::query::macro_text::query_macro_extents;
use tine_core::query::wire_parse::{
    anchored_view, parse_query_pair, ParsedQuery, QueryTextDialect,
};
use tine_store::Resolved;
use tine_store::{IrAnswer, IrRequest, Store, WholeGraph};

const MAX_PAGES: usize = 20_000;
const MAX_EXPORT_BYTES: usize = 128 * 1024 * 1024;

fn export_time() -> io::Result<String> {
    let seconds = match std::env::var("SOURCE_DATE_EPOCH") {
        Ok(raw) => raw
            .parse::<i64>()
            .map_err(|_| refusal("invalid SOURCE_DATE_EPOCH"))?,
        Err(_) => std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_secs() as i64,
    };
    time::OffsetDateTime::from_unix_timestamp(seconds)
        .map_err(io::Error::other)?
        .format(&time::format_description::well_known::Rfc3339)
        .map_err(io::Error::other)
}

/// The same raw argument, dialect and host properties passed to `query_parse`.
/// A name chooses a portable leaf under the user-selected parent; a current
/// page binds advanced inputs and the exported query's home run.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryExportRequest {
    pub argument: String,
    pub dialect: QueryTextDialect,
    #[serde(default)]
    pub properties: Vec<(String, String)>,
    #[serde(default)]
    pub current_page: Option<String>,
    pub name: String,
    #[serde(default)]
    pub host_block_id: Option<String>,
}

/// One complete owner page in a reviewed query selection.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportPage {
    pub name: String,
    pub path: String,
    pub journal: bool,
}

/// Stateless plan shown before publication. `fingerprint` binds selected
/// source revisions, query input and result membership; no server-side session.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryExportPlan {
    pub anchor: String,
    pub row_count: usize,
    pub pages: Vec<ExportPage>,
    pub folder: String,
    pub fingerprint: String,
}

/// Published destination and number of selected source pages.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportReceipt {
    pub path: String,
    pub pages: usize,
    pub files: u64,
}

fn refusal(message: impl Into<String>) -> io::Error {
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
        .query_ir(IrRequest::Registry)
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
        .query_ir(IrRequest::Run {
            query: &parsed.query,
            view: &view,
            context,
        })
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

struct Planned {
    plan: QueryExportPlan,
    parsed: ParsedQuery,
    result: QueryResult,
    selected: tine_core::Corpus,
}

fn resolve_plan(
    store: &Store,
    graph: &WholeGraph,
    request: &QueryExportRequest,
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
    let corpus = graph.corpus();
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
                let matches: Vec<_> = corpus
                    .pages
                    .iter()
                    .filter(|p| p.kind == group.kind && p.name.eq_ignore_ascii_case(&group.page))
                    .collect();
                if matches.len() != 1 {
                    return Err(refusal("query result has an ambiguous page owner"));
                }
                paths.insert(matches[0].id.as_str().to_owned());
            }
            "block"
        }
    };
    if paths.len() > MAX_PAGES {
        return Err(refusal("query selects too many pages"));
    }
    let mut selected = tine_core::Corpus {
        pages: corpus
            .pages
            .into_iter()
            .filter(|page| paths.contains(page.id.as_str()))
            .collect(),
    };
    if selected.pages.len() != paths.len() {
        return Err(refusal("query result owner is missing"));
    }
    selected
        .pages
        .sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));
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
    hasher.update(serde_json::to_vec(request).map_err(io::Error::other)?);
    hasher.update(serde_json::to_vec(&result.rows).map_err(io::Error::other)?);
    for page in &selected.pages {
        let read = store.page(&page.id).map_err(crate::store_error)?;
        hasher.update(page.id.as_str().as_bytes());
        hasher.update(serde_json::to_vec(&read.rev).map_err(io::Error::other)?);
    }
    let fingerprint = format!("{:x}", hasher.finalize());
    Ok(Planned {
        plan: QueryExportPlan {
            anchor: anchor.into(),
            row_count: result.total,
            pages,
            folder: slug(&request.name),
            fingerprint,
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
    Ok(resolve_plan(store, &graph, request)?.plan)
}

fn collect_static(
    store: &Store,
    graph: &WholeGraph,
    corpus: &tine_core::Corpus,
) -> io::Result<Vec<(String, Vec<u8>)>> {
    let config = store.config();
    let render_graph = RenderGraph {
        corpus,
        whole: graph,
        store,
    };
    let mut files = Vec::new();
    let mut used = 0usize;
    render::publish_graph(
        &render_graph,
        true,
        &config.favorites,
        &mut |name, bytes| {
            used = used
                .checked_add(bytes.len())
                .ok_or_else(|| refusal("export byte budget exceeded"))?;
            if used > MAX_EXPORT_BYTES {
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
    files.extend(tine_store::publication_assets(store, corpus).map_err(crate::store_error)?);
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
            groups.retain(|group| names.contains(&(group.kind, group.page.to_lowercase())))
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

fn substitute_current_page(argument: &str, page: &str) -> Option<String> {
    let mut changed = false;
    let mut out = String::new();
    let mut rest = argument;
    while let Some(start) = rest.find("<%") {
        let Some(end) = rest[start..].find("%>") else {
            break;
        };
        let inner = &rest[start + 2..start + end];
        if inner.trim().eq_ignore_ascii_case("current page") {
            out.push_str(&rest[..start]);
            out.push_str(&format!("[[{page}]]"));
            changed = true;
        } else {
            out.push_str(&rest[..start + end + 2]);
        }
        rest = &rest[start + end + 2..];
    }
    if !changed {
        return None;
    }
    out.push_str(rest);
    Some(out)
}

fn baked_queries(graph: &WholeGraph, corpus: &tine_core::Corpus) -> io::Result<Vec<Value>> {
    let paths: HashSet<_> = corpus.pages.iter().map(|page| page.id.as_str()).collect();
    let names: HashSet<_> = corpus
        .pages
        .iter()
        .map(|page| (page.kind, page.name.to_lowercase()))
        .collect();
    let mut records = Vec::new();
    let registry = ir_registry(graph)?;
    for page in &corpus.pages {
        let mut stack: Vec<_> = page.document.roots.iter().collect();
        while let Some(block) = stack.pop() {
            stack.extend(block.children.iter());
            // The raw-extent reader is quadratic with many malformed starts;
            // bound its input before calling it (I-22).
            if block.raw().len() > tine_core::query::QUERY_SOURCE_MAX_BYTES {
                continue;
            }
            for extent in query_macro_extents(block.raw()) {
                if records.len() >= 256 {
                    return Err(refusal("too many queries in published pages"));
                }
                let dialect = if extent.name.eq_ignore_ascii_case("tine-query") {
                    QueryTextDialect::MacroTql
                } else {
                    QueryTextDialect::MacroQuery
                };
                let properties: Vec<_> = block
                    .properties()
                    .into_iter()
                    .filter(|(key, _)| key.starts_with("tine."))
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
        }
    }
    Ok(records)
}

fn snapshot(
    store: &Store,
    graph: &WholeGraph,
    corpus: &tine_core::Corpus,
    name: &str,
    home: &str,
    home_query: Option<(&QueryExportRequest, &ParsedQuery, &QueryResult)>,
) -> io::Result<Vec<u8>> {
    let mut pages: Vec<Value> = Vec::new();
    let mut entries: Vec<Value> = Vec::new();
    let selected: HashSet<_> = corpus
        .pages
        .iter()
        .map(|p| (p.kind, p.name.to_lowercase()))
        .collect();
    let selected_paths: HashSet<_> = corpus.pages.iter().map(|p| p.id.as_str()).collect();
    let inventory = graph.inventory();
    for page in &corpus.pages {
        let mut read = store.page(&page.id).map_err(crate::store_error)?.doc;
        read.read_only = true;
        let mut value = serde_json::to_value(read).map_err(io::Error::other)?;
        value["path"] = json!(page.id.as_str());
        value["rev"] = Value::Null;
        value["activation"] = Value::Null;
        pages.push(value);
        let day = inventory.0.iter().find_map(|entry| match &entry.target {
            Resolved::Existing { id, .. } if id == &page.id => entry.day.map(|d| d.0),
            _ => None,
        });
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
            if key.starts_with("tine.") && !key.contains('\n') && !value.contains('\n') {
                raw.push_str(&format!("\n  {key}:: {}", value.trim()));
            }
        }
        let block_id = format!("published-query:{}", slug(home));
        let mut query_block = json!({ "id": block_id, "raw": raw,
            "collapsed": false, "children": [], "breadcrumb": [] });
        if !request.properties.is_empty() {
            query_block["properties"] = json!(request.properties);
        }
        let mut home_blocks = vec![query_block];
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
        let executed_context = ExecutionContext {
            current_page: request.current_page.clone(),
        };
        let view = anchored_view(parsed, parsed.query.anchor);
        queries.push(
            json!({ "host": home, "argument": request.argument, "dialect": request.dialect,
            "properties": request.properties, "parsed": parsed, "context": context,
            "executed_context": executed_context, "view": view, "result": result }),
        );
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
            .filter(|g| selected.contains(&(g.kind, g.page.to_lowercase())))
            .cloned()
            .collect();
        if !closed.is_empty() {
            backlinks.insert(page.name.clone(), closed);
        }
    }
    let names: Vec<_> = corpus.pages.iter().map(|p| p.name.clone()).collect();
    let icons = graph.page_icons(&names);
    let aliases: Vec<_> = inventory
        .0
        .iter()
        .filter_map(|entry| match &entry.target {
            Resolved::Alias { owners }
                if owners.len() == 1 && selected_paths.contains(owners[0].as_str()) =>
            {
                corpus
                    .pages
                    .iter()
                    .find(|page| page.id == owners[0])
                    .map(|page| (entry.name.clone(), page.name.clone()))
            }
            _ => None,
        })
        .collect();
    let block_ref_counts = tine_store::publication_block_ref_counts(store, corpus);
    let snapshot = json!({ "schema": 1, "name": name, "exported_at": export_time()?,
        "home": home, "pages": pages, "entries": entries, "backlinks": backlinks,
        "block_ref_counts": block_ref_counts, "aliases": aliases, "icons": icons, "queries": queries });
    let bytes = serde_json::to_vec(&snapshot).map_err(io::Error::other)?;
    if bytes.len() > MAX_EXPORT_BYTES {
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
    let receipt =
        tine_store::publish_site_external(store, parent.as_os_str(), leaf, &mut |writer| {
            for (path, bytes) in &files {
                writer.write(path, bytes)?;
            }
            Ok(())
        })
        .map_err(|failure| io::Error::new(failure.cause.kind, failure.cause.message))?;
    Ok(ExportReceipt {
        path: receipt.site.display().to_string(),
        pages,
        files: receipt.files,
    })
}

/// Commit a reviewed query to an external user-picked parent. It re-runs the
/// plan and refuses changed membership or content before creating output.
/// The resulting folder contains static HTML and the read-only browser app.
pub fn publish_query(
    store: &Store,
    request: &QueryExportRequest,
    fingerprint: &str,
    parent: &Path,
    bundle: &[(String, Vec<u8>)],
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
    let mut files = collect_static(store, &graph, &planned.selected)?;
    let mut taken: HashSet<_> = planned
        .selected
        .pages
        .iter()
        .map(|p| p.name.to_lowercase())
        .collect();
    let mut home = request.name.trim().to_owned();
    if taken.contains(&home.to_lowercase()) {
        home.push_str(" (export)");
    }
    while !taken.insert(home.to_lowercase()) {
        home.push_str(" 2");
    }
    let snap = snapshot(
        store,
        &graph,
        &planned.selected,
        &request.name,
        &home,
        Some((request, &planned.parsed, &planned.result)),
    )?;
    app_files(&mut files, bundle, snap, &request.name)?;
    commit(
        store,
        parent,
        &planned.plan.folder,
        files,
        planned.selected.pages.len(),
    )
}

/// Publish a whole-graph read-only browser app, with the static site as a
/// fallback. The caller explicitly selects `all_pages`; otherwise only pages
/// with `public:: true` are included. Output is create-only outside the graph.
pub fn publish_live(
    store: &Store,
    parent: &Path,
    name: &str,
    all_pages: bool,
    bundle: &[(String, Vec<u8>)],
) -> io::Result<ExportReceipt> {
    if name.trim().is_empty() || name.len() > 256 {
        return Err(refusal("live export name is invalid"));
    }
    let graph = store
        .whole_graph()
        .map_err(|e| io::Error::other(format!("graph load failed: {e:?}")))?;
    let mut corpus = graph.corpus();
    if !all_pages {
        corpus
            .pages
            .retain(|p| render::page_is_public(p.document.pre_block.as_deref()));
    }
    if corpus.pages.len() > MAX_PAGES {
        return Err(refusal("live export selects too many pages"));
    }
    corpus.pages.sort_by(|a, b| a.name.cmp(&b.name));
    let home = corpus
        .pages
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case("Welcome to Tine"))
        .or_else(|| corpus.pages.first())
        .map(|p| p.name.clone())
        .unwrap_or_default();
    let mut files = collect_static(store, &graph, &corpus)?;
    let snap = snapshot(store, &graph, &corpus, name, &home, None)?;
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
) -> io::Result<ExportReceipt> {
    if name.trim().is_empty() || name.len() > 256 {
        return Err(refusal("static export name is invalid"));
    }
    let graph = store
        .whole_graph()
        .map_err(|e| io::Error::other(format!("graph load failed: {e:?}")))?;
    let mut corpus = graph.corpus();
    if !all_pages {
        corpus
            .pages
            .retain(|p| render::page_is_public(p.document.pre_block.as_deref()));
    }
    if corpus.pages.len() > MAX_PAGES {
        return Err(refusal("static export selects too many pages"));
    }
    corpus.pages.sort_by(|a, b| a.name.cmp(&b.name));
    let files = collect_static(store, &graph, &corpus)?;
    commit(store, parent, &slug(name), files, corpus.pages.len())
}
