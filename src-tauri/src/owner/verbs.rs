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

/// The most nodes a `block.show` preview carries; `max_nodes` clamps to it.
/// The human output names it when it reports omitted nodes.
pub(crate) const PREVIEW_NODES: usize = 2000;

/// The named diagnostic ids of an `internal` error (PROTOCOL §2.3: "message
/// names the diag id"). The set is closed: `internal` takes one of these, so a
/// bare-prose internal error cannot be written, and the output contract
/// (`docs/contracts/cli-output.md`) lists exactly [`Diag::ALL`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Diag {
    /// The graph's load failed or the Store closed under a read.
    GraphLoad,
    /// A reply payload could not be encoded as JSON.
    Encode,
    /// A graph read (references, preview, search) failed.
    ReadFailed,
    /// A parsed query failed to run.
    QueryFailed,
    /// The Store failed on a page read in a way no code names.
    StoreFailed,
    /// A write's plan failed on an I/O error, or planned no bytes.
    WriteFailed,
}

impl Diag {
    #[cfg(test)]
    pub(crate) const ALL: [Diag; 6] = [
        Diag::GraphLoad,
        Diag::Encode,
        Diag::ReadFailed,
        Diag::QueryFailed,
        Diag::StoreFailed,
        Diag::WriteFailed,
    ];

    /// The id in the error message, the owner's log and the contract.
    pub(crate) fn id(self) -> &'static str {
        match self {
            Self::GraphLoad => "cli-graph-load",
            Self::Encode => "cli-reply-encode",
            Self::ReadFailed => "cli-read-failed",
            Self::QueryFailed => "cli-query-failed",
            Self::StoreFailed => "cli-store-failed",
            Self::WriteFailed => "cli-write-failed",
        }
    }
}

