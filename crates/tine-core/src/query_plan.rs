//! Typed execution plan for friendly graph search.
//!
//! This module is deliberately narrower than the full `{{query}}` DSL today:
//! it unifies the two graph-backed parts of Ctrl-K (page names and block text)
//! without changing the command/create-page providers or the established
//! block-query result contract.  The plan/result types are the seam that a
//! durable query workspace can grow into later.

use crate::doc::DocBlock;
use crate::model::{BlockDto, Graph, PageEntry, PageKind};
use crate::refs;
use crate::search_query::{canonical_fold, Matcher, Term};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};
use unicode_normalization::UnicodeNormalization;
use unicode_segmentation::UnicodeSegmentation;

const MAX_EVIDENCE_SPANS: usize = 32;

/// The entity kind a query-plan branch selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryTarget {
    Pages,
    Blocks,
}

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
    fn rank(self) -> i32 {
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

/// A typed text predicate in the shared plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextPredicate {
    pub clause_id: u32,
    pub field: TextField,
    pub mode: TextMatchMode,
    pub value: String,
}

/// Boolean query expression.  A successful NOT contributes no positive match
/// evidence, which keeps "why did this match?" explanations honest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum QueryExpr {
    Text(TextPredicate),
    And(Vec<QueryExpr>),
    Or(Vec<QueryExpr>),
    Not(Box<QueryExpr>),
    Never,
}

/// One independently limited entity branch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryBranch {
    pub target: QueryTarget,
    pub predicate: QueryExpr,
    pub limit: usize,
}

/// One routed page used to scope a block-search plan. A supplied relative path
/// is authoritative so duplicate display identities do not leak into results;
/// otherwise kind plus Logseq's canonical page identity selects the document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryPageScope {
    pub name: String,
    pub page_kind: PageKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
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
        path: String,
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

/// Compiled friendly graph-search plan.  Regexes are compiled once and kept off
/// the wire; the public expression remains inspectable/serializable.
#[derive(Debug, Clone)]
pub struct QueryPlan {
    pub branches: Vec<QueryBranch>,
    pub diagnostics: Vec<QueryDiagnostic>,
    page_scope: Option<QueryPageScope>,
    // Ctrl-K keeps the literal trimmed launcher source so a multi-word page
    // title/alias can retain the objective Exact class. The parsed AND terms
    // alone would otherwise downgrade `Foo Bar` to Prefix/Substring and make
    // the frontend offer a duplicate Create row beside the existing page.
    page_exact: Option<String>,
    regexes: HashMap<u32, Regex>,
}

impl QueryPlan {
    /// Ctrl-K graph providers: fuzzy page names for a single bare term, but
    /// ordinary contains/phrase/regex semantics for block content.  Multi-term
    /// and operator searches use the same boolean grammar on both entity kinds.
    pub fn friendly(query: &str, page_limit: usize, block_limit: usize) -> Self {
        let matcher = Matcher::parse(query);
        let mut next_id = 1;
        let mut regexes = HashMap::new();
        let mut diagnostics = Vec::new();
        if let Matcher::InvalidRegex(error) = &matcher {
            diagnostics.push(QueryDiagnostic {
                code: "invalid_regex".into(),
                message: error.clone(),
                span: Some(MatchSpan {
                    start: 0,
                    end: query.encode_utf16().count(),
                }),
            });
        }

        let mut branches = Vec::new();
        if !matches!(matcher, Matcher::Empty | Matcher::InvalidRegex(_)) {
            let page_expr = if let Some(term) = matcher.simple_term() {
                let id = take_id(&mut next_id);
                QueryExpr::Text(TextPredicate {
                    clause_id: id,
                    field: TextField::PageName,
                    mode: TextMatchMode::Fuzzy,
                    value: term.to_string(),
                })
            } else {
                expr_from_matcher(&matcher, TextField::PageName, &mut next_id, &mut regexes)
            };
            branches.push(QueryBranch {
                target: QueryTarget::Pages,
                predicate: page_expr,
                limit: page_limit,
            });
            branches.push(QueryBranch {
                target: QueryTarget::Blocks,
                predicate: expr_from_matcher(
                    &matcher,
                    TextField::VisibleContent,
                    &mut next_id,
                    &mut regexes,
                ),
                limit: block_limit,
            });
        }
        Self {
            branches,
            diagnostics,
            page_scope: None,
            page_exact: (!query.trim().is_empty()).then(|| canonical_fold(query.trim())),
            regexes,
        }
    }

    /// Current-page search is a block-only execution profile of the same typed
    /// friendly plan—not a frontend filter over whole-graph results.
    pub fn friendly_for_page(query: &str, block_limit: usize, scope: QueryPageScope) -> Self {
        let mut plan = Self::block_search(query, block_limit);
        plan.page_scope = Some(scope);
        plan
    }

    /// Explicit page-name fuzzy plan for normal-query frontends and tests.  This
    /// constructor makes the opt-in visible in the typed IR; it never changes the
    /// default behavior of existing block queries.
    pub fn page_name_fuzzy(value: impl Into<String>, limit: usize) -> Self {
        let value = value.into();
        Self {
            branches: vec![QueryBranch {
                target: QueryTarget::Pages,
                predicate: QueryExpr::Text(TextPredicate {
                    clause_id: 1,
                    field: TextField::PageName,
                    mode: TextMatchMode::Fuzzy,
                    value: canonical_fold(&value),
                }),
                limit,
            }],
            diagnostics: Vec::new(),
            page_scope: None,
            page_exact: None,
            regexes: HashMap::new(),
        }
    }

    /// Existing block-search API expressed as one typed branch.
    pub fn block_search(query: &str, limit: usize) -> Self {
        let matcher = Matcher::parse(query);
        let mut next_id = 1;
        let mut regexes = HashMap::new();
        let mut diagnostics = Vec::new();
        if let Matcher::InvalidRegex(error) = &matcher {
            diagnostics.push(QueryDiagnostic {
                code: "invalid_regex".into(),
                message: error.clone(),
                span: Some(MatchSpan {
                    start: 0,
                    end: query.encode_utf16().count(),
                }),
            });
        }
        let branches = if matches!(matcher, Matcher::Empty | Matcher::InvalidRegex(_)) {
            Vec::new()
        } else {
            vec![QueryBranch {
                target: QueryTarget::Blocks,
                predicate: expr_from_matcher(
                    &matcher,
                    TextField::VisibleContent,
                    &mut next_id,
                    &mut regexes,
                ),
                limit,
            }]
        };
        Self {
            branches,
            diagnostics,
            page_scope: None,
            page_exact: None,
            regexes,
        }
    }

    /// Legacy quick-switch backend candidates.  Empty/pure-negative input keeps
    /// the old "all pages" candidate behavior; the authoritative friendly plan
    /// intentionally treats those inputs as no graph search.
    pub fn legacy_page_search(query: &str, limit: usize) -> Self {
        let matcher = Matcher::parse(query);
        if matches!(matcher, Matcher::InvalidRegex(_)) {
            return Self {
                branches: Vec::new(),
                diagnostics: Vec::new(),
                page_scope: None,
                page_exact: None,
                regexes: HashMap::new(),
            };
        }
        if matches!(matcher, Matcher::Empty) {
            return Self::page_name_fuzzy("", limit);
        }
        let mut next_id = 1;
        let mut regexes = HashMap::new();
        let predicate = if let Some(term) = matcher.simple_term() {
            QueryExpr::Text(TextPredicate {
                clause_id: take_id(&mut next_id),
                field: TextField::PageName,
                mode: TextMatchMode::Fuzzy,
                value: term.to_string(),
            })
        } else {
            expr_from_matcher(&matcher, TextField::PageName, &mut next_id, &mut regexes)
        };
        Self {
            branches: vec![QueryBranch {
                target: QueryTarget::Pages,
                predicate,
                limit,
            }],
            diagnostics: Vec::new(),
            page_scope: None,
            page_exact: (!query.trim().is_empty()).then(|| canonical_fold(query.trim())),
            regexes,
        }
    }

    pub fn explanation(&self) -> QueryExplanation {
        QueryExplanation {
            branches: self
                .branches
                .iter()
                .map(|branch| ExplainNode {
                    clause_id: None,
                    description: match branch.target {
                        QueryTarget::Pages => "Page names".into(),
                        QueryTarget::Blocks => "Block text".into(),
                    },
                    children: vec![explain_expr(&branch.predicate)],
                })
                .collect(),
        }
    }

