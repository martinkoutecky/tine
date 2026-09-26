//! Pure query request limits and result data shared with graph clients.
#![deny(missing_docs)]

use crate::model::RefGroup;

/// Maximum query source length accepted by evaluators, in UTF-8 bytes.
pub const QUERY_SOURCE_MAX_BYTES: usize = 64 * 1024;
const QUERY_NESTING_MAX: usize = 64;

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

/// Result of an advanced (datalog) query: matched groups + which clause heads
/// ran vs were ignored, so the UI shows "ran X; ignored Y" rather than a blunt
/// "unsupported". `supported` is false when no supported clause was recognized.
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
    /// Evaluate as advanced datalog when true, simple syntax otherwise.
    pub advanced: bool,
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

/// Whether the source uses advanced datalog syntax.
pub fn is_advanced(query_src: &str) -> bool {
    let s = query_src.trim_start();
    s.starts_with("[:find") || s.contains(":where") || s.contains(":find")
}
