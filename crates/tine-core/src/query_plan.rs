//! Pure query result data shared by the store and clients.
#![deny(missing_docs)]

use crate::model::{BlockDto, PageEntry, PageId, PageKind};
use serde::{Deserialize, Serialize};

/// Text field tested by a text predicate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextField {
    /// Decoded page title or alias.
    PageName,
    /// Projected visible block text.
    VisibleContent,
}

/// Matching is explicit in the plan.  In particular, a fuzzy page-name match
/// never makes block-content predicates fuzzy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextMatchMode {
    /// Literal substring.
    Contains,
    /// Literal phrase.
    Phrase,
    /// Regular expression.
    Regex,
    /// Ordered-subsequence fuzzy match.
    Fuzzy,
}

/// Explainable objective relevance. Variant order is deliberately not used for
/// ranking; `rank()` below is the single ordering contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectiveMatchClass {
    /// Exact entity name.
    Exact,
    /// Name prefix.
    Prefix,
    /// Name substring.
    Substring,
    /// Fuzzy name match.
    Fuzzy,
    /// Match found in block body.
    BodyEvidence,
}

impl ObjectiveMatchClass {
    /// Fixed rank for this relevance class, higher first.
    pub fn rank(self) -> i32 {
        match self {
            Self::Exact => 5,
            Self::Prefix => 4,
            Self::Substring => 3,
            Self::Fuzzy => 2,
            Self::BodyEvidence => 1,
        }
    }
}

/// Browser-facing offsets are UTF-16 code-unit offsets (the unit used by JS
/// string slicing and DOM selection), not Rust/regex byte offsets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatchSpan {
    /// Inclusive start offset in UTF-16 code units.
    pub start: usize,
    /// Exclusive end offset in UTF-16 code units.
    pub end: usize,
}

/// One positive clause's reason for accepting an entity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatchEvidence {
    /// Predicate clause that matched.
    pub clause_id: u32,
    /// Entity text field that matched.
    pub field: TextField,
    /// Matching rule applied.
    pub mode: TextMatchMode,
    /// Matched source spans.
    pub spans: Vec<MatchSpan>,
    /// Predicate-local relevance.  Only fuzzy predicates currently populate it;
    /// final page ranking is carried on the page hit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<i32>,
}

/// Stable diagnostic codes let the frontend localize/rephrase messages later.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryDiagnostic {
    /// Stable machine-readable diagnostic code.
    pub code: String,
    /// Human-readable diagnostic.
    pub message: String,
    /// Source span, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub span: Option<MatchSpan>,
}

/// A cheap declarative explanation tree.  Per-candidate counts/timings can be
/// layered on later without changing query membership or match evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExplainNode {
    /// Clause represented by this node, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clause_id: Option<u32>,
    /// Human-readable explanation.
    pub description: String,
    /// Nested explanation nodes.
    pub children: Vec<ExplainNode>,
}

/// Explanation branches for an executed query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryExplanation {
    /// Top-level explanation nodes.
    pub branches: Vec<ExplainNode>,
}

/// Result-only entity union.  Match metadata intentionally does not live on
/// `BlockDto`, because that DTO also crosses the write boundary.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "entity", rename_all = "snake_case")]
pub enum QueryHit {
    /// One page-name result.
    Page {
        /// Physical page entry.
        page: PageEntry,
        /// Text displayed for this hit.
        display_text: String,
        /// Predicates that matched.
        evidence: Vec<MatchEvidence>,
        /// Objective relevance score.
        score: i32,
        /// Primary relevance band.
        match_class: ObjectiveMatchClass,
        /// Alias matched, if this is an alias hit.
        #[serde(skip_serializing_if = "Option::is_none")]
        matched_alias: Option<String>,
    },
    /// One block-text result.
    Block {
        /// Source page name.
        page: String,
        /// Source page kind.
        kind: PageKind,
        /// Graph-root-relative physical owner of this result. Block ids and page
        /// names are not unique enough to recover it after a duplicate-name hit.
        path: PageId,
        /// Matched block.
        block: BlockDto,
        /// Exact lsdoc-projected visible text indexed by `evidence.spans`.
        display_text: String,
        /// Predicates that matched.
        evidence: Vec<MatchEvidence>,
        /// Objective block relevance. The match class is the primary band;
        /// this score summarizes boundary, offset, length, and occurrence
        /// quality inside that band.
        score: i32,
        /// Primary relevance band.
        match_class: ObjectiveMatchClass,
    },
}

/// Whether additional matches were found beyond enabled category limits.
/// A `false` value for a category with a zero limit means it was not checked.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QueryHasMore {
    /// More page hits existed when the page limit was nonzero.
    #[serde(default)]
    pub pages: bool,
    /// More block hits existed when the block limit was nonzero.
    #[serde(default)]
    pub blocks: bool,
}

/// Bounded graph search answer. Check `has_more` before treating it as complete.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryExecution {
    /// Returned page and block hits.
    pub hits: Vec<QueryHit>,
    /// Query parsing or execution diagnostics.
    pub diagnostics: Vec<QueryDiagnostic>,
    /// Declarative explanation of the search.
    pub explanation: QueryExplanation,
    /// Per-category top-k truncation, detected during the existing candidate scan.
    #[serde(default)]
    pub has_more: QueryHasMore,
    /// Whether execution was cancelled; cancelled work returns no partial results.
    // A cancelled latest-wins lane returns no partial results.
    pub cancelled: bool,
}
