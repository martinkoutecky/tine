//! Typed execution plans for page-name and block-text graph search.
//! This covers friendly search expressions and exact page scopes; `{{query}}`
//! expressions use the separate query evaluator.

use crate::model::ReadSnapshot;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};
use tine_core::doc::DocBlock;
use tine_core::model::{PageEntry, PageKind};
use tine_core::query::ir::FriendlyPageMatchScope;
use tine_core::query::ir::{SortDir, ViewSettings};
use tine_core::query::sort::{compare_sort_decorations, SortDecor};
use tine_core::query_plan::{
    ExplainNode, MatchEvidence, MatchSpan, ObjectiveMatchClass, QueryDiagnostic, QueryExecution,
    QueryExplanation, QueryHasMore, QueryHit, TextField, TextMatchMode,
};
use tine_core::refs;
use tine_core::refs::page_key;
use tine_core::search_query::{canonical_fold, literal_fold, Matcher, Term};
pub(crate) mod completion;

#[cfg(test)]
thread_local! { pub(crate) static COMPLETION_SCORES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
#[cfg(test)]
thread_local! {
    /// (admissions, peak retained) of the last block selection: how many
    /// candidates paid for an ancestor path, and the most any moment held (PC-3).
    pub(crate) static BLOCK_SELECTION: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
}
#[cfg(test)]
thread_local! {
    /// The same for the last page selection.
    pub(crate) static PAGE_SELECTION: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
}
#[cfg(test)]
mod completion_bench_tests;
#[cfg(test)]
mod completion_tests;
#[cfg(test)]
mod sort_tests;

const MAX_EVIDENCE_SPANS: usize = 32;

/// The entity kind a query-plan branch selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum QueryTarget {
    Pages,
    Blocks,
}

/// A typed text predicate in the shared plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TextPredicate {
    pub(crate) clause_id: u32,
    pub(crate) field: TextField,
    pub(crate) mode: TextMatchMode,
    pub(crate) value: String,
}

/// Boolean query expression.  A successful NOT contributes no positive match
/// evidence, which keeps "why did this match?" explanations honest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub(crate) enum QueryExpr {
    Text(TextPredicate),
    And(Vec<QueryExpr>),
    Or(Vec<QueryExpr>),
    Not(Box<QueryExpr>),
    Never,
}

/// One independently limited entity branch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct QueryBranch {
    pub(crate) target: QueryTarget,
    pub(crate) predicate: QueryExpr,
    pub(crate) limit: usize,
}

/// One routed page used to scope a block-search plan. A supplied relative path
/// is authoritative so duplicate display identities do not leak into results;
/// otherwise kind plus Logseq's canonical page identity selects the document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QueryPageScope {
    pub(crate) name: String,
    pub(crate) page_kind: PageKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) path: Option<String>,
}

/// Compiled friendly graph-search plan.  Regexes are compiled once and kept off
/// the wire; the public expression remains inspectable/serializable.
#[derive(Debug, Clone)]
pub(crate) struct QueryPlan {
    pub(crate) branches: Vec<QueryBranch>,
    pub(crate) diagnostics: Vec<QueryDiagnostic>,
    page_scope: Option<QueryPageScope>,
    // Ctrl-K keeps the literal trimmed launcher source so a multi-word page
    // title/alias can retain the objective Exact class. The parsed AND terms
    // alone would otherwise downgrade `Foo Bar` to Prefix/Substring and make
    // the frontend offer a duplicate Create row beside the existing page.
    page_exact: Option<String>,
    // Ctrl-K/friendly search includes user properties; reference completion
    // keeps its existing body-only, referenceable-block surface.
    include_user_properties: bool,
    regexes: HashMap<u32, Regex>,
    // Compiled `Words` predicates, so the scan builds no searcher per block.
    // (clause id, matcher): at most two entries, scanned linearly because a
    // hashed lookup per block would cost about as much as the match itself.
    words: Vec<(u32, WordsMatcher)>,
    fragments: Vec<(u32, FragmentsMatcher)>,
    remove_accents: bool,
    pub(crate) transliteration: bool,
    transliteration_queries: HashMap<u32, String>,
    pub(crate) name_keys: Option<std::sync::Arc<crate::model::NameKeyMap>>,
    page_match_scope: FriendlyPageMatchScope,
    page_view: Option<ViewSettings>,
    block_view: Option<ViewSettings>,
    // Inline completion lists spellings, while graph search lists identities.
    pub(crate) separate_aliases: bool,
    pub(crate) editor_reference_baseline: Option<(String, String, String)>,
}

