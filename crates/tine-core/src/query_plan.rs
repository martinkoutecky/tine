//! Pure query result data shared by the store and clients.

use crate::model::{BlockDto, PageEntry, PageId, PageKind};
use serde::{Deserialize, Serialize};

/// Text field tested by a text predicate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextField {
    PageName,
    VisibleContent,
}

/// Matching is explicit in the plan.  In particular, a fuzzy page-name match
/// never makes block-content predicates fuzzy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextMatchMode {
    Contains,
    Phrase,
    Regex,
    Fuzzy,
}

/// Explainable objective relevance. Variant order is deliberately not used for
/// ranking; `rank()` below is the single ordering contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectiveMatchClass {
    Exact,
    Prefix,
    Substring,
    Fuzzy,
    BodyEvidence,
}

impl ObjectiveMatchClass {
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
    pub start: usize,
    pub end: usize,
}

/// One positive clause's reason for accepting an entity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatchEvidence {
    pub clause_id: u32,
    pub field: TextField,
    pub mode: TextMatchMode,
    pub spans: Vec<MatchSpan>,
    /// Predicate-local relevance.  Only fuzzy predicates currently populate it;
    /// final page ranking is carried on the page hit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<i32>,
}

/// Stable diagnostic codes let the frontend localize/rephrase messages later.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryDiagnostic {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub span: Option<MatchSpan>,
}

/// A cheap declarative explanation tree.  Per-candidate counts/timings can be
/// layered on later without changing query membership or match evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExplainNode {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clause_id: Option<u32>,
    pub description: String,
    pub children: Vec<ExplainNode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryExplanation {
    pub branches: Vec<ExplainNode>,
}

/// Result-only entity union.  Match metadata intentionally does not live on
/// `BlockDto`, because that DTO also crosses the write boundary.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "entity", rename_all = "snake_case")]
pub enum QueryHit {
    Page {
        page: PageEntry,
        display_text: String,
        evidence: Vec<MatchEvidence>,
        score: i32,
        match_class: ObjectiveMatchClass,
        #[serde(skip_serializing_if = "Option::is_none")]
        matched_alias: Option<String>,
    },
    Block {
        page: String,
        kind: PageKind,
        /// Graph-root-relative physical owner of this result. Block ids and page
        /// names are not unique enough to recover it after a duplicate-name hit.
        path: PageId,
        block: BlockDto,
        /// Exact lsdoc-projected visible text indexed by `evidence.spans`.
        display_text: String,
        evidence: Vec<MatchEvidence>,
        /// Objective block relevance. The match class is the primary band;
        /// this score summarizes boundary, offset, length, and occurrence
        /// quality inside that band.
        score: i32,
        match_class: ObjectiveMatchClass,
    },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QueryHasMore {
    #[serde(default)]
    pub pages: bool,
    #[serde(default)]
    pub blocks: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryExecution {
    pub hits: Vec<QueryHit>,
    pub diagnostics: Vec<QueryDiagnostic>,
    pub explanation: QueryExplanation,
    /// Per-category top-k truncation, detected during the existing candidate scan.
    #[serde(default)]
    pub has_more: QueryHasMore,
    /// A cancelled latest-wins lane returns no partial results.
    pub cancelled: bool,
}
