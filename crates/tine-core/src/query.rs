//! Pure query request guards and result data.

use crate::model::RefGroup;

/// Query source crosses several boundaries (live macros, native IPC, static
/// publication, and export). Keep one shared ceiling so no caller can make the
/// parser or its cache key proportional to an unbounded graph-authored string.
pub const QUERY_SOURCE_MAX_BYTES: usize = 64 * 1024;
const QUERY_NESTING_MAX: usize = 64;

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
/// "unsupported". `supported` is false only when nothing in the subset matched.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AdvancedResult {
    pub groups: Vec<RefGroup>,
    pub ran: Vec<String>,
    pub ignored: Vec<String>,
    pub supported: bool,
}

/// One query macro requested by Copy / Export. Query evaluation and subtree
/// hydration stay in the same native operation so a shallow result never causes
/// the WebView to fetch and retain its complete source page.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct QueryExportSpec {
    pub key: String,
    pub query: String,
    pub advanced: bool,
}

/// A single query macro's bounded, hierarchy-preserving export projection.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct QueryExportResult {
    pub key: String,
    pub groups: Vec<RefGroup>,
    pub shown: usize,
    pub total: usize,
    pub omitted_nodes: usize,
}

/// All query macros in one export session share the same construction budget.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct QueryExportBatch {
    pub results: Vec<QueryExportResult>,
    /// Query macros beyond the native request cap are not evaluated. The caller
    /// renders an explicit truncation note rather than silently expanding them
    /// through an unbounded sequence of independent requests.
    pub omitted_queries: usize,
}

/// Is this query body an advanced datalog query we don't support?
pub fn is_advanced(query_src: &str) -> bool {
    let s = query_src.trim_start();
    s.starts_with("[:find") || s.contains(":where") || s.contains(":find")
}
