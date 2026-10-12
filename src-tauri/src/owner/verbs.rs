//! The slice-1 read verbs (CLI PROTOCOL §5), shared by both owners. Each is
//! a thin call into the function the app's own command calls
//! (`specs/campaigns/2026-10-cli/notes/RESEARCH-2.md`), except page text,
//! which is the page host's read: a held page's buffer, so the command line
//! sees what the window shows, unsaved text included.

use serde::Deserialize;
use serde_json::{json, Value};
use tine_core::date::{JournalDate, JournalFormat};
use tine_core::query::wire_parse::QueryTextDialect;
use tine_store::{Day, HostReadError, InventoryScope, PageId, Resolved, StoreError, WholeGraph};

use super::endpoint::busy;
use super::wire::{ok, ErrorBody, Request};
use crate::state::GraphSlot;

/// Answer `request` against `slot`'s graph and page host.
pub(crate) fn answer(slot: &GraphSlot, request: &Request) -> Result<Value, ErrorBody> {
    let args = &request.args;
    match request.verb.as_str() {
        "page.show" => {
            let PageArgs { name } = parse(args)?;
            let graph = whole_graph(slot)?;
            page(slot, resolve_page(&graph, &name)?, "page")
        }
        "journal" => {
            let JournalArgs { date } = parse(args)?;
            let day = journal_day(slot, date.as_deref())?;
            let id = slot.store.journal_id(Day(day.ordinal_key()));
            page(slot, id, "journal")
        }
        "block.show" => {
            let BlockArgs { uuid, max_nodes } = parse(args)?;
            check_uuid(&uuid)?;
            let graph = whole_graph(slot)?;
            let max_nodes = max_nodes.unwrap_or(2000).clamp(1, 2000);
            match graph.preview_block(&uuid, max_nodes) {
                Ok(Some(preview)) => Ok(ok("block", json!({ "block": preview }))),
                Ok(None) => Err(not_found(format!("no block has id:: {uuid}"))),
                Err(error) => Err(read_failed(
                    crate::commands::query_error_wire::reference_error(error),
                )),
            }
        }
        "refs" => {
            let RefsArgs { target, block } = parse(args)?;
            let graph = whole_graph(slot)?;
            let groups = if block {
                check_uuid(&target)?;
                graph.block_referrers(&target)
            } else {
                graph.backlinks(&target)
            }
            .map_err(|e| read_failed(crate::commands::query_error_wire::reference_error(e)))?;
            Ok(ok(
                "refs",
                json!({ "target": target, "block": block, "groups": &*groups }),
            ))
        }
        "search" => {
            let SearchArgs {
                text,
                pages,
                blocks,
            } = parse(args)?;
            let execution = tine_graph_features::search::run_graph_search(
                &slot.store,
                &slot.block_search_lanes,
                text,
                pages.unwrap_or(20).min(500),
                blocks.unwrap_or(50).min(5000),
                None,
                false,
                None,
                None,
                None,
                None,
            )
            .map_err(|e| read_failed(crate::commands::feature_search_error(e)))?;
            Ok(ok("search", to_value(&execution)?))
        }
        "query" => {
            let QueryArgs { text, tql } = parse(args)?;
            let graph = whole_graph(slot)?;
            // `MacroQuery` is what a `{{query …}}` holds: OG simple or
            // advanced, decided by the one Rust discriminator.
            let dialect = if tql {
                QueryTextDialect::MacroTql
            } else {
                QueryTextDialect::MacroQuery
            };
            let parsed = crate::query_ir::parse(&graph, &text, dialect, &[])
                .map_err(|e| ErrorBody::new("bad-args", e, false))?;
            if parsed.query.is_invalid() {
                let mut body = ErrorBody::new("bad-args", "the query does not parse", false);
                body.detail = Some(json!({
                    "admitted": false,
                    "diagnostics": parsed.query.diagnostics,
                }));
                return Err(body);
            }
            let result = crate::query_ir::run(
                &graph,
                &parsed.query,
                &parsed.view,
                &tine_core::query::ir::ExecutionContext::none(),
            )
            .map_err(read_failed)?;
            Ok(ok("query", to_value(&result)?))
        }
        "pages" => {
            let PagesArgs { namespace } = parse(args)?;
            let graph = whole_graph(slot)?;
            let key = namespace.as_deref().map(tine_core::refs::page_key);
            let scope = match &key {
                Some(key) => InventoryScope::FilesAtOrUnder(key),
                None => InventoryScope::All,
            };
            let inventory = graph.inventory(scope);
            let pages: Vec<Value> = inventory
                .0
                .iter()
                .filter_map(|entry| match &entry.target {
                    Resolved::Existing { id, .. } => Some(json!({
                        "name": entry.name,
                        "id": id.as_str(),
                        "journal": entry.is_journal,
                    })),
                    Resolved::Alias { .. } | Resolved::Absent { .. } => None,
                })
                .collect();
            Ok(ok(
                "pages",
                json!({ "namespace": namespace, "pages": pages }),
            ))
        }
        other => Err(ErrorBody::new(
            "unknown-verb",
            format!("this owner does not know `{other}`; it may be an older Tine"),
            false,
        )),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PageArgs {
    name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JournalArgs {
    #[serde(default)]
    date: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BlockArgs {
    uuid: String,
    #[serde(default)]
    max_nodes: Option<usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RefsArgs {
    target: String,
    #[serde(default)]
    block: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchArgs {
    text: String,
    #[serde(default)]
    pages: Option<usize>,
    #[serde(default)]
    blocks: Option<usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QueryArgs {
    text: String,
    #[serde(default)]
    tql: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PagesArgs {
    #[serde(default)]
    namespace: Option<String>,
}

fn parse<T: serde::de::DeserializeOwned>(args: &Value) -> Result<T, ErrorBody> {
    let args = if args.is_null() {
        json!({})
    } else {
        args.clone()
    };
    serde_json::from_value(args).map_err(|e| ErrorBody::new("bad-args", e.to_string(), false))
}

fn to_value(value: &impl serde::Serialize) -> Result<Value, ErrorBody> {
    serde_json::to_value(value).map_err(|e| internal(e.to_string()))
}

fn not_found(message: impl Into<String>) -> ErrorBody {
    ErrorBody::new("not-found", message, true)
}

fn internal(message: impl Into<String>) -> ErrorBody {
    ErrorBody::new("internal", message, true)
}

/// A read's failure: an answer over a result budget is `too-large`.
fn read_failed(message: String) -> ErrorBody {
    if message.starts_with("result-too-large") {
        ErrorBody::new("too-large", message, true)
    } else {
        internal(message)
    }
}

fn whole_graph(slot: &GraphSlot) -> Result<WholeGraph, ErrorBody> {
    slot.store
        .whole_graph()
        .map_err(|e| internal(format!("graph load failed: {e:?}")))
}

/// A persisted `id::` value (A2): runtime block ids change every open.
fn check_uuid(uuid: &str) -> Result<(), ErrorBody> {
    let groups: Vec<&str> = uuid.split('-').collect();
    let shaped = groups.iter().map(|g| g.len()).eq([8, 4, 4, 4, 12])
        && groups
            .iter()
            .all(|g| g.bytes().all(|b| b.is_ascii_hexdigit()));
    if shaped {
        Ok(())
    } else {
        Err(ErrorBody::new(
            "bad-args",
            format!("`{uuid}` is not a block's id:: UUID"),
            false,
        ))
    }
}

/// The page a name names, as the app resolves it: a file claimant, else a
/// sole alias owner, else a journal title.
fn resolve_page(graph: &WholeGraph, name: &str) -> Result<PageId, ErrorBody> {
    match graph.resolve(name, false) {
        Resolved::Existing { id, .. } => return Ok(id),
        Resolved::Alias { mut owners } if owners.len() == 1 => return Ok(owners.remove(0)),
        Resolved::Alias { owners } => {
            let mut body = ErrorBody::new(
                "ambiguous",
                format!("`{name}` is an alias of {} pages", owners.len()),
                true,
            );
            body.detail = Some(json!({
                "admitted": true,
                "candidates": owners.iter().map(PageId::as_str).collect::<Vec<_>>(),
            }));
            return Err(body);
        }
        Resolved::Absent { .. } => {}
    }
    match graph.resolve(name, true) {
        Resolved::Existing { id, .. } => Ok(id),
        _ => Err(not_found(format!("no page is named `{name}`"))),
    }
}

/// The journal day `date` names: today when absent, else `YYYY-MM-DD` (or
/// `YYYY_MM_DD`), else a title in the graph's journal format.
fn journal_day(slot: &GraphSlot, date: Option<&str>) -> Result<JournalDate, ErrorBody> {
    let Some(date) = date.map(str::trim) else {
        return Ok(JournalDate::today());
    };
    if date == "today" {
        return Ok(JournalDate::today());
    }
    if let Some(day) = JournalDate::from_file_stem(date) {
        return Ok(day);
    }
    let config = slot.store.config();
    JournalFormat::new(
        config.journal_file_name_format.as_deref(),
        config.journal_page_title_format.as_deref(),
    )
    .parse(date)
    .ok_or_else(|| {
        ErrorBody::new(
            "bad-args",
            format!("`{date}` is not a date (use YYYY-MM-DD)"),
            false,
        )
    })
}

/// Page `id` as the page host sees it: a held page's buffer, else its file.
fn page(slot: &GraphSlot, id: PageId, kind: &str) -> Result<Value, ErrorBody> {
    let host = slot.host_slot().map_err(|_| busy())?;
    let Some(host) = host.running() else {
        return Err(busy());
    };
    let read = host.read(&id).map_err(|error| match error {
        HostReadError::Ended => busy(),
        HostReadError::Store(error) => store_failed(error),
    })?;
    let Some(bytes) = read.bytes else {
        return Err(not_found(format!("{} does not exist", id.as_str())));
    };
    let parsed = slot
        .store
        .page_of(&read.page, &bytes)
        .map_err(store_failed)?;
    let revision = read
        .revision
        .map(|(incarnation, version)| json!({ "incarnation": incarnation, "version": version }));
    Ok(ok(
        kind,
        json!({
            "id": parsed.id.as_str(),
            "rev": String::from(parsed.rev),
            "revision": revision,
            "read_only": parsed.read_only,
            "text": String::from_utf8_lossy(&bytes),
            "page": parsed.doc,
        }),
    ))
}

fn store_failed(error: StoreError) -> ErrorBody {
    match error {
        StoreError::TooLarge { limit, len } => ErrorBody::new(
            "too-large",
            format!("the page is {len} bytes, over the {limit}-byte parse limit"),
            true,
        ),
        StoreError::NotFound => not_found("the page does not exist"),
        StoreError::Closed => busy(),
        other => internal(format!("{other:?}")),
    }
}