    /// Execute all graph-backed branches.  Cancellation is checked between page
    /// candidates and before every block projection; no partial result escapes.
    pub fn execute(&self, graph: &Graph, cancelled: impl Fn() -> bool) -> QueryExecution {
        self.execute_with_explain(graph, cancelled, true)
    }

    pub fn execute_with_explain(
        &self,
        graph: &Graph,
        cancelled: impl Fn() -> bool,
        explain: bool,
    ) -> QueryExecution {
        let explanation = if explain {
            self.explanation()
        } else {
            QueryExplanation {
                branches: Vec::new(),
            }
        };
        if !self.diagnostics.is_empty() {
            return QueryExecution {
                hits: Vec::new(),
                diagnostics: self.diagnostics.clone(),
                explanation,
                has_more: QueryHasMore::default(),
                cancelled: false,
            };
        }
        let mut hits = Vec::new();
        let mut has_more = QueryHasMore::default();
        for branch in &self.branches {
            if cancelled() {
                return cancelled_execution(self, explanation);
            }
            let branch_hits = match branch.target {
                QueryTarget::Pages => execute_pages(self, graph, branch, &cancelled),
                QueryTarget::Blocks => execute_blocks(self, graph, branch, &cancelled),
            };
            let Some((mut branch_hits, branch_has_more)) = branch_hits else {
                return cancelled_execution(self, explanation);
            };
            match branch.target {
                QueryTarget::Pages => has_more.pages |= branch_has_more,
                QueryTarget::Blocks => has_more.blocks |= branch_has_more,
            }
            hits.append(&mut branch_hits);
        }
        QueryExecution {
            hits,
            diagnostics: self.diagnostics.clone(),
            explanation,
            has_more,
            cancelled: false,
        }
    }
}

fn cancelled_execution(plan: &QueryPlan, explanation: QueryExplanation) -> QueryExecution {
    QueryExecution {
        hits: Vec::new(),
        diagnostics: plan.diagnostics.clone(),
        explanation,
        has_more: QueryHasMore::default(),
        cancelled: true,
    }
}

fn take_id(next: &mut u32) -> u32 {
    let id = *next;
    *next = next.saturating_add(1);
    id
}

fn expr_from_matcher(
    matcher: &Matcher,
    field: TextField,
    next_id: &mut u32,
    regexes: &mut HashMap<u32, Regex>,
) -> QueryExpr {
    match matcher {
        Matcher::Regex(re) => {
            let id = take_id(next_id);
            regexes.insert(id, re.clone());
            QueryExpr::Text(TextPredicate {
                clause_id: id,
                field,
                mode: TextMatchMode::Regex,
                value: re.as_str().to_string(),
            })
        }
        Matcher::Boolean(groups) => {
            let groups = groups
                .iter()
                .map(|group| {
                    let terms = group
                        .iter()
                        .map(|term| expr_from_term(term, field, next_id))
                        .collect::<Vec<_>>();
                    if terms.len() == 1 {
                        terms.into_iter().next().unwrap()
                    } else {
                        QueryExpr::And(terms)
                    }
                })
                .collect::<Vec<_>>();
            if groups.len() == 1 {
                groups.into_iter().next().unwrap()
            } else {
                QueryExpr::Or(groups)
            }
        }
        Matcher::InvalidRegex(_) | Matcher::Empty => QueryExpr::Never,
    }
}

fn expr_from_term(term: &Term, field: TextField, next_id: &mut u32) -> QueryExpr {
    let text = QueryExpr::Text(TextPredicate {
        clause_id: take_id(next_id),
        field,
        mode: if term.quoted {
            TextMatchMode::Phrase
        } else {
            TextMatchMode::Contains
        },
        value: term.text.clone(),
    });
    if term.negated {
        QueryExpr::Not(Box::new(text))
    } else {
        text
    }
}

fn explain_expr(expr: &QueryExpr) -> ExplainNode {
    match expr {
        QueryExpr::Text(p) => ExplainNode {
            clause_id: Some(p.clause_id),
            description: match p.mode {
                TextMatchMode::Contains => format!("contains “{}”", p.value),
                TextMatchMode::Phrase => format!("contains the exact phrase “{}”", p.value),
                TextMatchMode::Regex => format!("matches /{}/", p.value),
                TextMatchMode::Fuzzy => format!("fuzzily matches “{}”", p.value),
            },
            children: Vec::new(),
        },
        QueryExpr::And(children) => ExplainNode {
            clause_id: None,
            description: "All of".into(),
            children: children.iter().map(explain_expr).collect(),
        },
        QueryExpr::Or(children) => ExplainNode {
            clause_id: None,
            description: "Any of".into(),
            children: children.iter().map(explain_expr).collect(),
        },
        QueryExpr::Not(child) => ExplainNode {
            clause_id: None,
            description: "Exclude".into(),
            children: vec![explain_expr(child)],
        },
        QueryExpr::Never => ExplainNode {
            clause_id: None,
            description: "matches nothing".into(),
            children: Vec::new(),
        },
    }
}

#[derive(Debug)]
struct EvalMatch {
    evidence: Vec<MatchEvidence>,
}

fn eval_expr(
    plan: &QueryPlan,
    expr: &QueryExpr,
    expected_field: TextField,
    original: &str,
) -> Option<EvalMatch> {
    match expr {
        QueryExpr::Never => None,
        QueryExpr::Text(pred) => {
            if pred.field != expected_field {
                return None;
            }
            match_text(plan, pred, original).map(|evidence| EvalMatch {
                evidence: vec![evidence],
            })
        }
        QueryExpr::And(children) => {
            let mut evidence = Vec::new();
            for child in children {
                let matched = eval_expr(plan, child, expected_field, original)?;
                evidence.extend(matched.evidence);
            }
            Some(EvalMatch { evidence })
        }
        QueryExpr::Or(children) => children
            .iter()
            .find_map(|child| eval_expr(plan, child, expected_field, original)),
        QueryExpr::Not(child) => eval_expr(plan, child, expected_field, original)
            .is_none()
            .then_some(EvalMatch {
                evidence: Vec::new(),
            }),
    }
}

/// Membership-only hot path. Search scans use the lsdoc projection's cached
/// lowercase text and compute UTF-16 evidence only for bounded winning hits.
fn eval_expr_fast(
    plan: &QueryPlan,
    expr: &QueryExpr,
    expected_field: TextField,
    original: &str,
    lower: &str,
) -> bool {
    match expr {
        QueryExpr::Never => false,
        QueryExpr::Text(pred) if pred.field != expected_field => false,
        QueryExpr::Text(pred) => match pred.mode {
            TextMatchMode::Contains | TextMatchMode::Phrase => lower.contains(&pred.value),
            TextMatchMode::Regex => plan
                .regexes
                .get(&pred.clause_id)
                .is_some_and(|regex| regex.is_match(original)),
            TextMatchMode::Fuzzy => fuzzy_name_score(lower, &pred.value).is_some(),
        },
        QueryExpr::And(children) => children
            .iter()
            .all(|child| eval_expr_fast(plan, child, expected_field, original, lower)),
        QueryExpr::Or(children) => children
            .iter()
            .any(|child| eval_expr_fast(plan, child, expected_field, original, lower)),
        QueryExpr::Not(child) => !eval_expr_fast(plan, child, expected_field, original, lower),
    }
}

fn match_text(plan: &QueryPlan, pred: &TextPredicate, original: &str) -> Option<MatchEvidence> {
    match pred.mode {
        TextMatchMode::Contains | TextMatchMode::Phrase => {
            let spans = casefold_substring_spans(original, &pred.value);
            (!spans.is_empty()).then_some(MatchEvidence {
                clause_id: pred.clause_id,
                field: pred.field,
                mode: pred.mode,
                spans,
                score: None,
            })
        }
        TextMatchMode::Regex => {
            let re = plan.regexes.get(&pred.clause_id)?;
            let spans = re
                .find_iter(original)
                .take(MAX_EVIDENCE_SPANS)
                .map(|m| MatchSpan {
                    start: original[..m.start()].encode_utf16().count(),
                    end: original[..m.end()].encode_utf16().count(),
                })
                .collect::<Vec<_>>();
            (!spans.is_empty()).then_some(MatchEvidence {
                clause_id: pred.clause_id,
                field: pred.field,
                mode: pred.mode,
                spans,
                score: None,
            })
        }
        TextMatchMode::Fuzzy => fuzzy_evidence(pred, original),
    }
}

