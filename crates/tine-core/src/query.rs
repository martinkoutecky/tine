//! Pure query request limits and result data shared with graph clients, and
//! the pure query language layer (master 0.6.983+ model, og lane Q1):
//!
//! - [`ir`] — the one query IR every dialect lowers to;
//! - [`parse`] (re-exported here) — the ONE text → IR entry for every input
//!   shape (`{{query}}` OG DSL, `{{tine-query}}` TQL, advanced datalog) and
//!   the execution-time binding [`resolve_for_execution`];
//! - [`print`] — IR → text for either dialect; [`macro_text`] — macro
//!   argument scanning; [`wire_parse`] — the IPC parse helper;
//! - [`atom`], [`registry`] — typed property atoms and the observed/declared
//!   property registry built from caller-supplied rows;
//! - [`view`], [`sort`], [`statistics`], [`path_refs`], [`text`] — pure
//!   view, ordering, aggregate and reference helpers.
//!
//! Everything here is pure: no graph walk, no store, no SQL. Costs are
//! O(source length) for parsing/printing and O(rows supplied) for the
//! registry and statistics folds. Execution lives in `tine-store`.
#![deny(missing_docs)]

use crate::model::RefGroup;

#[allow(missing_docs)]
mod advanced_patterns;
#[allow(missing_docs)]
pub mod atom;
#[cfg(test)]
mod columns_resolution_tests;
#[cfg(test)]
mod conformance;
#[cfg(test)]
mod grouping_resolution_tests;
#[allow(missing_docs)]
pub mod ir;
#[cfg(test)]
mod ir_wire_tests;
#[cfg(test)]
mod macro_extents_tests;
#[allow(missing_docs)]
pub mod macro_text;
#[allow(missing_docs)]
pub mod og;
#[allow(missing_docs)]
mod parse;
#[cfg(test)]
mod parse_tests;
#[allow(missing_docs)]
pub mod path_refs;
#[allow(missing_docs)]
pub mod print;
#[allow(missing_docs)]
pub mod registry;
#[allow(missing_docs)]
pub mod sort;
#[allow(missing_docs)]
pub mod statistics;
#[allow(missing_docs)]
pub mod text;
#[allow(missing_docs)]
pub mod tql;
#[allow(missing_docs)]
pub mod view;
#[allow(missing_docs)]
pub mod wire_parse;

pub use advanced_patterns::{resolve_date_token, DateToken, DateUnit, MAX_DATE_OFFSET_YEARS};
pub use parse::*;

/// Properties that are internal/metadata and are not offered as query
/// filters or registry keys (mirrors the frontend's hidden-property set).
const INTERNAL_PROPS: &[&str] = &[
    "id",
    "collapsed",
    "hl-page",
    "hl-color",
    "hl-type",
    "ls-type",
    "background-color",
    "logseq.order-list-type",
    "template",
    "template-including-parent",
];

/// The built-in half of the registry's internal-key exclusion (master §6.2
/// K15): the registry excludes this set, the configured hidden properties and
/// every `tine.*` key. O(1).
pub fn internal_property_keys() -> &'static [&'static str] {
    INTERNAL_PROPS
}

/// Maximum query source length accepted by evaluators, in UTF-8 bytes.
pub const QUERY_SOURCE_MAX_BYTES: usize = 64 * 1024;
/// Maximum parenthesis depth either query parser accepts, matching the
/// document parser's 128-level input ceiling.
pub(crate) const QUERY_NESTING_MAX: usize = 128;

/// Reason a query source cannot enter an evaluator or cache.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceRefusal {
    /// UTF-8 source exceeds the byte limit.
    TooLarge,
    /// Boolean parentheses exceed the parser depth limit.
    TooDeep,
}

/// Admit a query before parsing or storing its source in a cache key.
pub fn admit_source(source: &str) -> Result<(), SourceRefusal> {
    if !query_source_within_limit(source) {
        Err(SourceRefusal::TooLarge)
    } else if !query_nesting_within_limit(source) {
        Err(SourceRefusal::TooDeep)
    } else {
        Ok(())
    }
}