impl QueryPlan {
    pub(crate) fn with_transliteration(mut self, enabled: bool) -> Self {
        self.transliteration = enabled;
        if enabled {
            fn collect(expr: &QueryExpr, queries: &mut HashMap<u32, String>) {
                match expr {
                    QueryExpr::Text(pred)
                        if pred.field == TextField::PageName
                            && pred.mode != TextMatchMode::Regex =>
                    {
                        if let Some(query) = crate::transliteration::normalize_query(&pred.value) {
                            queries.insert(pred.clause_id, query);
                        }
                    }
                    QueryExpr::And(children) | QueryExpr::Or(children) => {
                        for child in children {
                            collect(child, queries);
                        }
                    }
                    QueryExpr::Not(child) => collect(child, queries),
                    _ => {}
                }
            }
            for branch in &self.branches {
                collect(&branch.predicate, &mut self.transliteration_queries);
            }
        }
        self
    }

    fn transliteration_score(
        &self,
        original: &str,
        pred: &TextPredicate,
    ) -> Option<(i32, ObjectiveMatchClass)> {
        let names = self.name_keys.as_ref()?;
        let query = self.transliteration_queries.get(&pred.clause_id)?;
        let key = crate::transliteration::cache_identity(original)?;
        names
            .get(&key)?
            .score(query)
            .map(|score| (score, ObjectiveMatchClass::Transliteration))
    }