/// Lowercase-plus-NFC text plus one original UTF-16 span per folded scalar.
/// Normalization is performed per extended grapheme cluster, which is the
/// boundary across which canonical composition cannot contribute. Every output
/// scalar maps to the full union of original scalars that formed that grapheme,
/// including lowercase expansions, reordered marks, and Hangul Jamo.
fn folded_with_map(original: &str) -> (Vec<char>, Vec<MatchSpan>) {
    let lowered = original.to_lowercase();
    let mut lowered_sources = Vec::new();
    let mut original_utf16 = 0;
    for ch in original.chars() {
        let start = original_utf16;
        original_utf16 += ch.len_utf16();
        for _ in ch.to_lowercase() {
            lowered_sources.push(MatchSpan {
                start,
                end: original_utf16,
            });
        }
    }
    // Rust's whole-string lowercase differs from scalar lowercase only by
    // contextual substitutions such as final sigma, never by scalar count.
    debug_assert_eq!(lowered.chars().count(), lowered_sources.len());

    let mut folded = Vec::new();
    let mut map = Vec::new();
    let mut source_at = 0;
    for grapheme in lowered.graphemes(true) {
        let scalar_count = grapheme.chars().count();
        let contributors = &lowered_sources[source_at..source_at + scalar_count];
        source_at += scalar_count;
        let source = MatchSpan {
            start: contributors.first().map_or(0, |span| span.start),
            end: contributors.last().map_or(0, |span| span.end),
        };
        for normalized in grapheme.nfc() {
            folded.push(normalized);
            map.push(source);
        }
    }
    debug_assert_eq!(folded.iter().collect::<String>(), canonical_fold(original));
    (folded, map)
}

fn folded_chars(value: &str) -> Vec<char> {
    canonical_fold(value).chars().collect()
}

fn merge_spans(spans: impl IntoIterator<Item = MatchSpan>) -> Vec<MatchSpan> {
    let mut out: Vec<MatchSpan> = Vec::new();
    for span in spans {
        if let Some(last) = out.last_mut() {
            if last.end == span.start {
                last.end = span.end;
                continue;
            }
            if *last == span {
                continue;
            }
        }
        out.push(span);
        if out.len() == MAX_EVIDENCE_SPANS {
            break;
        }
    }
    out
}

fn casefold_substring_spans(original: &str, needle: &str) -> Vec<MatchSpan> {
    let (hay, map) = folded_with_map(original);
    let needle = folded_chars(needle);
    if needle.is_empty() || needle.len() > hay.len() {
        return Vec::new();
    }
    let mut spans = Vec::new();
    for start in 0..=hay.len() - needle.len() {
        if hay[start..start + needle.len()] == needle {
            let first = map[start];
            let last = map[start + needle.len() - 1];
            spans.push(MatchSpan {
                start: first.start,
                end: last.end,
            });
            if spans.len() == MAX_EVIDENCE_SPANS {
                break;
            }
        }
    }
    spans
}

fn fuzzy_evidence(pred: &TextPredicate, original: &str) -> Option<MatchEvidence> {
    let (hay, map) = folded_with_map(original);
    let needle = folded_chars(&pred.value);
    if needle.is_empty() {
        return Some(MatchEvidence {
            clause_id: pred.clause_id,
            field: pred.field,
            mode: pred.mode,
            spans: Vec::new(),
            score: Some(0),
        });
    }
    if needle.len() <= hay.len() {
        for start in 0..=hay.len() - needle.len() {
            if hay[start..start + needle.len()] == needle {
                let first = map[start];
                let last = map[start + needle.len() - 1];
                return Some(MatchEvidence {
                    clause_id: pred.clause_id,
                    field: pred.field,
                    mode: pred.mode,
                    spans: vec![MatchSpan {
                        start: first.start,
                        end: last.end,
                    }],
                    score: Some(if start == 0 { 1000 } else { 500 }),
                });
            }
        }
    }
    let mut at = 0;
    let mut picked = Vec::new();
    for (i, ch) in hay.iter().enumerate() {
        if needle.get(at) == Some(ch) {
            picked.push(map[i]);
            at += 1;
            if at == needle.len() {
                return Some(MatchEvidence {
                    clause_id: pred.clause_id,
                    field: pred.field,
                    mode: pred.mode,
                    spans: merge_spans(picked),
                    score: Some(100),
                });
            }
        }
    }
    None
}

#[derive(Debug)]
enum PageCandidate {
    File(usize),
    Referenced(PageEntry),
}

#[derive(Debug)]
struct ScoredPage {
    score: i32,
    match_class: ObjectiveMatchClass,
    matched_text: String,
    matched_alias: Option<String>,
    index: usize,
    candidate: PageCandidate,
}

impl ScoredPage {
    fn is_better_than(&self, other: &Self) -> bool {
        self.match_class.rank() > other.match_class.rank()
            || (self.match_class == other.match_class
                && (self.score > other.score
                    || (self.score == other.score && self.index < other.index)))
    }
}

impl PartialEq for ScoredPage {
    fn eq(&self, other: &Self) -> bool {
        self.match_class == other.match_class
            && self.score == other.score
            && self.index == other.index
    }
}
impl Eq for ScoredPage {}
impl PartialOrd for ScoredPage {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for ScoredPage {
    fn cmp(&self, other: &Self) -> Ordering {
        // Max-heap root is the WORST retained candidate, ready for eviction.
        other
            .match_class
            .rank()
            .cmp(&self.match_class.rank())
            .then_with(|| other.score.cmp(&self.score))
            .then_with(|| self.index.cmp(&other.index))
    }
}

fn push_page(heap: &mut BinaryHeap<ScoredPage>, limit: usize, candidate: ScoredPage) {
    if heap.len() < limit {
        heap.push(candidate);
    } else if heap
        .peek()
        .is_some_and(|worst| candidate.is_better_than(worst))
    {
        *heap.peek_mut().unwrap() = candidate;
    }
}

#[derive(Debug, Clone, Copy)]
struct BlockRelevance {
    match_class: ObjectiveMatchClass,
    word_boundary: bool,
    first_offset: usize,
    text_len: usize,
    occurrences: usize,
    positive: bool,
}

impl BlockRelevance {
    fn neutral() -> Self {
        Self {
            match_class: ObjectiveMatchClass::Exact,
            word_boundary: true,
            first_offset: 0,
            text_len: 0,
            occurrences: 0,
            positive: false,
        }
    }

    /// Greater means objectively better. Keep the components explicit so very
    /// large blocks cannot collapse distinct offsets or lengths into one
    /// saturated scalar score.
    fn cmp_quality(&self, other: &Self) -> Ordering {
        self.match_class
            .rank()
            .cmp(&other.match_class.rank())
            .then_with(|| self.word_boundary.cmp(&other.word_boundary))
            .then_with(|| other.first_offset.cmp(&self.first_offset))
            .then_with(|| other.text_len.cmp(&self.text_len))
            .then_with(|| other.occurrences.cmp(&self.occurrences))
    }

    fn score(self) -> i32 {
        let offset_penalty = self.first_offset.min(50_000) as i32;
        let length_penalty = self.text_len.min(40_000) as i32;
        let occurrence_penalty = self.occurrences.saturating_sub(1).min(9_999) as i32;
        self.match_class.rank() * 1_000_000 + i32::from(self.word_boundary) * 100_000
            - offset_penalty
            - length_penalty
            - occurrence_penalty
    }
}

#[derive(Debug)]
struct ScoredBlock<'a> {
    relevance: BlockRelevance,
    index: usize,
    page: &'a PageEntry,
    block: &'a DocBlock,
    breadcrumb: Vec<String>,
}

impl ScoredBlock<'_> {
    fn is_better_than(&self, other: &Self) -> bool {
        let quality = self.relevance.cmp_quality(&other.relevance);
        quality == Ordering::Greater
            || (quality == Ordering::Equal
                && (self.page.rel_path.as_str(), self.index)
                    < (other.page.rel_path.as_str(), other.index))
    }
}

impl PartialEq for ScoredBlock<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.relevance.cmp_quality(&other.relevance) == Ordering::Equal
            && (self.page.rel_path.as_str(), self.index)
                == (other.page.rel_path.as_str(), other.index)
    }
}
impl Eq for ScoredBlock<'_> {}
impl PartialOrd for ScoredBlock<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for ScoredBlock<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        // Max-heap root is the WORST retained candidate, ready for eviction.
        other
            .relevance
            .cmp_quality(&self.relevance)
            .then_with(|| {
                (self.page.rel_path.as_str(), self.index)
                    .cmp(&(other.page.rel_path.as_str(), other.index))
            })
    }
}

