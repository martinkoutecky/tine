//! The graph-backed query command wire for desktop and browser. Parsing
//! admits the source before looking up its generation's registry. Runs use
//! the store's construction and response bounds.

use std::sync::Arc;

use tine_core::query::ir::{ExecutionContext, Query, QueryResult, ViewSettings};
use tine_core::query::registry::Registry;
use tine_core::query::wire_parse::{parse_query_pair, ParsedQuery, QueryTextDialect};
use tine_store::{Cancel, IrAnswer, IrRequest, WholeGraph};

/// O(source bytes); refuse before parsing or acquiring the registry.
pub fn validate_query_source(query: &str) -> Result<(), String> {
    if !tine_core::query::query_source_within_limit(query) {
        return Err(format!(
            "query-too-large: query source is {} bytes (limit: {} bytes)",
            query.len(),
            tine_core::query::QUERY_SOURCE_MAX_BYTES
        ));
    }
    if !tine_core::query::query_nesting_within_limit(query) {
        return Err("query-nesting-too-deep: query nesting exceeds the parser limit".into());
    }
    Ok(())
}

/// A run stopped by its flag (the binding was revoked: graph switch or
/// close) answers this, never a partial result.
pub const QUERY_CANCELLED: &str = "query-cancelled: the graph was closed or switched";

pub fn registry(graph: &WholeGraph) -> Result<Arc<Registry>, String> {
    // The registry request never checks its flag: it is built once per
    // generation and shared.
    match graph.query_ir(IrRequest::Registry, &Cancel::default()) {
        Ok(Some(IrAnswer::Registry(registry))) => Ok(registry),
        Ok(_) => Err("query-registry: unexpected answer".into()),
        Err(error) => Err(error.to_string()),
    }
}

pub fn parse(
    graph: &WholeGraph,
    text: &str,
    dialect: QueryTextDialect,
    block_properties: &[(String, String)],
) -> Result<ParsedQuery, String> {
    validate_query_source(text)?;
    Ok(parse_query_pair(
        text,
        dialect,
        block_properties,
        &*registry(graph)?,
    ))
}

/// `query_run`'s answer on the wire: the result plus, for a day-relative
/// query, the day it was resolved for (`QueryResult::as_of_day`, card hB3Lv8),
/// which the result's own serde form never writes.
#[derive(serde::Serialize)]
pub struct QueryRunAnswer {
    #[serde(flatten)]
    result: QueryResult,
    #[serde(skip_serializing_if = "Option::is_none")]
    as_of_day: Option<i64>,
}

/// Execution inputs carried only by the generated query home of a folder
/// export. The reviewed names already appear in the selected source graph.
#[derive(Default, serde::Deserialize, serde::Serialize)]
pub struct PublicationQuery {
    pub current_page: Option<String>,
    pub host_block_id: Option<String>,
}

pub fn run(
    graph: &WholeGraph,
    query: &Query,
    view: &ViewSettings,
    context: &ExecutionContext,
    cancel: &Cancel,
) -> Result<QueryResult, String> {
    run_request(
        graph,
        IrRequest::Run {
            query,
            view,
            context,
        },
        cancel,
    )
}

fn run_request(
    graph: &WholeGraph,
    request: IrRequest<'_>,
    cancel: &Cancel,
) -> Result<QueryResult, String> {
    match graph.query_ir(request, cancel) {
        Ok(Some(answer)) => QueryResult::try_from(answer),
        Ok(None) => Err(QUERY_CANCELLED.into()),
        Err(error) => Err(error.to_string()),
    }
}

/// Execute one IR query and include the resolution day for relative dates.
pub fn run_wire(
    graph: &WholeGraph,
    query: &Query,
    view: &ViewSettings,
    context: &ExecutionContext,
    cancel: &Cancel,
) -> Result<QueryRunAnswer, String> {
    run(graph, query, view, context, cancel).map(run_answer)
}

/// Run the generated home query with its reviewed inputs, against selected
/// physical owners. Bounds and serialization are shared with desktop runs.
pub fn run_publication(
    graph: &WholeGraph,
    query: &Query,
    view: &ViewSettings,
    home: &str,
    reviewed: &PublicationQuery,
) -> Result<QueryRunAnswer, String> {
    let context = ExecutionContext {
        current_page: reviewed.current_page.clone(),
    };
    run_request(
        graph,
        IrRequest::RunPublication {
            query,
            view,
            context: &context,
            home,
            host_block_id: reviewed.host_block_id.as_deref(),
        },
        &Cancel::default(),
    )
    .map(run_answer)
}

fn run_answer(result: QueryResult) -> QueryRunAnswer {
    QueryRunAnswer {
        as_of_day: result.as_of_day,
        result,
    }
}