/// Answer `request` against `slot`'s graph and page host.
pub(crate) fn answer_verb(slot: &GraphSlot, request: &Request) -> Result<Value, ErrorBody> {
    let args = &request.args;
    match request.verb.as_str() {
        "page.show" => {
            let PageArgs { name } = parse(args)?;
            let graph = whole_graph(slot)?;
            page_answer(slot, resolve_page(&graph, &name)?, "page")
        }
        "journal" => {
            let JournalArgs { date } = parse(args)?;
            let day = journal_day(slot, date.as_deref())?;
            let id = slot.store.journal_id(Day(day.ordinal_key()));
            page_answer(slot, id, "journal")
        }
        "block.show" => {
            let BlockArgs { uuid, max_nodes } = parse(args)?;
            check_uuid(&uuid)?;
            let graph = whole_graph(slot)?;
            let max_nodes = max_nodes.unwrap_or(PREVIEW_NODES).clamp(1, PREVIEW_NODES);
            match graph.preview_block(&uuid, max_nodes) {
                Ok(Some(preview)) => {
                    let page = resolve_page(&graph, &preview.group.page)?;
                    // One host read gives the content, positions and
                    // fingerprints together (review 6); the store preview is
                    // the fallback, positions only, when the page is busy.
                    let block = match host_preview(slot, &page, &preview.group, &uuid, max_nodes)? {
                        Some(block) => block,
                        None => to_value(&preview)?,
                    };
                    Ok(ok("block", json!({ "block": block })))
                }
                Ok(None) => Err(not_found(format!("no block has id:: {uuid}"))),
                Err(error) => Err(read_failed(
                    Diag::ReadFailed,
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
            .map_err(|e| {
                read_failed(
                    Diag::ReadFailed,
                    crate::commands::query_error_wire::reference_error(e),
                )
            })?;
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
            .map_err(|e| read_failed(Diag::ReadFailed, crate::commands::feature_search_error(e)))?;
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
            // An advanced (datalog) form carries a provisional parse-time
            // diagnostic that execution-time resolution replaces by its own
            // report (`resolve_for_execution`, SPEC §4.4): it runs, and the
            // answer's `report` and `diagnostics` say what ran and what was
            // ignored. Any other invalid query does not run.
            let advanced = matches!(
                parsed.query.source,
                tine_core::query::ir::Source::Advanced { .. }
            );
            if parsed.query.is_invalid() && !advanced {
                let mut body = ErrorBody::new("bad-args", "the query does not parse", false);
                body.detail = Some(json!({
                    "admitted": false,
                    "diagnostics": parsed.query.diagnostics,
                }));
                return Err(body);
            }
            let run = slot.begin_query_run();
            let result = crate::query_ir::run(
                &graph,
                &parsed.query,
                &parsed.view,
                &tine_core::query::ir::ExecutionContext::none(),
                run.cancel(),
            )
            .map_err(|e| read_failed(Diag::QueryFailed, e))?;
            let mut payload = to_value(&result)?;
            // D-18: the cue for a Logseq form that adds no condition travels
            // with the answer, in both outputs.
            if let (Some(hint), Some(fields)) = (&parsed.og_hint, payload.as_object_mut()) {
                fields.insert("og_hint".into(), og_hint_payload(hint, &parsed.view)?);
            }
            Ok(ok("query", payload))
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
        "block.edit" => in_run(slot, args, "tine edit", || {
            super::write::edit_block(slot, args)
        }),
        "block.prop" => in_run(slot, args, "tine prop", || {
            super::write::prop_block(slot, args)
        }),
        "block.move" => in_run(slot, args, "tine move", || {
            super::write::move_block(slot, args)
        }),
        "block.delete" => in_run(slot, args, "tine delete", || {
            super::write::delete_block(slot, args)
        }),
        "block.add" => in_run(slot, args, "tine add", || super::add::add_block(slot, args)),
        "page.create" => in_run(slot, args, "tine page create", || {
            super::pages::create_named_page(slot, args)
        }),
        "page.rename" => super::pages::rename_named_page(slot, args),
        "page.delete" => in_run(slot, args, "tine page delete", || {
            super::pages::delete_named_page(slot, args)
        }),
        "ref" | "embed" => in_run(slot, args, "tine block id", || {
            super::pages::block_reference(slot, args, request.verb == "embed")
        }),
        #[cfg(desktop)]
        "run.begin" => super::git_run::run_begin(slot),
        #[cfg(desktop)]
        "run.show" => super::git_run::run_show(slot, args),
        #[cfg(desktop)]
        "run.end" => super::git_run::serialized(|| super::git_run::run_end(slot, args)),
        #[cfg(desktop)]
        "run.revert" => super::git_run::serialized(|| super::git_run::run_revert(slot, args)),
        other => Err(ErrorBody::new(
            "unknown-verb",
            format!("this owner does not know `{other}`; it may be an older Tine"),
            false,
        )),
    }
}

/// A write verb, committed to git when its arguments name a run.
fn in_run(
    slot: &GraphSlot,
    args: &Value,
    subject: &str,
    verb: impl FnOnce() -> Result<Value, ErrorBody>,
) -> Result<Value, ErrorBody> {
    #[cfg(desktop)]
    {
        let Some(run) = super::git_run::run_of(args)? else {
            return verb();
        };
        // The save and its commits as one step, in save order (CL1).
        super::git_run::serialized(|| {
            super::git_run::check_repository(slot)?;
            super::write::in_run(Some(run), subject, verb)
        })
    }
    #[cfg(not(desktop))]
    {
        let _ = subject;
        if args
            .get("run")
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty())
        {
            let _ = slot;
            return Err(super::write::refused(
                "git",
                None,
                "git runs need the desktop app",
            ));
        }
        verb()
    }
}

#[derive(Deserialize)]
struct PageArgs {
    name: String,
}

#[derive(Deserialize)]
struct JournalArgs {
    #[serde(default)]
    date: Option<String>,
}

#[derive(Deserialize)]
struct BlockArgs {
    uuid: String,
    #[serde(default)]
    max_nodes: Option<usize>,
}

#[derive(Deserialize)]
struct RefsArgs {
    target: String,
    #[serde(default)]
    block: bool,
}

#[derive(Deserialize)]
struct SearchArgs {
    text: String,
    #[serde(default)]
    pages: Option<usize>,
    #[serde(default)]
    blocks: Option<usize>,
}

#[derive(Deserialize)]
struct QueryArgs {
    text: String,
    #[serde(default)]
    tql: bool,
}

#[derive(Deserialize)]
struct PagesArgs {
    #[serde(default)]
    namespace: Option<String>,
}

pub(super) fn parse<T: serde::de::DeserializeOwned>(args: &Value) -> Result<T, ErrorBody> {
    super::wire::read_object(args.clone()).map_err(|e| ErrorBody::new("bad-args", e, false))
}

fn to_value(value: &impl serde::Serialize) -> Result<Value, ErrorBody> {
    serde_json::to_value(value).map_err(|e| internal(Diag::Encode, e.to_string()))
}

/// `ParsedQuery.og_hint` as the payload carries it: the DTO verbatim, plus
/// each rewrite's `text`, the explicit OG form the existing printer writes
/// for it (`null` when the OG syntax cannot say it). The command line prints
/// rather than re-derives; nothing here decides what a form means.
fn og_hint_payload(
    hint: &tine_core::query::wire_parse::OgQueryHint,
    view: &tine_core::query::ir::ViewSettings,
) -> Result<Value, ErrorBody> {
    use tine_core::query::print::{og_expressible, query_print, PrintDialect};
    let mut value = to_value(hint)?;
    if let Some(rewrites) = value.get_mut("rewrites").and_then(Value::as_array_mut) {
        for (slot, rewrite) in rewrites.iter_mut().zip(&hint.rewrites) {
            let text = og_expressible(&rewrite.query, view)
                .then(|| query_print(&rewrite.query, view, PrintDialect::Og, false).ok())
                .flatten();
            if let Some(slot) = slot.as_object_mut() {
                slot.insert("text".into(), json!(text));
            }
        }
    }
    Ok(value)
}

fn not_found(message: impl Into<String>) -> ErrorBody {
    ErrorBody::new("not-found", message, true)
}

/// An owner bug or an unnamed failure: the message leads with the diag id
/// (which the owner also logs; the detail may hold graph content, so only
/// the id goes to stderr), and `detail.diag` repeats it for scripts.
pub(super) fn internal(diag: Diag, message: impl Into<String>) -> ErrorBody {
    let message = message.into();
    crate::debug::diag_private(diag.id(), &message);
    let mut body = ErrorBody::new("internal", format!("{}: {message}", diag.id()), true);
    body.detail = Some(json!({ "admitted": true, "diag": diag.id() }));
    body
}

/// A read's failure: an answer over a result budget is `too-large`.
fn read_failed(diag: Diag, message: String) -> ErrorBody {
    if message.starts_with("result-too-large") {
        ErrorBody::new("too-large", message, true)
    } else {
        internal(diag, message)
    }
}

pub(super) fn whole_graph(slot: &GraphSlot) -> Result<WholeGraph, ErrorBody> {
    slot.store
        .whole_graph()
        .map_err(|e| internal(Diag::GraphLoad, format!("graph load failed: {e:?}")))
}

/// A persisted `id::` value (A2): runtime block ids change every open.
pub(super) fn check_uuid(uuid: &str) -> Result<(), ErrorBody> {
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
pub(super) fn resolve_page(graph: &WholeGraph, name: &str) -> Result<PageId, ErrorBody> {
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
pub(super) fn journal_day(slot: &GraphSlot, date: Option<&str>) -> Result<JournalDate, ErrorBody> {
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
/// The byte bound of a preview, as the store's (`PREVIEW_MAX_BYTES`).
const PREVIEW_BYTES: usize = (32 << 20) - 4 * 1024;

/// `block show` from the host's read of the block's page: the subtree as the
/// host holds it, bounded like the store's preview (`max_nodes`, then the
/// byte bound, in outline order), with the handles a page read prints:
/// page-absolute positions and fingerprints of that same text (design §2), so
/// a handle it prints resolves to the block it shows or is `stale`. The page
/// is named as `indexed` names it, the name that resolved to `page`, never the
/// name the bytes now declare: a `title::` changed since indexing would route
/// a printed handle to another file (review 7). `None` when the host cannot
/// read the page now or no longer holds the id there.
fn host_preview(
    slot: &GraphSlot,
    page: &PageId,
    indexed: &tine_core::model::RefGroup,
    uuid: &str,
    max_nodes: usize,
) -> Result<Option<Value>, ErrorBody> {
    let Ok(Ok(read)) = crate::page_commands::with_host(slot, |host| host.read(page)) else {
        return Ok(None);
    };
    let Some(parsed) = read
        .bytes
        .and_then(|bytes| slot.store.page_of(&read.page, &bytes).ok())
    else {
        return Ok(None);
    };
    let handle = super::write::Handle::Uuid(uuid.to_ascii_lowercase());
    let Ok(path) = super::write::resolve_on(&parsed.doc, page, &handle, uuid, false) else {
        return Ok(None);
    };
    let Some(root) = super::write::block_at(&parsed.doc.blocks, &path) else {
        return Ok(None);
    };
    let total = node_count(root);
    let mut nodes = max_nodes;
    let mut bytes = PREVIEW_BYTES;
    let blocks: Vec<_> = bounded(root, &mut nodes, &mut bytes).into_iter().collect();
    let emitted = max_nodes - nodes;
    let preview = tine_core::model::BlockPreview {
        group: tine_core::model::RefGroup {
            page: indexed.page.clone(),
            kind: indexed.kind,
            blocks,
            evidence: Vec::new(),
        },
        truncated: total - emitted,
    };
    let mut value = to_value(&preview)?;
    let org = parsed.doc.format == tine_core::model::Format::Org;
    let at = super::write::position(&path);
    if let Some(root) = value["group"]["blocks"]
        .get_mut(0)
        .and_then(Value::as_object_mut)
    {
        let fp =
            super::write::fingerprint(root.get("raw").and_then(Value::as_str).unwrap_or(""), org);
        root.insert("handle".into(), json!(format!("{at}~{fp}")));
        if let Some(children) = root.get_mut("children") {
            super::write::annotate_handles(children, org, &format!("{at}."));
        }
    }
    Ok(Some(value))
}

fn node_count(block: &tine_core::model::BlockDto) -> usize {
    1 + block.children.iter().map(node_count).sum::<usize>()
}

/// `block` and its descendants in outline order until a bound is reached,
/// each node charged as the store's projector charges it (id, text,
/// properties, tags, breadcrumb, overhead; review 7).
fn bounded(
    block: &tine_core::model::BlockDto,
    nodes: &mut usize,
    bytes: &mut usize,
) -> Option<tine_core::model::BlockDto> {
    if *nodes == 0 || block.raw.len().saturating_add(block.id.len()) > *bytes {
        return None;
    }
    let mut kept = tine_core::model::BlockDto {
        children: Vec::new(),
        ..block.clone()
    };
    let cost = tine_core::model::block_dto_estimated_bytes(&kept);
    if cost > *bytes {
        return None;
    }
    *nodes -= 1;
    *bytes -= cost;
    for child in &block.children {
        match bounded(child, nodes, bytes) {
            Some(child) => kept.children.push(child),
            None => break,
        }
    }
    Some(kept)
}

fn page_answer(slot: &GraphSlot, id: PageId, kind: &str) -> Result<Value, ErrorBody> {
    // The host lock is held for the read only (A-K3: nothing under it takes it again).
    let read = crate::page_commands::with_host(slot, |host| host.read(&id))
        .map_err(|_| busy())?
        .map_err(|error| match error {
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
    let mut blocks = to_value(&parsed.doc.blocks)?;
    super::write::annotate_handles(
        &mut blocks,
        parsed.doc.format == tine_core::model::Format::Org,
        "",
    );
    let mut doc = to_value(&parsed.doc)?;
    if let Some(fields) = doc.as_object_mut() {
        fields.insert("blocks".into(), blocks);
    }
    Ok(ok(
        kind,
        json!({
            "id": parsed.id.as_str(),
            "rev": String::from(parsed.rev),
            "revision": revision,
            "read_only": parsed.read_only,
            "text": String::from_utf8_lossy(&bytes),
            "page": doc,
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
        other => internal(Diag::StoreFailed, format!("{other:?}")),
    }
}