fn push_block<'a>(
    heap: &mut BinaryHeap<ScoredBlock<'a>>,
    limit: usize,
    candidate: ScoredBlock<'a>,
) {
    if heap.len() < limit {
        heap.push(candidate);
    } else if heap
        .peek()
        .is_some_and(|worst| candidate.is_better_than(worst))
    {
        *heap.peek_mut().unwrap() = candidate;
    }
}

fn starts_at_word_boundary(value: &str, byte_offset: usize) -> bool {
    byte_offset == 0
        || value[..byte_offset]
            .chars()
            .next_back()
            .is_none_or(|ch| !ch.is_alphanumeric() && ch != '_')
}

fn text_predicate_relevance(
    plan: &QueryPlan,
    pred: &TextPredicate,
    original: &str,
    lower: &str,
) -> Option<BlockRelevance> {
    let text_len = original.encode_utf16().count();
    let (match_class, word_boundary, first_offset, occurrences) = match pred.mode {
        TextMatchMode::Contains | TextMatchMode::Phrase => {
            let mut matches = lower.match_indices(&pred.value);
            let (first, _) = matches.next()?;
            let occurrences = 1 + matches.count();
            let match_class = if lower == pred.value {
                ObjectiveMatchClass::Exact
            } else if first == 0 {
                ObjectiveMatchClass::Prefix
            } else {
                ObjectiveMatchClass::Substring
            };
            (
                match_class,
                starts_at_word_boundary(lower, first),
                lower[..first].encode_utf16().count(),
                occurrences,
            )
        }
        TextMatchMode::Regex => {
            let regex = plan.regexes.get(&pred.clause_id)?;
            let mut matches = regex.find_iter(original);
            let first = matches.next()?;
            let occurrences = 1 + matches.count();
            let match_class = if first.start() == 0 && first.end() == original.len() {
                ObjectiveMatchClass::Exact
            } else if first.start() == 0 {
                ObjectiveMatchClass::Prefix
            } else {
                ObjectiveMatchClass::Substring
            };
            (
                match_class,
                starts_at_word_boundary(original, first.start()),
                original[..first.start()].encode_utf16().count(),
                occurrences,
            )
        }
        TextMatchMode::Fuzzy => {
            let (_, match_class) = fuzzy_name_score(lower, &pred.value)?;
            let first = lower.find(&pred.value).unwrap_or(0);
            (
                match_class,
                starts_at_word_boundary(lower, first),
                lower[..first].encode_utf16().count(),
                1,
            )
        }
    };
    Some(BlockRelevance {
        match_class,
        word_boundary,
        first_offset,
        text_len,
        occurrences,
        positive: true,
    })
}

fn block_relevance(
    plan: &QueryPlan,
    expr: &QueryExpr,
    original: &str,
    lower: &str,
) -> Option<BlockRelevance> {
    match expr {
        QueryExpr::Never => None,
        QueryExpr::Text(pred) if pred.field != TextField::VisibleContent => None,
        QueryExpr::Text(pred) => text_predicate_relevance(plan, pred, original, lower),
        QueryExpr::And(children) => {
            let mut combined = BlockRelevance::neutral();
            for child in children {
                let relevance = block_relevance(plan, child, original, lower)?;
                if !relevance.positive {
                    continue;
                }
                if !combined.positive {
                    combined = relevance;
                    continue;
                }
                if relevance.match_class.rank() < combined.match_class.rank() {
                    combined.match_class = relevance.match_class;
                }
                combined.word_boundary &= relevance.word_boundary;
                combined.first_offset =
                    combined.first_offset.saturating_add(relevance.first_offset);
                combined.occurrences = combined.occurrences.saturating_add(relevance.occurrences);
            }
            combined.positive.then_some(combined)
        }
        QueryExpr::Or(children) => {
            let mut best: Option<BlockRelevance> = None;
            for child in children {
                let Some(relevance) = block_relevance(plan, child, original, lower) else {
                    continue;
                };
                if !relevance.positive {
                    continue;
                }
                if best
                    .as_ref()
                    .is_none_or(|current| relevance.cmp_quality(current) == Ordering::Greater)
                {
                    best = Some(relevance);
                }
            }
            best
        }
        QueryExpr::Not(child) => block_relevance(plan, child, original, lower)
            .is_none()
            .then_some(BlockRelevance::neutral()),
    }
}

fn eval_ranked_block_expr(
    plan: &QueryPlan,
    expr: &QueryExpr,
    original: &str,
    lower: &str,
) -> Option<EvalMatch> {
    match expr {
        QueryExpr::Never => None,
        QueryExpr::Text(pred) => {
            if pred.field != TextField::VisibleContent {
                return None;
            }
            match_text(plan, pred, original).map(|evidence| EvalMatch {
                evidence: vec![evidence],
            })
        }
        QueryExpr::And(children) => {
            let mut evidence = Vec::new();
            for child in children {
                evidence.extend(eval_ranked_block_expr(plan, child, original, lower)?.evidence);
            }
            Some(EvalMatch { evidence })
        }
        QueryExpr::Or(children) => {
            let mut best: Option<(BlockRelevance, EvalMatch)> = None;
            for child in children {
                let Some(relevance) = block_relevance(plan, child, original, lower) else {
                    continue;
                };
                let Some(matched) = eval_ranked_block_expr(plan, child, original, lower) else {
                    continue;
                };
                if best
                    .as_ref()
                    .is_none_or(|(current, _)| relevance.cmp_quality(current) == Ordering::Greater)
                {
                    best = Some((relevance, matched));
                }
            }
            best.map(|(_, matched)| matched)
        }
        QueryExpr::Not(child) => eval_ranked_block_expr(plan, child, original, lower)
            .is_none()
            .then_some(EvalMatch {
                evidence: Vec::new(),
            }),
    }
}

fn fuzzy_name_score(lower_name: &str, lower_query: &str) -> Option<(i32, ObjectiveMatchClass)> {
    if lower_query.is_empty() {
        Some((0, ObjectiveMatchClass::Fuzzy))
    } else if lower_name == lower_query {
        Some((1500, ObjectiveMatchClass::Exact))
    } else if lower_name.starts_with(lower_query) {
        Some((1000, ObjectiveMatchClass::Prefix))
    } else if lower_name.contains(lower_query) {
        Some((500, ObjectiveMatchClass::Substring))
    } else {
        let mut chars = lower_name.chars();
        lower_query
            .chars()
            .all(|needle| chars.any(|candidate| candidate == needle))
            .then_some((100, ObjectiveMatchClass::Fuzzy))
    }
}

/// Match + page relevance in one cached-lowercase pass. AND takes the best
/// positive clause; OR uses the first matching branch, mirroring
/// `Matcher::score_name`.
fn page_base_score(
    plan: &QueryPlan,
    expr: &QueryExpr,
    original: &str,
    lower: &str,
) -> Option<(i32, ObjectiveMatchClass)> {
    match expr {
        QueryExpr::Never => None,
        QueryExpr::Text(pred) if pred.field != TextField::PageName => None,
        QueryExpr::Text(pred) => match pred.mode {
            TextMatchMode::Fuzzy => fuzzy_name_score(lower, &pred.value),
            TextMatchMode::Regex => plan
                .regexes
                .get(&pred.clause_id)
                .is_some_and(|regex| regex.is_match(original))
                .then_some((500, ObjectiveMatchClass::Substring)),
            TextMatchMode::Contains | TextMatchMode::Phrase => {
                if lower == pred.value {
                    Some((1500, ObjectiveMatchClass::Exact))
                } else if lower.starts_with(&pred.value) {
                    Some((1000, ObjectiveMatchClass::Prefix))
                } else if lower.contains(&pred.value) {
                    Some((500, ObjectiveMatchClass::Substring))
                } else {
                    None
                }
            }
        },
        QueryExpr::And(children) => {
            let mut score = 0;
            let mut class = ObjectiveMatchClass::Exact;
            for child in children {
                let (child_score, child_class) = page_base_score(plan, child, original, lower)?;
                score = score.max(child_score);
                if child_class.rank() < class.rank() {
                    class = child_class;
                }
            }
            Some((score, class))
        }
        QueryExpr::Or(children) => children
            .iter()
            .find_map(|child| page_base_score(plan, child, original, lower)),
        QueryExpr::Not(child) => {
            (!eval_expr_fast(plan, child, TextField::PageName, original, lower))
                // A successful exclusion contributes no positive relevance and
                // therefore must not weaken the class supplied by an AND sibling.
                .then_some((0, ObjectiveMatchClass::Exact))
        }
    }
}