/// Whether `source` fits the shared byte limit.
pub fn query_source_within_limit(source: &str) -> bool {
    source.len() <= QUERY_SOURCE_MAX_BYTES
}

/// Iterative, string/comment-aware guard before either recursive DSL parser.
/// Count parentheses because those are the only delimiters that construct
/// recursive predicates; brackets/braces are scanned iteratively as data.
pub fn query_nesting_within_limit(source: &str) -> bool {
    let semicolon_comments = is_advanced(source);
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    let mut in_comment = false;
    for byte in source.bytes() {
        if in_comment {
            if byte == b'\n' {
                in_comment = false;
            }
            continue;
        }
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b';' if semicolon_comments => in_comment = true,
            b'"' => in_string = true,
            b'(' => {
                depth = depth.saturating_add(1);
                if depth > QUERY_NESTING_MAX {
                    return false;
                }
            }
            b')' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    true
}

/// Result of an advanced (datalog) query run over Tine's recognized clause
/// subset, so the UI shows "ran X; ignored Y" rather than a blunt "unsupported".
///
/// `ran` lists the clause heads that were evaluated; `ignored` lists the ones
/// that were not understood and were DROPPED from the predicate, so when
/// `ignored` is non-empty `groups` answers a different query than the one
/// written (usually a broader one) and must be presented as partial.
/// `supported` is false, and `groups` empty, when no clause was recognized or
/// when the source was refused before parsing (`ignored` then holds
/// `"query-too-large"` or `"query-nesting-too-deep"`).
#[deny(missing_docs)]
#[derive(Debug, Clone, serde::Serialize)]
pub struct AdvancedResult {
    /// Matched source-page groups.
    pub groups: Vec<RefGroup>,
    /// Recognized clause heads that ran.
    pub ran: Vec<String>,
    /// Unsupported clause heads that were ignored.
    pub ignored: Vec<String>,
    /// Whether at least one supported clause was recognized.
    pub supported: bool,
}

/// One query macro requested by Copy / Export. Its selected subtree is
/// returned without requiring the caller to fetch the entire source page.
#[deny(missing_docs)]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct QueryExportSpec {
    /// Caller key returned with the corresponding result.
    pub key: String,
    /// Query expression source.
    pub query: String,
}

/// A single query macro's bounded, hierarchy-preserving export projection.
#[deny(missing_docs)]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct QueryExportResult {
    /// Caller key from the request.
    pub key: String,
    /// Projected source-page groups.
    pub groups: Vec<RefGroup>,
    /// Number of roots shown.
    pub shown: usize,
    /// Total matching roots before truncation.
    pub total: usize,
    /// Nodes omitted by the node or byte budget, including a root that did not
    /// fit and a selected root absent when hydrated.
    pub omitted_nodes: usize,
}

/// All query macros in one export session share the same construction budget.
#[deny(missing_docs)]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct QueryExportBatch {
    /// Results for evaluated query specs, in request order.
    pub results: Vec<QueryExportResult>,
    /// Accepted query specs beyond the 64-query processing cap are not evaluated. The caller
    /// renders an explicit truncation note rather than silently expanding them
    /// through an unbounded sequence of independent requests.
    pub omitted_queries: usize,
}

/// Whether a `{{query}}` form is datalog: a `:find`/`:where` token outside
/// an OG string or a `[[page]]` ref. This is the ONE answerer (I-12): it is
/// the §7.1 macro discriminator (`parse::advanced_form`) itself, so the OG
/// parser, [`query_nesting_within_limit`]'s comment rule and query export
/// cannot disagree with the macro reader (`{{query "meeting :where"}}` was
/// refused as datalog by a second, substring answerer; og C3 L02).
/// No size check. O(source length).
pub fn is_advanced(query_src: &str) -> bool {
    parse::advanced_form(query_src)
}