    fn words_matcher(&self, pred: &TextPredicate) -> std::borrow::Cow<'_, WordsMatcher> {
        match self.words.iter().find(|(id, _)| *id == pred.clause_id) {
            Some((_, matcher)) => std::borrow::Cow::Borrowed(matcher),
            None => std::borrow::Cow::Owned(WordsMatcher::new(&pred.value)),
        }
    }

    fn fragments_matcher(&self, pred: &TextPredicate) -> std::borrow::Cow<'_, FragmentsMatcher> {
        match self.fragments.iter().find(|(id, _)| *id == pred.clause_id) {
            Some((_, matcher)) => std::borrow::Cow::Borrowed(matcher),
            None => std::borrow::Cow::Owned(FragmentsMatcher::new(&pred.value)),
        }
    }

    fn fold(&self, value: &str) -> String {
        if self.remove_accents {
            canonical_fold(value)
        } else {
            literal_fold(value)
        }
    }
    /// Ctrl-K graph providers: fuzzy page names for a single bare term, but
    /// ordinary contains/phrase/regex semantics for block content.  Multi-term
    /// and operator searches use the same boolean grammar on both entity kinds.
    /// Test-only default-policy shorthand; production passes the graph policy.
    #[cfg(test)]
    pub(crate) fn friendly(query: &str, page_limit: usize, block_limit: usize) -> Self {
        Self::friendly_with_policy(query, page_limit, block_limit, true)
    }

    /// The production constructor; `remove_accents` comes from the graph config.
    #[cfg(test)]
    pub(crate) fn friendly_with_policy(
        query: &str,
        page_limit: usize,
        block_limit: usize,
        remove_accents: bool,
    ) -> Self {
        Self::friendly_with_scope(
            query,
            page_limit,
            block_limit,
            remove_accents,
            FriendlyPageMatchScope::Names,
        )
    }

    /// Graph-scale friendly search with explicit page membership. Names is the
    /// historical default; Content admits physical owners of matching blocks.
    pub(crate) fn friendly_with_scope(
        query: &str,
        page_limit: usize,
        block_limit: usize,
        remove_accents: bool,
        page_match_scope: FriendlyPageMatchScope,
    ) -> Self {
        let matcher = Matcher::parse_with_policy(query, remove_accents);
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
            page_exact: (!query.trim().is_empty()).then(|| page_key(query)),
            include_user_properties: true,
            regexes,
            words: Vec::new(),
            fragments: Vec::new(),
            remove_accents,
            transliteration: false,
            name_keys: None,
            transliteration_queries: HashMap::new(),
            page_match_scope,
            page_view: None,
            block_view: None,
            separate_aliases: false,
            editor_reference_baseline: None,
        }
    }

    /// Current-page search is a block-only execution profile of the same typed
    /// friendly plan—not a frontend filter over whole-graph results.
    /// Test-only default-policy shorthand; production passes the graph policy.
    #[cfg(test)]
    pub(crate) fn friendly_for_page(
        query: &str,
        block_limit: usize,
        scope: QueryPageScope,
    ) -> Self {
        let mut plan = Self::block_search(query, block_limit);
        plan.page_scope = Some(scope);
        plan
    }

    /// Current-page search with the graph's explicit accent policy.
    pub(crate) fn friendly_for_page_with_policy(
        query: &str,
        block_limit: usize,
        scope: QueryPageScope,
        remove_accents: bool,
    ) -> Self {
        let mut plan = Self::block_search_with_policy(query, block_limit, remove_accents);
        plan.page_scope = Some(scope);
        plan
    }

    /// Explicit page-name fuzzy plan for normal-query frontends and tests.  This
    /// constructor makes the opt-in visible in the typed IR; it never changes the
    /// default behavior of existing block queries.
    /// Test-only default-policy shorthand; production passes the graph policy.
    #[cfg(test)]
    pub(crate) fn page_name_fuzzy(value: impl Into<String>, limit: usize) -> Self {
        Self::page_name_fuzzy_with_policy(value, limit, true)
    }

    /// Page-name search with the graph's explicit accent policy.
    pub(crate) fn page_name_fuzzy_with_policy(
        value: impl Into<String>,
        limit: usize,
        remove_accents: bool,
    ) -> Self {
        let value = value.into();
        Self {
            branches: vec![QueryBranch {
                target: QueryTarget::Pages,
                predicate: QueryExpr::Text(TextPredicate {
                    clause_id: 1,
                    field: TextField::PageName,
                    mode: TextMatchMode::Fuzzy,
                    value: if remove_accents {
                        canonical_fold(&value)
                    } else {
                        literal_fold(&value)
                    },
                }),
                limit,
            }],
            diagnostics: Vec::new(),
            page_scope: None,
            page_exact: Some(page_key(&value)),
            include_user_properties: false,
            regexes: HashMap::new(),
            words: Vec::new(),
            fragments: Vec::new(),
            remove_accents,
            transliteration: false,
            name_keys: None,
            transliteration_queries: HashMap::new(),
            page_match_scope: FriendlyPageMatchScope::Names,
            page_view: None,
            block_view: None,
            separate_aliases: false,
            editor_reference_baseline: None,
        }
    }

    /// Existing block-search API expressed as one typed branch.
    /// Test-only default-policy shorthand; production passes the graph policy.
    #[cfg(test)]
    pub(crate) fn block_search(query: &str, limit: usize) -> Self {
        Self::block_search_with_policy(query, limit, true)
    }

    /// Block search with the graph's explicit accent policy.
    pub(crate) fn block_search_with_policy(
        query: &str,
        limit: usize,
        remove_accents: bool,
    ) -> Self {
        let matcher = Matcher::parse_with_policy(query, remove_accents);
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
            include_user_properties: false,
            regexes,
            words: Vec::new(),
            fragments: Vec::new(),
            remove_accents,
            transliteration: false,
            name_keys: None,
            transliteration_queries: HashMap::new(),
            page_match_scope: FriendlyPageMatchScope::Names,
            page_view: None,
            block_view: None,
            separate_aliases: false,
            editor_reference_baseline: None,
        }
    }

    /// `((` autocomplete, matching OG's `search-blocks` (`electron/search.cljs`):
    /// fold the whole query (OG's `search-normalize` on both sides), then list
    /// first the blocks holding the typed words as whole tokens in any order
    /// (its FTS5 leg), then those holding the whitespace-separated pieces
    /// literally in order (its `LIKE` leg), without Ctrl-K's Boolean grammar and
    /// without scattering a piece's letters. Blank input has no candidates.
    pub(crate) fn block_search_literal_with_policy(
        query: &str,
        limit: usize,
        remove_accents: bool,
    ) -> Self {
        let folded = if remove_accents {
            canonical_fold(query)
        } else {
            literal_fold(query)
        };
        let mut words = Vec::new();
        let mut fragments_by_clause = Vec::new();
        let branches = if folded.split_whitespace().next().is_none() {
            Vec::new()
        } else {
            // OG's `search-blocks` unions an FTS5 `MATCH` (whole words, any
            // order) with `LIKE '%a%b%'` (literal pieces, in order), full-text
            // hits first. One scan evaluates both legs per block.
            fragments_by_clause.push((1, FragmentsMatcher::new(&folded)));
            let fragments = QueryExpr::Text(TextPredicate {
                clause_id: 1,
                field: TextField::VisibleContent,
                mode: TextMatchMode::Fragments,
                value: folded.clone(),
            });
            let predicate = match words_value(&folded) {
                Some(value) => {
                    words.push((2, WordsMatcher::new(&value)));
                    QueryExpr::Or(vec![
                        QueryExpr::Text(TextPredicate {
                            clause_id: 2,
                            field: TextField::VisibleContent,
                            mode: TextMatchMode::Words,
                            value,
                        }),
                        fragments,
                    ])
                }
                None => fragments,
            };
            vec![QueryBranch {
                target: QueryTarget::Blocks,
                predicate,
                limit,
            }]
        };
        Self {
            branches,
            diagnostics: Vec::new(),
            page_scope: None,
            page_exact: None,
            include_user_properties: false,
            regexes: HashMap::new(),
            words,
            fragments: fragments_by_clause,
            remove_accents,
            transliteration: false,
            name_keys: None,
            transliteration_queries: HashMap::new(),
            page_match_scope: FriendlyPageMatchScope::Names,
            page_view: None,
            block_view: None,
            separate_aliases: false,
            editor_reference_baseline: None,
        }
    }

    /// Literal autocomplete for `#`/`[[`. OG rev 6e7afa8eb routes
    /// `handler/editor.cljs:get-matched-pages` through
    /// `search.cljs:page-search`/`exact-matched?`; the whole normalized query is
    /// one ordered-subsequence term, not Ctrl-K's AND/OR/negation/regex DSL.
    /// Blank input therefore keeps the established all-pages candidate listing.
    /// Test-only default-policy shorthand; production passes the graph policy.
    #[cfg(test)]
    pub(crate) fn legacy_page_search(query: &str, limit: usize) -> Self {
        Self::legacy_page_search_with_policy(query, limit, true)
    }

    /// Page picker with the graph's explicit accent policy.
    pub(crate) fn legacy_page_search_with_policy(
        query: &str,
        limit: usize,
        remove_accents: bool,
    ) -> Self {
        let mut plan = Self::page_name_fuzzy_with_policy(query, limit, remove_accents);
        plan.separate_aliases = true;
        plan
    }

    pub(crate) fn explanation(&self) -> QueryExplanation {
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

    /// Apply independent Display sections before search admission. Sampling
    /// caps each section's own limit; a missing view retains relevance order.
    /// Cost O(view fields); unsupported recency fields are ignored by execution.
    pub(crate) fn with_display(
        mut self,
        page: Option<ViewSettings>,
        block: Option<ViewSettings>,
    ) -> Self {
        for branch in &mut self.branches {
            let view = match branch.target {
                QueryTarget::Pages => page.as_ref(),
                QueryTarget::Blocks => block.as_ref(),
            };
            if let Some(sample) = view.and_then(|view| view.sample) {
                branch.limit = branch.limit.min(sample as usize);
            }
        }
        self.page_view = page;
        self.block_view = block;
        self
    }

    /// Execute all graph-backed branches.  Cancellation is checked between page
    /// candidates and before every block projection; no partial result escapes.
    pub(crate) fn execute(
        &self,
        graph: &ReadSnapshot,
        cancelled: impl Fn() -> bool,
    ) -> QueryExecution {
        self.execute_with_explain(graph, cancelled, true)
    }

    pub(crate) fn execute_with_explain(
        &self,
        graph: &ReadSnapshot,
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
        if self.transliteration
            && self.name_keys.is_none()
            && !graph.transliteration_names.is_empty()
        {
            let mut expanded = self.clone();
            expanded.name_keys = Some(std::sync::Arc::clone(&graph.transliteration_names));
            return expanded.execute_with_explain(graph, cancelled, explain);
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

/// Supported Friendly sort fields, with authored directions. The master's
/// Friendly reader omits recency because its snapshot carries no recency axis.
fn friendly_sort_fields(view: Option<&ViewSettings>) -> Vec<(&str, bool)> {
    view.into_iter()
        .flat_map(|view| &view.sort)
        .filter_map(|(field, direction)| {
            let field = field.as_str();
            (!matches!(
                field.to_ascii_lowercase().as_str(),
                "modified" | "updated" | "updated-at" | "date"
            ))
            .then_some((field, *direction == SortDir::Asc))
        })
        .collect()
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
    if term.text.is_empty() {
        return if term.negated {
            QueryExpr::Not(Box::new(QueryExpr::Never))
        } else {
            QueryExpr::Never
        };
    }
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
                TextMatchMode::Fragments => {
                    format!("contains the fragments “{}” in order", p.value)
                }
                TextMatchMode::Words => format!(
                    "contains the whole words “{}” in any order",
                    p.value.replace('\n', "”, “")
                ),
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
            TextMatchMode::Fragments => plan.fragments_matcher(pred).first_offset(lower).is_some(),
            TextMatchMode::Words => plan.words_matcher(pred).first_offset(lower).is_some(),
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
            let spans = casefold_substring_spans(original, &pred.value, plan.remove_accents);
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
            let spans = tine_core::search_query::regex_spans(re, original, MAX_EVIDENCE_SPANS)
                .into_iter()
                .map(|span| MatchSpan {
                    start: span.start,
                    end: span.end,
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
        TextMatchMode::Fuzzy => fuzzy_evidence(pred, original, plan.remove_accents),
        TextMatchMode::Fragments => fragments_evidence(pred, original, plan.remove_accents),
        TextMatchMode::Words => words_evidence(pred, original, plan.remove_accents),
    }
}

#[path = "query_plan/fold.rs"]
mod fold;
use fold::{casefold_substring_spans, fragments_evidence, fuzzy_evidence, words_evidence};
mod blocks;
mod text;
use text::search_text;
#[path = "query_plan/picker_legs.rs"]
mod picker_legs;
use picker_legs::{words_value, FragmentsMatcher, WordsMatcher};
#[path = "query_plan/pages.rs"]
mod pages;
mod subscription;
use blocks::{execute_blocks, text_predicate_relevance};
#[cfg(test)]
use pages::best_page_match;
use pages::execute_pages;

/// The highest `BlockRelevance::leg`: the `((` picker's full-text leg.
const TOP_LEG: u8 = 1;

#[derive(Debug, Clone, Copy)]
struct BlockRelevance {
    /// Which OG leg listed the block: 1 for the `((` picker's full-text leg,
    /// which OG lists before every fragment-only block; 0 everywhere else.
    leg: u8,
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
            leg: 0,
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
        self.leg
            .cmp(&other.leg)
            .then_with(|| self.match_class.rank().cmp(&other.match_class.rank()))
            .then_with(|| self.word_boundary.cmp(&other.word_boundary))
            .then_with(|| other.first_offset.cmp(&self.first_offset))
            .then_with(|| other.text_len.cmp(&self.text_len))
            .then_with(|| other.occurrences.cmp(&self.occurrences))
    }

    fn score(self) -> i32 {
        let offset_penalty = self.first_offset.min(50_000) as i32;
        let length_penalty = self.text_len.min(40_000) as i32;
        let occurrence_penalty = self.occurrences.saturating_sub(1).min(9_999) as i32;
        i32::from(self.leg) * 10_000_000
            + self.match_class.rank() * 1_000_000
            + i32::from(self.word_boundary) * 100_000
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
    is_comment: bool,
    /// The authored sort keys of a sorted search, with their directions in
    /// `ascending`. With no sort, `ascending` is empty and relevance orders.
    keys: Vec<SortDecor>,
    ascending: &'a [bool],
    /// Borrowed, so a candidate costs one pointer per ancestor; the labels are
    /// built for winners only. Filled by [`push_block`] on admission: a
    /// candidate that loses copies nothing (audit L7s3-02).
    ancestors: Vec<&'a DocBlock>,
}

impl PartialEq for ScoredBlock<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for ScoredBlock<'_> {}
impl PartialOrd for ScoredBlock<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for ScoredBlock<'_> {
    /// The result order: "less" is better. The max-heap root is therefore the
    /// WORST retained candidate, ready for eviction, and the final sort of the
    /// winners is this same comparator. The path and position tie is never
    /// reversed by a descending sort key.
    fn cmp(&self, other: &Self) -> Ordering {
        let by = if self.ascending.is_empty() {
            other.relevance.cmp_quality(&self.relevance)
        } else {
            compare_sort_decorations(&self.keys, &other.keys, self.ascending)
        };
        by.then_with(|| {
            (self.page.rel_path_str(), self.index).cmp(&(other.page.rel_path_str(), other.index))
        })
    }
}

/// Keep the best `limit` candidates. `path` (the candidate's ancestors) is
/// copied only if the candidate is kept.
fn push_block<'a>(
    heap: &mut BinaryHeap<ScoredBlock<'a>>,
    limit: usize,
    mut candidate: ScoredBlock<'a>,
    path: &[&'a DocBlock],
) {
    let full = heap.len() >= limit;
    if full && !heap.peek().is_some_and(|worst| candidate < *worst) {
        return;
    }
    candidate.ancestors = path.to_vec();
    if full {
        *heap.peek_mut().unwrap() = candidate;
    } else {
        heap.push(candidate);
    }
    #[cfg(test)]
    BLOCK_SELECTION.with(|c| c.set((c.get().0 + 1, c.get().1.max(heap.len()))));
}

fn starts_at_word_boundary(value: &str, byte_offset: usize) -> bool {
    byte_offset == 0
        || value[..byte_offset]
            .chars()
            .next_back()
            .is_none_or(|ch| !ch.is_alphanumeric() && ch != '_')
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
                combined.leg = combined.leg.min(relevance.leg);
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
                if relevance.leg == TOP_LEG {
                    // `leg` outranks every other component, so no later branch
                    // can beat this one; skip its (second) rank-length pass.
                    break;
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

fn walk_blocks<'a>(
    blocks: &'a [DocBlock],
    ancestors: &mut Vec<&'a DocBlock>,
    in_comment: bool,
    visit: &mut impl FnMut(&'a DocBlock, &[&'a DocBlock], bool) -> bool,
) -> bool {
    for block in blocks {
        let is_comment = in_comment
            || (!ancestors.is_empty()
                && tine_outline::editor_text::comment_quote(
                    block
                        .projection()
                        .properties()
                        .iter()
                        .map(|(key, value)| (key.as_str(), value.as_str())),
                )
                .is_some());
        #[cfg(test)]
        COMMENT_MEMBERSHIP_VISITS.with(|count| count.set(count.get() + 1));
        if !visit(block, ancestors, is_comment) {
            return false;
        }
        ancestors.push(block);
        let keep_going = walk_blocks(&block.children, ancestors, is_comment, visit);
        ancestors.pop();
        if !keep_going {
            return false;
        }
    }
    true
}

#[cfg(test)]
thread_local! {
    static COMMENT_MEMBERSHIP_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Convert typed block hits back to the exact grouped shape used by existing
/// search/query consumers. Hits arrive in global relevance order; only contiguous
/// hits from the same page are coalesced, so flattening the groups preserves that
/// order even when a page appears in more than one group.
pub(crate) fn block_hits_to_groups(hits: Vec<QueryHit>) -> Vec<tine_core::model::RefGroup> {
    let mut groups: Vec<tine_core::model::RefGroup> = Vec::new();
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
        groups.push(tine_core::model::RefGroup {
            page,
            kind,
            blocks: vec![block],
            evidence: Vec::new(),
        });
    }
    groups
}

#[cfg(test)]
pub(crate) fn page_hits_to_entries(hits: Vec<QueryHit>) -> Vec<PageEntry> {
    hits.into_iter()
        .filter_map(|hit| match hit {
            // An authored alias is offered under its own spelling on its
            // owner's identity (path), never as a pathless page (master GH
            // #482 / #353: the alias row keeps `rel_path`, only `name` is the
            // alias text).
            QueryHit::Page {
                mut page,
                matched_alias,
                ..
            } => {
                if let Some(alias) = matched_alias {
                    page.name = alias;
                }
                Some(page)
            }
            QueryHit::Block { .. } => None,
        })
        .collect()
}

#[cfg(test)]
mod tests;