fn best_page_match(
    plan: &QueryPlan,
    expr: &QueryExpr,
    page_name: &str,
    aliases: &[String],
) -> Option<(i32, ObjectiveMatchClass, String, Option<String>)> {
    let page_match = page_base_score(plan, expr, page_name, &canonical_fold(page_name));
    let mut best = page_match
        .map(|(score, class)| (score, class, page_name.to_string(), None));
    for alias in aliases {
        let Some((score, class)) = page_base_score(plan, expr, alias, &canonical_fold(alias))
        else {
            continue;
        };
        let replace = best.as_ref().is_none_or(|(best_score, best_class, _, _)| {
            class.rank() > best_class.rank() || (class == *best_class && score > *best_score)
        });
        if replace {
            best = Some((score, class, alias.clone(), Some(alias.clone())));
        }
    }
    // Upgrade only an outcome that already satisfied the parsed expression.
    // This repairs the objective class for ordinary multi-word titles without
    // bypassing NOT/OR/regex membership semantics for syntax-looking names.
    if let Some(exact) = plan.page_exact.as_deref() {
        if page_match.is_some() && canonical_fold(page_name) == exact {
            return Some((1500, ObjectiveMatchClass::Exact, page_name.to_string(), None));
        }
        if let Some(alias) = aliases.iter().find(|alias| {
            canonical_fold(alias) == exact
                && page_base_score(plan, expr, alias, &canonical_fold(alias)).is_some()
        }) {
            return Some((1500, ObjectiveMatchClass::Exact, alias.clone(), Some(alias.clone())));
        }
    }
    best
}

fn execute_pages(
    plan: &QueryPlan,
    graph: &Graph,
    branch: &QueryBranch,
    cancelled: &impl Fn() -> bool,
) -> Option<(Vec<QueryHit>, bool)> {
    if branch.limit == 0 {
        return Some((Vec::new(), false));
    }
    let file_pages = graph.list_pages();
    let mut aliases_by_owner: HashMap<String, Vec<String>> = HashMap::new();
    for (alias, _, owner_rel_path) in graph.page_aliases_with_owners() {
        aliases_by_owner
            .entry(owner_rel_path)
            .or_default()
            .push(alias);
    }
    let mut heap = BinaryHeap::new();
    let mut has_more = false;
    for (index, page) in file_pages.iter().enumerate() {
        if cancelled() {
            return None;
        }
        let aliases = aliases_by_owner
            .get(&page.rel_path)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        if let Some((base_score, match_class, matched_text, matched_alias)) =
            best_page_match(plan, &branch.predicate, &page.name, aliases)
        {
            has_more |= heap.len() >= branch.limit;
            push_page(
                &mut heap,
                branch.limit,
                ScoredPage {
                    score: base_score - page.name.len() as i32,
                    match_class,
                    matched_text,
                    matched_alias,
                    index,
                    candidate: PageCandidate::File(index),
                },
            );
        }
    }
    let have: HashSet<String> = file_pages
        .iter()
        .map(|page| canonical_fold(&page.name))
        .collect();
    for (offset, name) in graph.referenced_page_names().into_iter().enumerate() {
        if cancelled() {
            return None;
        }
        let key = canonical_fold(&name);
        if have.contains(&key) {
            continue;
        }
        if let Some((base_score, match_class, matched_text, matched_alias)) =
            best_page_match(plan, &branch.predicate, &name, &[])
        {
            has_more |= heap.len() >= branch.limit;
            let score = base_score - name.len() as i32;
            push_page(
                &mut heap,
                branch.limit,
                ScoredPage {
                    score,
                    match_class,
                    matched_text,
                    matched_alias,
                    index: file_pages.len() + offset,
                    candidate: PageCandidate::Referenced(PageEntry {
                        name,
                        kind: PageKind::Page,
                        date_key: None,
                        rel_path: String::new(),
                        path: std::path::PathBuf::new(),
                    }),
                },
            );
        }
    }
    let mut winners = heap.into_vec();
    winners.sort_by(|a, b| {
        b.match_class
            .rank()
            .cmp(&a.match_class.rank())
            .then_with(|| b.score.cmp(&a.score))
            .then_with(|| a.index.cmp(&b.index))
    });
    Some((
        winners
            .into_iter()
            .map(|winner| {
                let page = match winner.candidate {
                    PageCandidate::File(index) => file_pages[index].clone(),
                    PageCandidate::Referenced(page) => page,
                };
                let evidence = eval_expr(
                    plan,
                    &branch.predicate,
                    TextField::PageName,
                    &winner.matched_text,
                )
                .map(|matched| matched.evidence)
                .unwrap_or_default();
                QueryHit::Page {
                    display_text: winner.matched_text,
                    page,
                    evidence,
                    score: winner.score,
                    match_class: winner.match_class,
                    matched_alias: winner.matched_alias,
                }
            })
            .collect(),
        has_more,
    ))
}

fn crumb_line(block: &DocBlock) -> String {
    let line = block
        .visible_text()
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    if line.chars().count() > 60 {
        format!("{}…", line.chars().take(60).collect::<String>())
    } else {
        line
    }
}

fn walk_blocks<'a>(
    blocks: &'a [DocBlock],
    ancestors: &mut Vec<&'a DocBlock>,
    visit: &mut impl FnMut(&'a DocBlock, &[&'a DocBlock]) -> bool,
) -> bool {
    for block in blocks {
        if !visit(block, ancestors) {
            return false;
        }
        ancestors.push(block);
        let keep_going = walk_blocks(&block.children, ancestors, visit);
        ancestors.pop();
        if !keep_going {
            return false;
        }
    }
    true
}

fn execute_blocks(
    plan: &QueryPlan,
    graph: &Graph,
    branch: &QueryBranch,
    cancelled: &impl Fn() -> bool,
) -> Option<(Vec<QueryHit>, bool)> {
    if branch.limit == 0 {
        return Some((Vec::new(), false));
    }
    graph.with_pages(|pages| {
        let mut heap = BinaryHeap::new();
        let mut has_more = false;
        let mut index = 0usize;
        for (entry, doc) in pages {
            if cancelled() {
                return None;
            }
            if let Some(scope) = &plan.page_scope {
                let selected = match scope.path.as_deref() {
                    Some(path) => entry.rel_path == path,
                    None => {
                        entry.kind == scope.page_kind && refs::same_page(&entry.name, &scope.name)
                    }
                };
                if !selected {
                    continue;
                }
            }
            let mut ancestors = Vec::new();
            walk_blocks(&doc.roots, &mut ancestors, &mut |block, path| {
                if cancelled() {
                    return false;
                }
                let candidate_index = index;
                index = index.saturating_add(1);
                let projection = block.projection();
                let visible = &projection.visible;
                if let Some(relevance) =
                    block_relevance(plan, &branch.predicate, visible, &projection.visible_lower)
                {
                    has_more |= heap.len() >= branch.limit;
                    let retain = heap.len() < branch.limit
                        || heap.peek().is_some_and(|worst: &ScoredBlock<'_>| {
                            relevance.cmp_quality(&worst.relevance) == Ordering::Greater
                                || (relevance.cmp_quality(&worst.relevance) == Ordering::Equal
                                    && (entry.rel_path.as_str(), candidate_index)
                                        < (worst.page.rel_path.as_str(), worst.index))
                        });
                    if retain {
                        push_block(
                            &mut heap,
                            branch.limit,
                            ScoredBlock {
                                relevance,
                                index: candidate_index,
                                page: entry,
                                block,
                                breadcrumb: path
                                    .iter()
                                    .map(|ancestor| crumb_line(ancestor))
                                    .collect(),
                            },
                        );
                    }
                }
                true
            });
            if cancelled() {
                return None;
            }
        }
        let mut winners = heap.into_vec();
        winners.sort_by(|a, b| {
            b.relevance
                .cmp_quality(&a.relevance)
                .then_with(|| {
                    (a.page.rel_path.as_str(), a.index)
                        .cmp(&(b.page.rel_path.as_str(), b.index))
                })
        });
        Some((
            winners
                .into_iter()
                .map(|winner| {
                    let projection = winner.block.projection();
                    let matched = eval_ranked_block_expr(
                        plan,
                        &branch.predicate,
                        &projection.visible,
                        &projection.visible_lower,
                    )
                    .expect("rank and evidence evaluators must agree");
                    // Search hits are result identities, not independent copies
                    // of their entire descendant trees. The source page owns the
                    // hierarchy and live consumers hydrate it once per page.
                    let mut dto = crate::model::block_to_shallow_dto(winner.block);
                    dto.breadcrumb = winner.breadcrumb;
                    QueryHit::Block {
                        page: winner.page.name.clone(),
                        kind: winner.page.kind,
                        path: winner.page.rel_path.clone(),
                        block: dto,
                        display_text: projection.visible.clone(),
                        evidence: matched.evidence,
                        score: winner.relevance.score(),
                        match_class: winner.relevance.match_class,
                    }
                })
                .collect(),
            has_more,
        ))
    })
}

/// Convert typed block hits back to the exact grouped shape used by existing
/// search/query consumers. Hits arrive in global relevance order; only contiguous
/// hits from the same page are coalesced, so flattening the groups preserves that
/// order even when a page appears in more than one group.
pub(crate) fn block_hits_to_groups(hits: Vec<QueryHit>) -> Vec<crate::model::RefGroup> {
    let mut groups: Vec<crate::model::RefGroup> = Vec::new();
    for hit in hits {
        let QueryHit::Block {
            page, kind, block, ..
        } = hit
        else {
            continue;
        };
        if let Some(last) = groups.last_mut() {
            if last.page == page && last.kind == kind {
                last.blocks.push(block);
                continue;
            }
        }
        groups.push(crate::model::RefGroup {
            page,
            kind,
            blocks: vec![block],
            evidence: Vec::new(),
        });
    }
    groups
}

pub(crate) fn page_hits_to_entries(hits: Vec<QueryHit>) -> Vec<PageEntry> {
    hits.into_iter()
        .filter_map(|hit| match hit {
            QueryHit::Page { page, .. } => Some(page),
            QueryHit::Block { .. } => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn text_pred(mode: TextMatchMode, value: &str) -> TextPredicate {
        TextPredicate {
            clause_id: 1,
            field: TextField::PageName,
            mode,
            value: value.into(),
        }
    }

    fn fixture() -> (PathBuf, Graph) {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("tine-query-plan-{}-{nonce}", std::process::id()));
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::create_dir_all(dir.join("journals")).unwrap();
        fs::create_dir_all(dir.join("logseq")).unwrap();
        fs::write(
            dir.join("pages").join("Opinion Diffusion.md"),
            "- Parent\n\t- 🧠 foo ready\n- foo draft\n- ready only\n- regex ABC\n",
        )
        .unwrap();
        fs::write(
            dir.join("pages").join("Opdf Notes.md"),
            "alias:: Research Hub\n\n- unrelated\n",
        )
        .unwrap();
        fs::write(dir.join("pages").join("Xopdf.md"), "- unrelated\n").unwrap();
        fs::write(
            dir.join("pages").join("References.md"),
            "- [[Virtual Opdf]]\n",
        )
        .unwrap();
        let graph = Graph::open(&dir);
        graph.warm_cache();
        (dir, graph)
    }

    fn block_fingerprint(groups: Vec<crate::model::RefGroup>) -> Vec<(String, String)> {
        groups
            .into_iter()
            .flat_map(|group| {
                group
                    .blocks
                    .into_iter()
                    .map(move |block| (group.page.clone(), block.raw))
            })
            .collect()
    }

    fn reference_search(graph: &Graph, query: &str, limit: usize) -> Vec<(String, String)> {
        let matcher = Matcher::parse(query);
        if limit == 0 || matches!(matcher, Matcher::Empty | Matcher::InvalidRegex(_)) {
            return Vec::new();
        }
        graph.with_pages(|pages| {
            let mut out = Vec::new();
            fn visit(
                page: &str,
                blocks: &[DocBlock],
                matcher: &Matcher,
                remaining: &mut usize,
                out: &mut Vec<(String, String)>,
            ) {
                for block in blocks {
                    if *remaining == 0 {
                        return;
                    }
                    let projection = block.projection();
                    if matcher.matches(&projection.visible_lower, &projection.visible) {
                        out.push((page.to_string(), block.raw.clone()));
                        *remaining -= 1;
                    }
                    visit(page, &block.children, matcher, remaining, out);
                }
            }
            let mut remaining = limit;
            for (entry, document) in pages {
                visit(
                    &entry.name,
                    &document.roots,
                    &matcher,
                    &mut remaining,
                    &mut out,
                );
                if remaining == 0 {
                    break;
                }
            }
            out
        })
    }

    #[test]
    fn friendly_simple_term_is_fuzzy_only_for_page_names() {
        let plan = QueryPlan::friendly("opdf", 8, 50);
        assert_eq!(plan.branches.len(), 2);
        assert!(matches!(
            &plan.branches[0].predicate,
            QueryExpr::Text(TextPredicate {
                field: TextField::PageName,
                mode: TextMatchMode::Fuzzy,
                value,
                ..
            }) if value == "opdf"
        ));
        assert!(matches!(
            &plan.branches[1].predicate,
            QueryExpr::Text(TextPredicate {
                field: TextField::VisibleContent,
                mode: TextMatchMode::Contains,
                ..
            })
        ));
    }

    #[test]
    fn graph_search_reports_per_category_truncation() {
        let (dir, graph) = fixture();

        let page_truncated = graph.run_graph_search("opdf", 2, 1, false);
        assert!(page_truncated.has_more.pages);
        assert!(!page_truncated.has_more.blocks);

        let block_truncated = graph.run_graph_search("foo", 10, 1, false);
        assert!(!block_truncated.has_more.pages);
        assert!(block_truncated.has_more.blocks);

        let complete = graph.run_graph_search("foo", 10, 10, false);
        assert!(!complete.has_more.pages);
        assert!(!complete.has_more.blocks);

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn explicit_fuzzy_subsequence_has_score_and_utf16_spans() {
        let plan = QueryPlan::page_name_fuzzy("of", 8);
        let pred = &plan.branches[0].predicate;
        let hit = eval_expr(&plan, pred, TextField::PageName, "🧠 Opinion Diffusion").unwrap();
        assert_eq!(hit.evidence[0].score, Some(100));
        assert_eq!(
            hit.evidence[0].spans,
            vec![
                MatchSpan { start: 3, end: 4 },
                MatchSpan { start: 13, end: 14 }
            ]
        );
    }

    #[test]
    fn unicode_substring_and_regex_spans_use_utf16_units() {
        let contains = QueryPlan::page_name_fuzzy("foo", 8);
        let hit = eval_expr(
            &contains,
            &contains.branches[0].predicate,
            TextField::PageName,
            "🧠 foo",
        )
        .unwrap();
        assert_eq!(hit.evidence[0].spans, vec![MatchSpan { start: 3, end: 6 }]);

        let regex_plan = QueryPlan::friendly("/foo/", 8, 8);
        let page = &regex_plan.branches[0].predicate;
        let hit = eval_expr(&regex_plan, page, TextField::PageName, "🧠 foo").unwrap();
        assert_eq!(hit.evidence[0].spans, vec![MatchSpan { start: 3, end: 6 }]);
    }

    #[test]
    fn canonical_unicode_matches_pages_aliases_blocks_and_original_utf16_spans() {
        let multiword = QueryPlan::friendly("Canonical page", 8, 8);
        let multiword_page = best_page_match(
            &multiword,
            &multiword.branches[0].predicate,
            "Canonical page",
            &[],
        )
        .unwrap();
        assert_eq!(multiword_page.1, ObjectiveMatchClass::Exact);

        let syntax = QueryPlan::friendly("foo -draft", 8, 8);
        assert!(best_page_match(
            &syntax,
            &syntax.branches[0].predicate,
            "foo -draft",
            &[],
        )
        .is_none());
        assert!(best_page_match(
            &syntax,
            &syntax.branches[0].predicate,
            "foo ready",
            &[],
        )
        .is_some());

        let page_plan = QueryPlan::page_name_fuzzy("Café", 8);
        let page_pred = &page_plan.branches[0].predicate;
        let page = best_page_match(&page_plan, page_pred, "Cafe\u{301}", &[]).unwrap();
        assert_eq!(page.1, ObjectiveMatchClass::Exact);
        let page_evidence =
            eval_expr(&page_plan, page_pred, TextField::PageName, "Cafe\u{301}").unwrap();
        assert_eq!(
            page_evidence.evidence[0].spans,
            vec![MatchSpan { start: 0, end: 5 }]
        );

        let alias_plan = QueryPlan::page_name_fuzzy("Résumé", 8);
        let alias = best_page_match(
            &alias_plan,
            &alias_plan.branches[0].predicate,
            "Canonical page",
            &["Re\u{301}sume\u{301}".into()],
        )
        .unwrap();
        assert_eq!(alias.1, ObjectiveMatchClass::Exact);
        assert_eq!(alias.3.as_deref(), Some("Re\u{301}sume\u{301}"));

        let block_plan = QueryPlan::block_search("Résumé", 8);
        let block_pred = &block_plan.branches[0].predicate;
        let original = "🧠 Re\u{301}sume\u{301}";
        let folded = canonical_fold(original);
        assert!(eval_expr_fast(
            &block_plan,
            block_pred,
            TextField::VisibleContent,
            original,
            &folded,
        ));
        let evidence =
            eval_expr(&block_plan, block_pred, TextField::VisibleContent, original).unwrap();
        assert_eq!(
            evidence.evidence[0].spans,
            vec![MatchSpan { start: 3, end: 11 }]
        );

        let hangul = QueryPlan::block_search("\u{ac00}", 8);
        let hit = eval_expr(
            &hangul,
            &hangul.branches[0].predicate,
            TextField::VisibleContent,
            "\u{1100}\u{1161}",
        )
        .unwrap();
        assert_eq!(hit.evidence[0].spans, vec![MatchSpan { start: 0, end: 2 }]);

        let reordered = QueryPlan::block_search("è\u{315}", 8);
        let hit = eval_expr(
            &reordered,
            &reordered.branches[0].predicate,
            TextField::VisibleContent,
            "e\u{315}\u{300}",
        )
        .unwrap();
        assert_eq!(hit.evidence[0].spans, vec![MatchSpan { start: 0, end: 3 }]);

        let negative = QueryPlan::block_search("cafe", 8);
        assert!(eval_expr(
            &negative,
            &negative.branches[0].predicate,
            TextField::VisibleContent,
            "café",
        )
        .is_none());

        let expansion = QueryPlan::block_search("i\u{307}", 8);
        let hit = eval_expr(
            &expansion,
            &expansion.branches[0].predicate,
            TextField::VisibleContent,
            "\u{130}",
        )
        .unwrap();
        assert_eq!(hit.evidence[0].spans, vec![MatchSpan { start: 0, end: 1 }]);
    }

    #[test]
    fn canonical_unicode_executes_through_real_page_alias_and_block_projections() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "tine-query-plan-unicode-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::create_dir_all(dir.join("journals")).unwrap();
        fs::create_dir_all(dir.join("logseq")).unwrap();
        fs::write(
            dir.join("pages").join("Cafe\u{301}.md"),
            "alias:: Re\u{301}sume\u{301}\n\n- Re\u{301}sume\u{301}\n",
        )
        .unwrap();
        let graph = Graph::open(&dir);
        graph.warm_cache();

        let page = QueryPlan::page_name_fuzzy("Café", 8).execute(&graph, || false);
        assert!(matches!(
            page.hits.first(),
            Some(QueryHit::Page {
                page,
                match_class: ObjectiveMatchClass::Exact,
                matched_alias: None,
                evidence,
                ..
            }) if page.name == "Cafe\u{301}"
                && evidence[0].spans == vec![MatchSpan { start: 0, end: 5 }]
        ));

        let alias = QueryPlan::page_name_fuzzy("Résumé", 8).execute(&graph, || false);
        assert!(
            matches!(
                alias.hits.first(),
                Some(QueryHit::Page {
                    page,
                    match_class: ObjectiveMatchClass::Exact,
                    matched_alias: Some(name),
                    ..
                }) if page.name == "Cafe\u{301}" && name == "résumé"
            ),
            "{:#?}",
            alias.hits
        );

        let blocks = QueryPlan::block_search("Résumé", 8).execute(&graph, || false);
        assert!(matches!(
            blocks.hits.first(),
            Some(QueryHit::Block { display_text, evidence, .. })
                if display_text == "Re\u{301}sume\u{301}"
                    && evidence[0].spans == vec![MatchSpan { start: 0, end: 8 }]
        ));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn page_alias_search_is_scoped_to_its_physical_owner() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "tine-query-plan-alias-owner-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(dir.join("pages").join("sub")).unwrap();
        fs::create_dir_all(dir.join("journals")).unwrap();
        fs::create_dir_all(dir.join("logseq")).unwrap();
        fs::write(
            dir.join("pages").join("Foo.md"),
            "alias:: bar\n\n- declaring page\n",
        )
        .unwrap();
        fs::write(
            dir.join("pages").join("sub").join("Foo.md"),
            "- same-named sibling\n",
        )
        .unwrap();
        fs::write(
            dir.join("pages").join("Unique.md"),
            "alias:: quux\n\n- unique alias owner\n",
        )
        .unwrap();
        let graph = Graph::open(&dir);
        graph.warm_cache();

        let alias_hits = graph
            .run_graph_search("bar", 10, 0, false)
            .hits
            .into_iter()
            .filter_map(|hit| match hit {
                QueryHit::Page {
                    page,
                    match_class,
                    matched_alias,
                    ..
                } if !page.rel_path.is_empty() => {
                    Some((page.rel_path, match_class, matched_alias))
                }
                QueryHit::Page { .. } => None,
                QueryHit::Block { .. } => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            alias_hits,
            vec![(
                "pages/Foo.md".to_string(),
                ObjectiveMatchClass::Exact,
                Some("bar".to_string()),
            )],
            "a duplicate-named sibling must not inherit another file's alias"
        );

        let unique_hits = graph
            .run_graph_search("quux", 10, 0, false)
            .hits
            .into_iter()
            .filter_map(|hit| match hit {
                QueryHit::Page {
                    page,
                    match_class,
                    matched_alias,
                    ..
                } if !page.rel_path.is_empty() => {
                    Some((page.rel_path, match_class, matched_alias))
                }
                QueryHit::Page { .. } => None,
                QueryHit::Block { .. } => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            unique_hits,
            vec![(
                "pages/Unique.md".to_string(),
                ObjectiveMatchClass::Exact,
                Some("quux".to_string()),
            )],
            "a unique page name must retain ordinary alias matching"
        );

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn boolean_evidence_uses_positive_terms_and_first_matching_or_branch() {
        let plan = QueryPlan::friendly("foo -draft OR ready", 8, 8);
        let blocks = &plan.branches[1].predicate;
        let first = eval_expr(&plan, blocks, TextField::VisibleContent, "foo ship").unwrap();
        assert_eq!(first.evidence.len(), 1);
        assert_eq!(first.evidence[0].mode, TextMatchMode::Contains);
        assert!(eval_expr(&plan, blocks, TextField::VisibleContent, "foo draft").is_none());
        let second = eval_expr(&plan, blocks, TextField::VisibleContent, "ready draft").unwrap();
        assert_eq!(second.evidence.len(), 1);
        assert_ne!(first.evidence[0].clause_id, second.evidence[0].clause_id);
    }

    #[test]
    fn invalid_regex_is_a_rust_diagnostic_and_matches_nothing() {
        let plan = QueryPlan::friendly("/(unclosed/", 8, 8);
        assert!(plan.branches.is_empty());
        assert_eq!(plan.diagnostics.len(), 1);
        assert_eq!(plan.diagnostics[0].code, "invalid_regex");
        assert_eq!(
            plan.diagnostics[0].span,
            Some(MatchSpan { start: 0, end: 11 })
        );
    }

    #[test]
    fn zero_limit_friendly_plans_still_classify_saveable_sources() {
        let rust_only_invalid = QueryPlan::friendly("/(a)\\1/", 0, 0);
        assert!(rust_only_invalid.branches.is_empty());
        assert_eq!(
            rust_only_invalid
                .diagnostics
                .first()
                .map(|item| item.code.as_str()),
            Some("invalid_regex")
        );
        let valid = QueryPlan::friendly("alpha", 0, 0);
        assert!(!valid.branches.is_empty());
        assert!(valid.branches.iter().all(|branch| branch.limit == 0));
        let excluded = QueryPlan::friendly("-draft", 0, 0);
        assert!(excluded.diagnostics.is_empty() && excluded.branches.is_empty());
    }

    #[test]
    fn pure_negation_and_empty_friendly_search_have_no_branches() {
        assert!(QueryPlan::friendly("-draft", 8, 8).branches.is_empty());
        assert!(QueryPlan::friendly("", 8, 8).branches.is_empty());
        assert_eq!(QueryPlan::legacy_page_search("-draft", 8).branches.len(), 1);
    }

    #[test]
    fn matching_modes_are_explicit() {
        let pred = text_pred(TextMatchMode::Fuzzy, "abc");
        assert_eq!(pred.mode, TextMatchMode::Fuzzy);
        let plan = QueryPlan::friendly("\"exact phrase\"", 8, 8);
        assert!(matches!(
            &plan.branches[1].predicate,
            QueryExpr::Text(TextPredicate {
                mode: TextMatchMode::Phrase,
                ..
            })
        ));
    }

    #[test]
    fn combined_execution_returns_typed_ranked_hits_and_exact_evidence_text() {
        let (dir, graph) = fixture();
        let pages = QueryPlan::page_name_fuzzy("opdf", 10).execute(&graph, || false);
        let names = pages
            .hits
            .iter()
            .filter_map(|hit| match hit {
                QueryHit::Page { page, .. } => Some(page.name.as_str()),
                QueryHit::Block { .. } => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(names.first().copied(), Some("Opdf Notes"));
        assert!(
            names.iter().position(|name| *name == "Xopdf")
                < names.iter().position(|name| *name == "Opinion Diffusion")
        );
        let file_hit = pages.hits.iter().find_map(|hit| match hit {
            QueryHit::Page { page, .. } if page.name == "Opdf Notes" => Some(page),
            _ => None,
        });
        assert_eq!(file_hit.unwrap().rel_path, "pages/Opdf Notes.md");
        let virtual_hit = pages.hits.iter().find_map(|hit| match hit {
            QueryHit::Page { page, .. } if page.name == "Virtual Opdf" => Some(page),
            _ => None,
        });
        assert_eq!(virtual_hit.unwrap().rel_path, "");

        let execution = graph.run_graph_search("foo -draft OR ready", 10, 10, true);
        let blocks = execution
            .hits
            .iter()
            .filter_map(|hit| match hit {
                QueryHit::Block {
                    block,
                    display_text,
                    evidence,
                    ..
                } => Some((block, display_text, evidence)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(blocks.len(), 2);
        let nested = blocks
            .iter()
            .find(|(_, text, _)| text.as_str() == "🧠 foo ready")
            .unwrap();
        assert_eq!(nested.0.breadcrumb, vec!["Parent"]);
        assert_eq!(nested.2.len(), 1, "successful NOT has no positive evidence");
        assert_eq!(nested.2[0].spans, vec![MatchSpan { start: 3, end: 6 }]);
        assert!(!execution.explanation.branches.is_empty());
        let explanation = serde_json::to_string(&execution.explanation).unwrap();
        assert!(explanation.contains("Block text"));
        assert!(explanation.contains("contains “foo”"));
        assert!(!explanation.contains("PageName"));
        assert!(!explanation.contains("VisibleContent"));

        let no_explain = graph.run_graph_search("foo", 10, 10, false);
        assert!(no_explain.explanation.branches.is_empty());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn current_page_scope_is_block_only_and_path_authoritative() {
        let (dir, graph) = fixture();
        fs::create_dir_all(dir.join("pages").join("duplicate")).unwrap();
        fs::write(
            dir.join("pages")
                .join("duplicate")
                .join("Opinion Diffusion.md"),
            "- duplicate foo\n",
        )
        .unwrap();
        graph.warm_cache();

        let execution = QueryPlan::friendly_for_page(
            "foo",
            50,
            QueryPageScope {
                name: "Opinion Diffusion".into(),
                page_kind: PageKind::Page,
                path: Some("pages/Opinion Diffusion.md".into()),
            },
        )
        .execute(&graph, || false);
        assert!(!execution.hits.is_empty());
        assert!(execution.hits.iter().all(|hit| matches!(
            hit,
            QueryHit::Block { page, path, block, .. }
                if page == "Opinion Diffusion"
                    && path == "pages/Opinion Diffusion.md"
                    && block.raw != "duplicate foo"
        )));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn page_hits_expose_objective_classes_and_alias_evidence() {
        let (dir, graph) = fixture();
        let exact = QueryPlan::page_name_fuzzy("Opdf Notes", 10).execute(&graph, || false);
        assert!(matches!(
            exact.hits.first(),
            Some(QueryHit::Page {
                match_class: ObjectiveMatchClass::Exact,
                matched_alias: None,
                ..
            })
        ));

        let alias = QueryPlan::page_name_fuzzy("Research Hub", 10).execute(&graph, || false);
        let hit = alias
            .hits
            .iter()
            .find(|hit| matches!(hit, QueryHit::Page { page, .. } if page.name == "Opdf Notes"));
        assert!(matches!(
            hit,
            Some(QueryHit::Page {
                display_text,
                match_class: ObjectiveMatchClass::Exact,
                matched_alias: Some(matched_alias),
                ..
            }) if display_text == "research hub" && matched_alias == "research hub"
        ));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn regex_evidence_is_authoritative_and_bounded_to_projected_text() {
        let (dir, graph) = fixture();
        let execution = graph.run_graph_search("/[A-Z]{3}/", 10, 10, true);
        let (text, evidence) = execution
            .hits
            .iter()
            .find_map(|hit| match hit {
                QueryHit::Block {
                    display_text,
                    evidence,
                    ..
                } if display_text == "regex ABC" => Some((display_text, evidence)),
                _ => None,
            })
            .unwrap();
        assert_eq!(text, "regex ABC");
        assert_eq!(evidence[0].spans, vec![MatchSpan { start: 6, end: 9 }]);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn legacy_block_search_adapter_preserves_matcher_membership_and_ranked_topk() {
        let (dir, graph) = fixture();
        for query in [
            "",
            "foo",
            "foo ready",
            "foo OR ready",
            "foo -draft",
            "-draft",
            "\"foo ready\"",
            "/[A-Z]{3}/",
            "/(unclosed/",
        ] {
            let full = block_fingerprint(crate::query::search(&graph, query, usize::MAX));
            let mut full_membership = full.clone();
            full_membership.sort();
            let mut reference = reference_search(&graph, query, usize::MAX);
            reference.sort();
            assert_eq!(full_membership, reference, "query={query:?}");
            for limit in [0, 1, 2, 20] {
                assert_eq!(
                    block_fingerprint(crate::query::search(&graph, query, limit)),
                    full.iter().take(limit).cloned().collect::<Vec<_>>(),
                    "query={query:?} limit={limit}"
                );
            }
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn query_hit_json_contract_uses_tagged_entities_and_utf16_evidence() {
        let hit = QueryHit::Page {
            page: PageEntry {
                name: "🧠 Foo".into(),
                kind: PageKind::Page,
                date_key: None,
                rel_path: "pages/Foo.md".into(),
                path: std::path::PathBuf::new(),
            },
            display_text: "🧠 Foo".into(),
            evidence: vec![MatchEvidence {
                clause_id: 7,
                field: TextField::PageName,
                mode: TextMatchMode::Fuzzy,
                spans: vec![MatchSpan { start: 3, end: 6 }],
                score: None,
            }],
            score: 1_000,
            match_class: ObjectiveMatchClass::Prefix,
            matched_alias: None,
        };

        assert_eq!(
            serde_json::to_value(hit).unwrap(),
            serde_json::json!({
                "entity": "page",
                "page": {
                    "name": "🧠 Foo",
                    "kind": "page",
                    "date_key": null,
                    "path": "pages/Foo.md"
                },
                "display_text": "🧠 Foo",
                "evidence": [{
                    "clause_id": 7,
                    "field": "page_name",
                    "mode": "fuzzy",
                    "spans": [{"start": 3, "end": 6}]
                }],
                "score": 1_000,
                "match_class": "prefix"
            })
        );
    }

    #[test]
    fn cancellation_discards_all_partial_combined_results() {
        let (dir, graph) = fixture();
        let checks = Cell::new(0usize);
        let execution = QueryPlan::friendly("unrelated", 10, 10).execute(&graph, || {
            checks.set(checks.get() + 1);
            checks.get() > 3
        });
        assert!(execution.cancelled);
        assert!(execution.hits.is_empty());
        fs::remove_dir_all(dir).unwrap();
    }
}
