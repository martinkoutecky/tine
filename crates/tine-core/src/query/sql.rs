//! The ONE IR → SQL lowering (SPEC §5.1–§5.7).
//!
//! **What this module is.** It turns a [`Query`] — the same value the walk
//! evaluates in [`crate::query::eval`] — into one statement plus its bound
//! parameters, to be run through `tine-storage`'s read-only projection
//! statement seam (`PhysicalProjectionQueryReader`, D-15). It is a compiler and
//! nothing else: it opens no database, holds no connection, and has no opinion
//! about which backend runs the statement.
//!
//! **Transcription (D-9, D-14).** Each function below names the upstream
//! function it transcribes, read from the upstream source rather than inferred
//! from behaviour:
//!
//! | Tine function | Upstream source |
//! |---|---|
//! | [`Compiler::filter`] | `hasura/ndc-postgres` `crates/query-engine/translation/src/translation/query/filtering.rs::translate_expression_with_joins` — the `And`/`Or`/`Not` skeleton folded over already-translated operand expressions, with an empty `In` list short-circuited to a false constant. |
//! | [`Compiler::quantified`] | `gorilla-co/odata-query` `odata_query/django/django_q.py::visit_CollectionLambda` — `Any` is the subquery, `All` is the subquery over the NEGATED predicate with the whole thing negated (its own comment: "If ALL items in the collection must match, we invert the condition and use NOT EXISTS()"). The `EXISTS`/`NOT EXISTS` pair is spelled here as §5.1's fixed `IN (subquery)` / `NOT IN (subquery)`, which is the same predicate over an owner column that is never NULL. |
//! | [`Compiler::relation_subquery`]'s `invert` flag | `prisma-engines` `query-compiler/query-builders/sql-query-builder/src/filter/visitor.rs` — its `reverse: bool` field and `invert_reverse` helper, a negation carried INTO the nested visit rather than wrapped around its result. |
//! | [`Compiler::exists_subquery`] | `hasura/ndc-postgres` `filtering.rs::translate_exists_in_collection` — one `SELECT <owner> FROM <relation> WHERE <join condition> AND <predicate>` per quantifier, built in the relation's own row scope. |
//!
//! **The three rules a plausible implementation gets wrong** (SPEC §5.1, §5.2,
//! §5.7), each of which has a guard test at the bottom of this file:
//!
//! 1. **One `IN (subquery)` per relational quantifier, never decomposed.**
//!    [`Compiler::quantified`] is the only producer of a relation predicate, and
//!    it always emits ONE subquery carrying the whole conjunction. Decomposing
//!    `props any(key='status' ∧ value='open')` and `props any(key='priority' ∧
//!    value='done')` into four independent probes is what makes a block match
//!    because *some* row satisfies each conjunct separately.
//! 2. **Nothing is NULL.** Every expression this module produces is two-valued.
//!    Subqueries over a nullable owner column (today only
//!    `blocks.parent_block_id`) carry `IS NOT NULL`, and every comparison on a
//!    nullable column is spelled `(<col> IS NOT NULL AND <cmp>)` — SQL's
//!    three-valued logic does not agree with §3.4's classical `not`, and the
//!    difference only shows up on sparse real data. §5.2 writes that guard as
//!    `COALESCE(<cmp>, 0)`; the two are the same two-valued function, and the
//!    `IS NOT NULL AND` spelling is the only one of them that is SARGABLE —
//!    measured, `COALESCE(bp.scheduled_day >= ?, 0)` turns
//!    `block_planning_scheduled_day_idx` from a range SEARCH into a covering
//!    SCAN, which is §5.7's own failure. Rules 2 and 7 both hold in this
//!    spelling and cannot both hold in the other.
//! 3. **No table scan where a positive index exists.**
//!    [`positively_bounded`] implements §5.7's table, which is **exhaustive**: a
//!    leaf/operator pair absent from it is unbounded.
//!
//! **`walk == SQL` is the contract (I-19, I-12).** Every comparison below is
//! written against the walk's own code in [`crate::query::eval`], and the
//! normalization applied to a literal is the SAME function the projection
//! producer applied to the column (`refs::page_key` for `pages.name_key` and
//! `tags.tag_key`, `refs::normalize` for `block_path_refs.normalized_name`,
//! `doc::property_key_norm` for `normalized_name`, `atom::atom_key` for
//! `atom_key`, `search_query::canonical_fold` for `query_visible_folded`) —
//! never a second normalizer that agrees by inspection.
//!
//! **`content match` (§5.10).** The compiler consumes the SAME parsed
//! [`search_query::Matcher`] the walk consumes — [`crate::query::eval::CompiledLeaves`],
//! keyed by [`Filter::match_sources`] — and never re-parses the payload
//! (I-12, D-14). Each retained OR arm becomes an `AND` of `instr` predicates on
//! `blocks.query_visible_folded`, and, when the FTS index is READY, gains a
//! trigram CANDIDATE BOUND that may only ever OVER-approximate: the exact
//! `instr` predicates stay as the final conditions on every path, so a bound
//! that admitted too many rows costs time and a bound that excluded one would
//! be a correctness bug. `foobar` is therefore found by `foo`, by `oob` AND by
//! `oo` — the last one through no bound at all, because word-token FTS cannot
//! answer it.
//!
//! **This compiler declines nothing.** It is total by TYPE — [`lower_query`]
//! returns a [`SqlQuery`] and there is no "unsupported" answer to return — which
//! is the enforceable form of §5.9's rule that a ready projection answers every
//! shape the IR can express. The two families that used to decline are lowered
//! here:
//!
//! * **A valid regex** — `content regexp <pattern>` and the whole-query
//!   `/pattern/` form of `content match`. §4.3.2's fixed SQL predicate is
//!   `tine_query_regex(<id>, <exact visible text>)`, a scalar function
//!   `tine-storage` registers on the read-only connection over a
//!   caller-owned table of compiled regexes ([`QueryRegexProgram`]). The IDs are
//!   BOUND VALUES and the regexes are cheap clones of the SAME
//!   [`CompiledLeaves`] values the walk consumes — never a second compile, a
//!   second grammar or an interpolated pattern (I-12, D-14, I-22). An INVALID
//!   pattern still needs no engine: it is a retained leaf that matches false
//!   (§4.3.2), so it lowers to the constant `0`.
//! * **A `refs` leaf nested inside a `children` predicate.** The walk evaluates
//!   it under the ANCHOR's ancestor multiset, carried through every `children`
//!   quantifier unchanged — see [`Compiler::refs`] for the three stored facts
//!   that reconstruct exactly that set.

use std::collections::HashMap;
use std::sync::Arc;

use tine_storage::sqlite::{MaterializationError, PhysicalQueryValue};

// The acceptance gates. `#[path]` keeps the file beside this one so the shared
// production-source scanner sees a `*_tests.rs` sibling include and blanks it
// from every census (print sites, termination sites, the tine-storage surface).
#[cfg(test)]
#[path = "sql_gates_tests.rs"]
mod sql_gates_tests;

use crate::date::JournalDate;
use crate::doc::property_key_norm;
use crate::query::atom::atom_key;
use crate::query::eval::{format_number, CompiledLeaves};
use crate::query::ir::{Anchor, Attr, CmpOp, Filter, Leaf, ObservedType, Quant, Query, Rel, Value};
use crate::query::registry::Registry;
use crate::refs;
use crate::search_query::{canonical_fold, AndGroup, Matcher, Term};

/// `owner_type` as the projection spells it (`PhysicalEntityId::sql_parts`).
const OWNER_PAGE: i64 = 0;
const OWNER_BLOCK: i64 = 1;

/// `pages.text_kind` for a journal page (`page_kind_to_sql`).
const TEXT_KIND_JOURNAL: i64 = 1;

/// One lowered statement and the values it binds.
///
/// The parameters are a positional list because the seam's signature takes one
/// (`run_projection_query(sql, &[PhysicalQueryValue])`); an interpolated
/// statement is not expressible through this type, which is how I-22 holds
/// structurally rather than by the caller's discipline.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SqlQuery {
    pub(crate) sql: String,
    pub(crate) params: Vec<PhysicalQueryValue>,
    /// §5.7: whether the root conjunction carries at least one positive leaf
    /// whose anchor bound is `yes`. The plan gate asserts `SEARCH` on the anchor
    /// alias exactly for these.
    pub(crate) positively_bounded: bool,
    /// The filter folded to the constant false, so the statement provably reads
    /// no row. It is still a statement — the caller has one path, not two — but
    /// there is no index for SQLite to choose and none to demand of it.
    pub(crate) matches_nothing: bool,
    /// §5.10's plan classes, one entry per `content match` / `content regexp`
    /// leaf that reached the statement, in depth-first order. They are recorded
    /// SEPARATELY from the indexed case rather than as failures or as a blanket
    /// content exemption: three of the four are content paths §5.10 says are
    /// explicitly not index-bounded, and a plan gate that could not name them
    /// would have to choose between failing them and exempting all content.
    pub(crate) content_plans: Vec<ContentPlan>,
    /// §4.3.2's compiled-regex table for THIS statement — the IDs its
    /// `tine_query_regex` calls bind, and the regex each one names. Empty for
    /// the overwhelming majority of statements; the executor installs it on the
    /// connection before the statement runs (see [`QueryRegexProgram`]).
    pub(crate) regexes: QueryRegexProgram,
}

/// §4.3.2's compiled-regex table, owned by ONE lowered statement.
///
/// **The seam, once, for every backend.** `tine-storage` exposes the SAME fixed
/// `set_query_regex_predicate` on the read-only reader Direct Files pools
/// ([`crate::direct_projection::DirectProjection::run_statement`]) and on the
/// owned read snapshot R3/R4 will hold, so [`QueryRegexProgram::predicate`] is
/// the only thing either of them installs — there is no second matcher in a
/// backend to disagree with this one (D-14, I-12).
///
/// **Why an ID table and not the pattern.** The statement binds `?n` = an
/// integer ID; the pattern text never enters the SQL, is never interpolated and
/// is never logged. The regex behind an ID is a CLONE of the value
/// [`CompiledLeaves`] already compiled for this execution — `regex::Regex` is
/// internally reference-counted, so the clone is cheap and, more importantly,
/// it is the SAME program the walk runs (I-12).
///
/// **Scope.** IDs are meaningful only for the statement that assigned them, so
/// an executor REPLACES the whole table before each dispatched statement rather
/// than adding to it, and an ID the table does not name fails the read instead
/// of matching anything.
#[derive(Clone, Default)]
pub(crate) struct QueryRegexProgram {
    /// Position `i` carries ID `i + 1`. The ID is positional rather than stored
    /// so the table and the statement cannot drift apart.
    bindings: Vec<QueryRegexBinding>,
}

/// One `(id, pattern, compiled)` row of a [`QueryRegexProgram`].
#[derive(Clone)]
struct QueryRegexBinding {
    /// The user's pattern source — the key this compiler de-duplicates on, and
    /// the ONLY thing that makes two programs comparable. It is never emitted
    /// into SQL, printed by [`std::fmt::Debug`] or logged.
    pattern: String,
    compiled: regex::Regex,
}

impl QueryRegexProgram {
    pub(crate) fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }

    /// The predicate `tine_query_regex(<id>, <exact visible text>)` calls.
    ///
    /// An ID this program does not name is an ERROR and not `false`: it means
    /// the statement and the installed table disagree, which is a failed read
    /// (§5.9's recovery), never a silently smaller result set.
    pub(crate) fn predicate(
        &self,
    ) -> impl Fn(u64, &str) -> Result<bool, MaterializationError> + Send + 'static {
        let table: Arc<HashMap<u64, regex::Regex>> = Arc::new(
            self.bindings
                .iter()
                .enumerate()
                .map(|(at, binding)| (at as u64 + 1, binding.compiled.clone()))
                .collect(),
        );
        move |id, text| match table.get(&id) {
            Some(regex) => Ok(regex.is_match(text)),
            // The message names the ID and never the pattern or the row's text.
            None => Err(MaterializationError::InvalidQuery(format!(
                "query regex id {id} is not bound by this statement"
            ))),
        }
    }
}

/// Two programs are equal when they bind the same patterns to the same IDs.
///
/// `regex::Regex` has no `PartialEq` — and an equality that compared compiled
/// programs by pointer would make [`SqlQuery`]'s derived `PartialEq` quietly
/// false for two identical lowerings. The pattern source IS the identity of a
/// compiled regex here, because [`CompiledLeaves`] keys its cache by exactly
/// that string.
impl PartialEq for QueryRegexProgram {
    fn eq(&self, other: &Self) -> bool {
        self.bindings.len() == other.bindings.len()
            && self
                .bindings
                .iter()
                .zip(&other.bindings)
                .all(|(ours, theirs)| ours.pattern == theirs.pattern)
    }
}

/// Deliberately COUNTS the bindings instead of printing them: a `SqlQuery` is
/// `Debug`-printed by failing assertions and by panics, and §4.3.2's pattern
/// text is user content that has no business in a log line.
impl std::fmt::Debug for QueryRegexProgram {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("QueryRegexProgram")
            .field("bindings", &self.bindings.len())
            .finish()
    }
}

/// How ONE content leaf reaches its rows (SPEC §5.10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContentPlan {
    /// Every retained OR arm supplied a trigram candidate needle and the FTS
    /// index is ready: the leaf bounds the anchor.
    Fts,
    /// At least one retained OR arm has no positive term yielding a
    /// three-scalar whitespace-free run (or its only candidates bear a NUL), so
    /// that arm is an explicitly unbounded SQL content predicate. `foobar`
    /// queried by `oo` lands here, correctly, and is still found.
    ShortUnindexable,
    /// The transient state: the FTS index is still building on this
    /// materialized read, so the SAME exact predicates are evaluated on the
    /// ready block columns with no candidate bounds. Not empty results, not an
    /// error, not a new walk route, and never a rebuild request or a wait
    /// inside a query (I-13) — the existing index owner finishes the build.
    FtsBuilding,
    /// A regex leaf. §4.3.2 makes it an explicitly unindexed content predicate;
    /// regex can never claim an FTS bound. Only the INVALID-pattern form
    /// reaches a statement in this wave (it is a constant-false leaf); a valid
    /// pattern is declined — see the module header.
    Regex,
}

/// How §5.3's result-set rule — OG's `tree/filter-top-level-blocks`, "drop a
/// matched block whose IMMEDIATE parent also matched" — is spelled in SQL.
///
/// The two spellings are the SAME predicate, because `filter(row)` is a pure
/// function of the row: "the parent matches" and "the parent is in the match
/// set" cannot differ. They differ only in how many times SQLite evaluates the
/// filter, which is why the choice between them is settled by MEASUREMENT
/// (§5.9's packet) and pinned by an identity gate that compares the two
/// spellings against each other and against the walk — including the transitive
/// case, a block whose GRANDPARENT matches but whose parent does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResultSetRule {
    /// A correlated probe on the primary key: `parent_block_id NOT IN (SELECT …
    /// WHERE block_id = <anchor>.parent_block_id AND <filter>)`. One seek per
    /// candidate row, but the whole filter is re-evaluated per row — for a
    /// `children` predicate that nests a subquery per row.
    CorrelatedProbe,
    /// SPEC §3.5's own spelling: name the match set as a CTE and anti-join it
    /// against its own `parent_block_id`. The anti-join subquery is
    /// UNCORRELATED, so SQLite evaluates it once for the whole statement
    /// instead of once per candidate row — but SQLite also INLINES an ordinary
    /// CTE, so the filter itself is still evaluated twice (once as the row
    /// source, once to build the anti-join list).
    MatchSetCte,
    /// The same anti-join with SQLite's `MATERIALIZED` hint, which is the only
    /// spelling that actually evaluates the filter ONCE: the match set is
    /// computed into a transient table and both references read it. This is the
    /// "single-evaluation spelling" §5.9's policy question is really about, and
    /// the reason the plain CTE is not it.
    MatchSetCteMaterialized,
}

/// The production spelling, chosen by the measurement recorded in P1-d's receipt
/// and reproducible through
/// `the_two_result_set_spellings_are_timed_against_each_other_on_a_real_corpus`.
///
/// **Measured, not argued** (anonymized graph, release, eight independent
/// sessions). On the decisive shape `any(children, task = 'DONE')`, 263 rows:
/// the correlated probe costs ~5.1 ms, the plain CTE ~5.3 ms (a regression) and
/// the materialized CTE ~2.8 ms — **0.54-0.56×, in every one of the eight
/// runs**. The plain CTE loses because SQLite INLINES it and the filter still
/// runs twice, so it is not the single-evaluation spelling the question was
/// about; only the `MATERIALIZED` hint forces one evaluation.
///
/// **The honest remainder, in three parts.**
/// 1. The decision rule's second half — "no other `PLAN_SHAPES` entry regresses
///    by more than 10%" — does NOT hold cleanly. `deadline is not null`
///    (~100-125 µs, 86 rows) straddles the threshold run to run, and the gate
///    printed ADOPT in three of eight sessions and KEEP in five. Every flagged
///    regression is 1-13 µs on a sub-millisecond shape; the win it is weighed
///    against is 2.3 ms. Adopting is therefore a recorded LANE DECISION, and
///    `RESULT_SET_RULE` is the single line that reverts it.
/// 2. Even with the win, that shape measures walk 435 µs vs SQL 2935 µs. It is
///    recorded, not routed around (§5.9 has no fourth route), and no remedy is
///    named here that has not been measured.
/// 3. The plain `MatchSetCte` variant is kept ALIVE rather than deleted,
///    because it is what makes claim (1) checkable: it is the spelling that
///    shows the inlining, and a gate that could only compare two options could
///    not have found that the third was the real one.
pub(crate) const RESULT_SET_RULE: ResultSetRule = ResultSetRule::MatchSetCteMaterialized;

/// Everything an execution binds that is not in the IR.
pub(crate) struct LoweringInputs<'a> {
    /// The ONE execution-day snapshot `resolve_for_execution` took.
    pub(crate) today: JournalDate,
    /// The registry snapshot that decides each property key's effective type
    /// (§6.3). The walk reads the same snapshot for the same execution.
    pub(crate) registry: &'a Registry,
    /// Page ids an unaccepted local overlay covers (§5.9, Managed Storage).
    /// Empty on Direct Files.
    pub(crate) masked_pages: &'a [[u8; 16]],
    /// `LIMIT cutoff + 1` when the caller supplies a cutoff (§5.6).
    pub(crate) cutoff: Option<usize>,
    /// The ONE parse of every `content match` payload for this execution
    /// (§5.10) — the same value the walk reads through
    /// `CompiledLeaves::match_program`, over the same `Filter::match_sources`
    /// keys. A second `Matcher::parse` here is the fork this campaign exists to
    /// prevent: `content match` and legacy `(search …)` would stop meaning the
    /// same thing the moment the two parses disagreed (I-12, D-14).
    pub(crate) compiled: &'a CompiledLeaves,
    /// The EXISTING FTS-building signal, read on the SAME materialized
    /// read/generation as the query and separately from projection readiness
    /// (§5.10). `false` is the transient `fts-building` class: the same exact
    /// predicates, evaluated on the ready block columns, with no candidate
    /// bounds anywhere in the statement.
    pub(crate) fts_ready: bool,
    /// Which spelling of §5.3's result-set rule to emit. Production passes
    /// [`RESULT_SET_RULE`]; the measurement gate passes both so the choice
    /// stays reproducible rather than remembered.
    pub(crate) result_set_rule: ResultSetRule,
}

/// Lower one resolved query (SPEC §5.1–§5.7).
///
/// The filter is the EVALUABLE one: `Off` subtrees are removed bottom-up first,
/// exactly as the walk does (§3.5), so the two engines never see different
/// trees.
pub(crate) fn lower_query(query: &Query, inputs: &LoweringInputs<'_>) -> SqlQuery {
    let mut compiler = Compiler {
        inputs,
        params: Vec::new(),
        next_alias: 0,
        regexes: Vec::new(),
    };
    // An invalid query returns zero results plus its diagnostics (§3.5); the
    // caller never reaches the statement, but a `0` predicate keeps this
    // function total.
    let filter = if query.is_invalid() {
        Filter::False
    } else {
        query.evaluable_filter()
    };
    let (row, select, from) = match query.anchor {
        // §5.3's block row is `(block_id, page_id, path)` and nothing else.
        // `block_id` is the answer, `page_id` is the routing identity Managed
        // Storage's overlay route will address a page by, and `path` is the key
        // Direct hydration loads a `Document` under TODAY
        // (`direct_projection_pages_for_paths_ordered`). `pages.name` and
        // `pages.text_kind` were decoration: no consumer of these rows ever
        // decoded either, and the hydration reads both from the page entry it
        // loads anyway.
        Anchor::Block => (
            Row::Block(BlockScope::anchored("b")),
            "SELECT b.block_id, b.page_id, p.path",
            "FROM blocks b JOIN pages p ON p.page_id = b.page_id",
        ),
        Anchor::Page => (
            Row::Page("p"),
            "SELECT p.page_id, p.name, p.text_kind, p.journal_day",
            "FROM pages p",
        ),
    };
    let mut where_ = compiler.filter(&filter, row);
    // §5.3: the result-set rule is applied in the SAME statement. OG's
    // `tree/filter-top-level-blocks` drops a matched block whose IMMEDIATE
    // parent also matched, and the walk implements it in
    // `collect_og_query_roots`'s `matched` stack — a block is emitted iff it
    // matches and its parent does not. Two spellings say that (see
    // [`ResultSetRule`]); which one is emitted is settled by measurement, not by
    // argument, and both are pinned identical by a gate.
    let matches_nothing = where_ == "0";
    let mut cte: Option<String> = None;
    // A filter that folded to false reads nothing under either spelling, and
    // neither the probe nor the CTE can add a row to the empty set. Leaving the
    // statement as the bare `WHERE 0` keeps that case byte-identical across the
    // two spellings, so the identity gate below compares real statements.
    if let (Row::Block(scope), false) = (row, matches_nothing) {
        let alias = scope.alias;
        match inputs.result_set_rule {
            ResultSetRule::CorrelatedProbe => {
                let parent = compiler.alias("root");
                // The probe asks the same question of the PARENT row, so the
                // parent is its own anchor: a `refs` leaf nested under it reads
                // the PARENT's ancestor context, exactly as the walk does when
                // `collect_og_query_roots` evaluates the filter there.
                let parent_matches =
                    compiler.filter(&filter, Row::Block(BlockScope::anchored(&parent)));
                // A parent that can never match cannot shadow anything, so the
                // whole probe folds away rather than becoming a correlated
                // subquery over `0`.
                let unshadowed = if parent_matches == "0" {
                    "1".to_string()
                } else {
                    format!(
                        "({alias}.parent_block_id IS NULL OR {alias}.parent_block_id NOT IN \
                         (SELECT {parent}.block_id FROM blocks {parent} \
                         WHERE {parent}.block_id = {alias}.parent_block_id AND {parent_matches}))"
                    )
                };
                where_ = fold_and(vec![where_, unshadowed]);
            }
            ResultSetRule::MatchSetCte | ResultSetRule::MatchSetCteMaterialized => {
                // SPEC §3.5's own spelling. The match set is named once and
                // anti-joined against its own `parent_block_id`; the anti-join
                // subquery is uncorrelated, so the filter is never re-evaluated
                // per candidate row.
                let hint = if inputs.result_set_rule == ResultSetRule::MatchSetCteMaterialized {
                    " MATERIALIZED"
                } else {
                    ""
                };
                //
                // The match set is a BLOCKS question: `FROM blocks b`, with no
                // join to `pages`. Every page predicate carries its own
                // `pages` subquery keyed by `page_id`
                // ([`Compiler::page_relation`] and the nested page relations it
                // compiles), so no fragment of the filter reads an outer `p`,
                // and `blocks.page_id` is a NOT NULL foreign key into
                // `pages(page_id)` — the join could neither add nor drop a
                // candidate. What it did do was probe the pages primary-key
                // index once per candidate row to carry columns the match does
                // not use. Routing to `pages.path` happens ONCE, on the answer,
                // in the outer select below.
                cte = Some(format!(
                    "WITH m(block_id, page_id, parent_block_id) AS{hint} \
                     (SELECT {alias}.block_id, {alias}.page_id, {alias}.parent_block_id \
                     FROM blocks {alias} WHERE {where_})"
                ));
                where_ = "(m.parent_block_id IS NULL OR m.parent_block_id NOT IN \
                     (SELECT block_id FROM m))"
                    .to_string();
            }
        }
    }
    // The anchor of the statement, once the result-set rule has chosen its
    // shape: `blocks b` for the correlated probe, the materialized match set for
    // the CTE. `@page` has no suppression rule and keeps `pages p`.
    let (select, from, mask_column) = match (row, &cte) {
        (Row::Block(scope), None) => (select, from, format!("{}.page_id", scope.alias)),
        // The mask stays on the ANCHOR, outside `m`, exactly as it is outside
        // the correlated probe today: a block and its parent are always on the
        // same page, so masking inside `m` would be unobservable either way, and
        // staying outside keeps the masked statement the same predicate.
        (Row::Block(_), Some(_)) => (
            "SELECT m.block_id, m.page_id, p.path",
            "FROM m JOIN pages p ON p.page_id = m.page_id",
            "m.page_id".to_string(),
        ),
        (Row::Page(alias), _) => (select, from, format!("{alias}.page_id")),
    };
    // §5.9: the overlay-masked page ids are removed inside the statement, so the
    // masked read and the overlay walk cannot both answer for one page.
    if !inputs.masked_pages.is_empty() {
        let list = inputs
            .masked_pages
            .iter()
            .map(|page| compiler.bind(PhysicalQueryValue::Blob(page.to_vec())))
            .collect::<Vec<_>>()
            .join(", ");
        where_ = fold_and(vec![where_, format!("{mask_column} NOT IN ({list})")]);
    }
    // **No `ORDER BY` (§5.3, measured).** The walk's base order is its page
    // SOURCE's enumeration order, which no column of the projection reproduces —
    // ordering by `pages.path` would be a guess, and it is a costly one: with
    // `ORDER BY p.path` in the statement SQLite prefers `SCAN p USING INDEX
    // pages_path_idx` over the `pages_journal_day_idx` range the filter asks
    // for, because scanning the path index avoids the sort. That is exactly
    // §5.7's failure mode, bought for an ordering the caller has to impose
    // anyway when it groups the rows by page. Base order and grouping therefore
    // belong to the result construction, and this statement answers only which
    // rows match.
    let mut sql = match &cte {
        Some(cte) => format!("{cte} {select} {from} WHERE {where_}"),
        None => format!("{select} {from} WHERE {where_}"),
    };
    if let Some(cutoff) = inputs.cutoff {
        let limit = compiler.bind(PhysicalQueryValue::Integer(
            i64::try_from(cutoff.saturating_add(1)).unwrap_or(i64::MAX),
        ));
        sql.push_str(&format!(" LIMIT {limit}"));
    }
    // Folding discards fragments that already bound values; positional
    // parameters have to be renumbered around the holes.
    let (sql, params) = compact_parameters(&sql, &compiler.params);
    SqlQuery {
        sql,
        params,
        // §5.7 asks for an index only where there are rows to find. A filter
        // that folded to false reads nothing, so there is no bound to demand.
        positively_bounded: !matches_nothing && positively_bounded(&filter, query.anchor, inputs),
        matches_nothing,
        // Classified from the FILTER, not accumulated while compiling: §5.3's
        // parent probe compiles the same tree a second time, and a class
        // counted once per compilation pass would double every entry.
        content_plans: content_plans(&filter, inputs),
        regexes: QueryRegexProgram {
            bindings: compiler.regexes,
        },
    }
}

/// Which row a filter is being compiled against, and under which alias.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Row<'a> {
    Block(BlockScope<'a>),
    Page(&'a str),
}

/// A block row scope: the alias the filter's own columns read, and the alias of
/// the row whose ANCESTOR-reference context a `refs` leaf inside it sees.
///
/// The two differ exactly inside a `children` predicate, and they keep
/// differing at every further level of nesting, because `eval_block_leaf`'s
/// `Rel::Children` arm passes `ancestor_refs` DOWN UNCHANGED: a grandchild is
/// evaluated under the same multiset the anchor was, not under its own parent's
/// (see [`Compiler::refs`]). Carrying the anchor explicitly is what makes that
/// rule a property of the compiler rather than of the order its recursion
/// happens to visit rows in.
#[derive(Clone, Copy, PartialEq, Eq)]
struct BlockScope<'a> {
    alias: &'a str,
    anchor: &'a str,
}

impl<'a> BlockScope<'a> {
    /// A row that establishes its own ancestor context: the statement's anchor,
    /// and §5.3's parent probe.
    fn anchored(alias: &'a str) -> BlockScope<'a> {
        BlockScope {
            alias,
            anchor: alias,
        }
    }

    /// A row reached THROUGH a relation from this one, keeping this scope's
    /// ancestor context.
    fn nested<'b>(self, alias: &'b str) -> BlockScope<'b>
    where
        'a: 'b,
    {
        BlockScope {
            alias,
            anchor: self.anchor,
        }
    }

    /// Whether this row is the one that established the ancestor context.
    /// Aliases are unique within a statement, so the comparison is exact.
    fn is_anchor(self) -> bool {
        self.alias == self.anchor
    }
}

struct Compiler<'a> {
    inputs: &'a LoweringInputs<'a>,
    params: Vec<PhysicalQueryValue>,
    next_alias: usize,
    /// §4.3.2's compiled-regex table, in ID order, de-duplicated by pattern
    /// source so §5.3's second compilation pass reuses the FIRST pass's IDs
    /// rather than growing a parallel table.
    regexes: Vec<QueryRegexBinding>,
}

impl Compiler<'_> {
    /// Bind one value and return its positional placeholder (§5.5).
    fn bind(&mut self, value: PhysicalQueryValue) -> String {
        self.params.push(value);
        format!("?{}", self.params.len())
    }

    /// Bind ONE compiled regex and return the placeholder holding its ID.
    ///
    /// The regex is a clone of the shared [`CompiledLeaves`] value, keyed by the
    /// same pattern source that map is keyed by, so the same pattern written
    /// twice in one query — or compiled twice by §5.3's parent probe — is one
    /// table row and one ID.
    fn bind_regex(&mut self, pattern: &str, compiled: &regex::Regex) -> String {
        let at = match self
            .regexes
            .iter()
            .position(|binding| binding.pattern == pattern)
        {
            Some(at) => at,
            None => {
                self.regexes.push(QueryRegexBinding {
                    pattern: pattern.to_owned(),
                    compiled: compiled.clone(),
                });
                self.regexes.len() - 1
            }
        };
        self.bind(PhysicalQueryValue::Integer(at as i64 + 1))
    }

    /// A fresh alias for a relation subquery, so nesting cannot shadow.
    fn alias(&mut self, stem: &str) -> String {
        self.next_alias += 1;
        format!("{stem}{}", self.next_alias)
    }

    // -----------------------------------------------------------------------
    // The boolean skeleton
    // -----------------------------------------------------------------------

    /// Transcribes `hasura/ndc-postgres`
    /// `filtering.rs::translate_expression_with_joins`: `And`/`Or` fold their
    /// already-translated operands, `Not` wraps one. Every operand is
    /// two-valued, so `NOT` is classical (§3.4) rather than SQL's three-valued
    /// `NOT NULL = NULL`.
    fn filter(&mut self, filter: &Filter, row: Row<'_>) -> String {
        match filter {
            // `normalized()` turns an originally-empty group into the constant
            // it means (§3.5); this arm keeps the compiler total for a tree that
            // never went through it.
            Filter::And { items } if items.is_empty() => "1".to_string(),
            Filter::Or { items } if items.is_empty() => "0".to_string(),
            Filter::And { items } => {
                fold_and(items.iter().map(|it| self.filter(it, row)).collect())
            }
            Filter::Or { items } => fold_or(items.iter().map(|it| self.filter(it, row)).collect()),
            Filter::Not { inner } => fold_not(self.filter(inner, row)),
            Filter::True => "1".to_string(),
            // A `Raw` span is never satisfiable, and `not(<raw>)` must not
            // invent matches — the walk's rule, verbatim.
            Filter::False | Filter::Raw { .. } => "0".to_string(),
            Filter::Off { .. } => {
                debug_assert!(false, "Off must be removed before lowering (§3.5)");
                "1".to_string()
            }
            Filter::Leaf { leaf } => match row {
                Row::Block(scope) => self.leaf_block(leaf, scope),
                Row::Page(alias) => self.leaf_page(leaf, alias),
            },
        }
    }

    // -----------------------------------------------------------------------
    // §5.1 — one `IN (subquery)` per relational quantifier
    // -----------------------------------------------------------------------

    /// Transcribes `odata-query`'s `visit_CollectionLambda`: `Any` is the
    /// subquery, `All` is the subquery over the INVERTED predicate, negated.
    /// `None` is `Any` negated, which is the same rewrite in the other
    /// direction.
    ///
    /// `owner` is the outer column the subquery's selected column is compared
    /// against, and it is never NULL on either side (J1) — which is what makes
    /// `NOT IN` mean what it reads as.
    fn quantified(
        &mut self,
        owner: &str,
        quant: Quant,
        mut subquery: impl FnMut(&mut Self, bool) -> Option<String>,
    ) -> String {
        match quant {
            // No row can satisfy the predicate, so no owner is `IN` it.
            Quant::Any => match subquery(self, false) {
                Some(sub) => format!("{owner} IN ({sub})"),
                None => "0".to_string(),
            },
            // `NOT IN` an empty set is true for every owner — and for `Every`
            // the empty set is the set of VIOLATORS, so every owner passes.
            Quant::None => match subquery(self, false) {
                Some(sub) => format!("{owner} NOT IN ({sub})"),
                None => "1".to_string(),
            },
            Quant::Every => match subquery(self, true) {
                Some(sub) => format!("{owner} NOT IN ({sub})"),
                None => "1".to_string(),
            },
        }
    }

    /// Transcribes `filtering.rs::translate_exists_in_collection`: one
    /// `SELECT <owner> FROM <relation> WHERE <guard> AND <predicate>` in the
    /// relation's own row scope. The `invert` flag is `prisma-engines`'
    /// `reverse` — a negation carried INTO the nested predicate.
    ///
    /// `None` is "this subquery selects no row" — the predicate folded to false,
    /// so there is nothing for SQLite to look for. Returning it rather than
    /// emitting `WHERE … AND 0` is what keeps [`Compiler::quantified`] able to
    /// fold the quantifier, and it is what keeps §5.7 honest: a provably empty
    /// subquery still gets planned, and SQLite plans it as a covering SCAN.
    fn exists_subquery(
        &mut self,
        select: &str,
        from: &str,
        guards: &[String],
        predicate: String,
        invert: bool,
    ) -> Option<String> {
        let predicate = if invert {
            fold_not(predicate)
        } else {
            predicate
        };
        let mut clauses: Vec<String> = guards.to_vec();
        clauses.push(predicate);
        let where_ = fold_and(clauses);
        if where_ == "0" {
            return None;
        }
        Some(format!("SELECT {select} FROM {from} WHERE {where_}"))
    }

    /// One relation subquery, in the relation element's own row scope.
    fn relation_subquery(
        &mut self,
        select: &str,
        from: &str,
        guards: &[String],
        pred: &Filter,
        row: Row<'_>,
        invert: bool,
    ) -> Option<String> {
        let predicate = self.filter(pred, row);
        self.exists_subquery(select, from, guards, predicate, invert)
    }

    // -----------------------------------------------------------------------
    // Block-row leaves
    // -----------------------------------------------------------------------

    fn leaf_block(&mut self, leaf: &Leaf, scope: BlockScope<'_>) -> String {
        let b = scope.alias;
        match leaf {
            Leaf::Attr { attr, op, value } => match attr {
                Attr::Content => self.content(*op, value, b),
                Attr::Task => self.task(*op, value, b),
                Attr::Priority => self.priority(*op, value, b),
                Attr::Scheduled => self.planning(*op, value, b, "scheduled"),
                Attr::Deadline => self.planning(*op, value, b, "deadline"),
                // Page attributes only appear under a `page` relation and the
                // property-element attributes only under `props`; the walk
                // answers false for anything else (`eval_block_leaf`).
                _ => "0".to_string(),
            },
            Leaf::Rel { rel, quant, pred } => match rel {
                Rel::Refs => self.refs(*quant, pred, scope),
                Rel::Tags => self.tags(*quant, pred, b, OWNER_BLOCK),
                Rel::Props => self.props(*quant, pred, b, "block_id", OWNER_BLOCK),
                Rel::Children => self.children(*quant, pred, scope),
                Rel::Page => self.page_relation(*quant, pred, b),
                // `blocks` is a page-row relation; the block-anchored walk
                // answers false for it (`eval_block_leaf`'s `Rel::Blocks` arm),
                // and so does this.
                Rel::Blocks => "0".to_string(),
            },
        }
    }

    /// `content` predicates read `blocks.query_visible_folded` — the EXACT
    /// visible text folded once at write time (§5.8), never the
    /// whitespace-collapsed `block_text.searchable_text` payload. The walk compares
    /// `BlockProjection::visible_lower`, which is the same fold of the same
    /// text.
    fn content(&mut self, op: CmpOp, value: &Value, b: &str) -> String {
        let Value::Text { text } = value else {
            return "0".to_string();
        };
        let column = format!("{b}.query_visible_folded");
        match op {
            CmpOp::Like => {
                let pattern = self.bind(PhysicalQueryValue::Text(canonical_fold(text)));
                format!("{column} LIKE {pattern} ESCAPE '\\'")
            }
            CmpOp::StartsWith => {
                let pattern = self.bind(PhysicalQueryValue::Text(format!(
                    "{}%",
                    like_escape(&canonical_fold(text))
                )));
                format!("{column} LIKE {pattern} ESCAPE '\\'")
            }
            CmpOp::Eq => {
                let literal = self.bind(PhysicalQueryValue::Text(canonical_fold(text)));
                format!("{column} = {literal}")
            }
            CmpOp::NotEq => {
                let literal = self.bind(PhysicalQueryValue::Text(canonical_fold(text)));
                format!("{column} <> {literal}")
            }
            CmpOp::Match => self.content_match(text, b),
            CmpOp::Regex => self.content_regex(text, b),
            _ => "0".to_string(),
        }
    }

    // -----------------------------------------------------------------------
    // §5.10 — `content match`
    // -----------------------------------------------------------------------

    /// One `content match <text>` leaf, from the SAME parsed
    /// [`Matcher`] the walk consumes for this execution.
    ///
    /// The walk's arm is `compiled.match_program(text).is_some_and(|m|
    /// m.matches(visible_lower, visible))`, so a payload that was never
    /// collected is FALSE there and is the constant `0` here — the two engines
    /// agree without the compiler having to know why the payload is missing.
    fn content_match(&mut self, source: &str, b: &str) -> String {
        match match_program(self.inputs.compiled, source) {
            // `Matcher::matches` answers false for `Empty` and `InvalidRegex`
            // (an exclusion-only query, a blank one, a pattern that did not
            // compile). §5.10: that is a FALSE LEAF, not an enabled whole-query
            // diagnostic — so `not (content match '-foo')` is classically true
            // on both engines (§3.4), and the matcher's own error message may
            // still be displayed without changing this truth rule.
            MatchProgram::AlwaysFalse | MatchProgram::Regex { compiled: None } => "0".to_string(),
            MatchProgram::Regex {
                compiled: Some(regex),
            } => self.content_regex_predicate(source, &regex, b),
            MatchProgram::Boolean(groups) => {
                let arms = groups
                    .iter()
                    .map(|group| self.match_group(group, b))
                    .collect();
                fold_or(arms)
            }
        }
    }

    /// One retained OR arm: the exact `instr` conjunction, plus — only when the
    /// FTS index is ready and the arm offers a needle — a candidate bound in
    /// front of it.
    ///
    /// **The exact predicates are never replaced by the bound, on any path.**
    /// That is what makes the bound safe to be a superset and fatal to be a
    /// subset, and it is why `search_fts`'s word tokens are not substituted for
    /// substrings (§5.10, CLOSURE §4).
    fn match_group(&mut self, group: &AndGroup, b: &str) -> String {
        let exact = fold_and(
            group
                .iter()
                .map(|term| self.match_term(term, b))
                .collect::<Vec<_>>(),
        );
        // An arm that provably matches nothing is not worth asking the index
        // for, and `AND 0` inside the bound subquery would be planned as a scan.
        if exact == "0" {
            return exact;
        }
        match self.fts_bound(group, b) {
            Some(bound) => fold_and(vec![bound, exact]),
            None => exact,
        }
    }

    /// One term of an AND group, transcribing `search_query::group_matches`
    /// verbatim: `present = !text.is_empty() && lower.contains(text)`, then
    /// `present != negated`.
    ///
    /// **Emptiness is decided in the parsed [`Term`], never in SQLite.**
    /// `instr(text, '')` is 1 and would make an empty positive term true, and
    /// `length()` stops at the first NUL so it cannot even measure the string —
    /// so the two engines can only agree if the Rust side answers (§5.10).
    fn match_term(&mut self, term: &Term, b: &str) -> String {
        if term.text.is_empty() {
            // `present` is false, so the term is satisfied exactly when it is a
            // negative one. (A group of only negative terms never reaches here:
            // `Matcher::parse` discards it.)
            return if term.negated { "1" } else { "0" }.to_string();
        }
        // The needle is the parser's own canonically folded text and the column
        // is `canonical_fold(visible)` written by both producers — the same
        // fold on both sides, never a second normalizer that agrees by
        // inspection.
        let needle = self.bind(PhysicalQueryValue::Text(term.text.clone()));
        let present = format!("(instr({b}.query_visible_folded, {needle}) > 0)");
        if term.negated {
            fold_not(present)
        } else {
            present
        }
    }

    /// The trigram candidate bound for one OR arm, or `None` when the arm
    /// supplies none — which makes the arm an explicitly unbounded SQL content
    /// predicate rather than a defect to work around (§5.10).
    ///
    /// `search_substring_fts` is a `tokenize = 'trigram'` FTS5 table over
    /// `normalized_searchable_text`, associated to its owner through
    /// `search_fts_owners.rowid`; both producers write that column as
    /// `canonical_fold(searchable_text)`, i.e. the SAME fold as the exact
    /// column over WHITESPACE-COLLAPSED text. That is the whole reason the
    /// needle is a whitespace-free run and not the phrase: a phrase with
    /// leading, repeated or line-breaking whitespace does not survive the
    /// collapse, and a bound that required it to would exclude a true match.
    fn fts_bound(&mut self, group: &AndGroup, b: &str) -> Option<String> {
        if !self.inputs.fts_ready {
            return None;
        }
        let needle = fts_candidate_needle(group)?;
        let literal = self.bind(PhysicalQueryValue::Text(fts_phrase_literal(needle)));
        let fts = self.alias("sf");
        let owners = self.alias("fo");
        Some(format!(
            "{b}.block_id IN (SELECT {owners}.entity_id \
             FROM search_substring_fts {fts} \
             JOIN search_fts_owners {owners} ON {owners}.rowid = {fts}.rowid \
             WHERE {fts}.normalized_text MATCH {literal} \
             AND {owners}.entity_type = {OWNER_BLOCK})"
        ))
    }

    /// One legacy `content regexp <pattern>` leaf (§4.3.2).
    ///
    /// The walk's arm is `compiled.regex(text).is_some_and(|r|
    /// r.is_match(visible))`, and `CompiledLeaves` stores `Regex::new(text).ok()`
    /// — so a pattern that did not compile is a retained leaf matching FALSE,
    /// which needs no regex engine in SQLite and lowers to the constant `0`.
    fn content_regex(&mut self, source: &str, b: &str) -> String {
        let Some(regex) = self.inputs.compiled.regex(source) else {
            return "0".to_string();
        };
        let regex = regex.clone();
        self.content_regex_predicate(source, &regex, b)
    }

    /// §4.3.2's fixed SQL predicate, shared by both regex spellings.
    ///
    /// **The text is `block_text.query_visible`, not `blocks.query_visible_folded`.**
    /// Both walk arms match against `BlockProjection::visible` — the EXACT
    /// visible text — and the folded column is lower-cased and NFC-normalized,
    /// so a case-sensitive or accent-sensitive pattern would answer differently
    /// there. §5.8's producers write `query_visible` as that same exact string.
    ///
    /// The subquery is CORRELATED on `block_text`'s primary key, so the regex
    /// runs once per candidate row that reaches the leaf — the walk's own cost
    /// model — instead of once per block in the graph, which an uncorrelated
    /// `IN (SELECT … WHERE tine_query_regex(…))` would have forced. Regex stays
    /// explicitly UNINDEXED either way (§4.3.2): no candidate bound may claim
    /// it, and the textual position of this conjunct promises nothing about the
    /// order SQLite evaluates the statement in.
    ///
    /// A block with no `block_text` row is simply not in the set, which is
    /// `false` and its classical negation — never a NULL and never a read error.
    fn content_regex_predicate(&mut self, source: &str, regex: &regex::Regex, b: &str) -> String {
        let alias = self.alias("bt");
        let owner = format!("{b}.block_id");
        let id = self.bind_regex(source, regex);
        let predicate = format!("tine_query_regex({id}, {alias}.query_visible)");
        match self.exists_subquery(
            &format!("{alias}.block_id"),
            &format!("block_text {alias}"),
            &[format!("{alias}.block_id = {owner}")],
            predicate,
            false,
        ) {
            Some(sub) => format!("{owner} IN ({sub})"),
            None => "0".to_string(),
        }
    }

    /// `task` reads `tasks.marker`. Both producers write the marker
    /// ASCII-uppercased and lsdoc's `MARKERS` list is uppercase-only, so
    /// binding the uppercased operand is exactly the walk's
    /// `eq_ignore_ascii_case` and still seeks `tasks_marker_idx`.
    fn task(&mut self, op: CmpOp, value: &Value, b: &str) -> String {
        let alias = self.alias("t");
        let from = format!("tasks {alias}");
        let select = format!("{alias}.block_id");
        let owner = format!("{b}.block_id");
        match op {
            CmpOp::IsSet => format!("{owner} IN (SELECT {select} FROM {from})"),
            CmpOp::IsNotSet => format!("{owner} NOT IN (SELECT {select} FROM {from})"),
            CmpOp::Eq | CmpOp::NotEq => {
                let Value::Text { text } = value else {
                    return "0".to_string();
                };
                let literal = self.bind(PhysicalQueryValue::Text(text.to_ascii_uppercase()));
                let comparison = if op == CmpOp::Eq { "=" } else { "<>" };
                format!("{owner} IN (SELECT {select} FROM {from} WHERE {alias}.marker {comparison} {literal})")
            }
            CmpOp::In | CmpOp::NotIn => {
                let Value::List { items } = value else {
                    return "0".to_string();
                };
                let list = self.text_list(items, |text| text.to_ascii_uppercase());
                let Some(list) = list else {
                    // An empty list of operands: `in` is false, and `not in` is
                    // "present and not equal to anything", i.e. present.
                    return match op {
                        CmpOp::In => "0".to_string(),
                        _ => format!("{owner} IN (SELECT {select} FROM {from})"),
                    };
                };
                let membership = if op == CmpOp::In { "IN" } else { "NOT IN" };
                format!("{owner} IN (SELECT {select} FROM {from} WHERE {alias}.marker {membership} ({list}))")
            }
            _ => "0".to_string(),
        }
    }

    /// `priority` reads `block_planning.priority`, which is written from the
    /// block's projection independently of the task marker (§3.2 M2) — a
    /// markerless `[#A]` block has a row here and none in `tasks`.
    ///
    /// lsdoc's grammar accepts `[#X]` for exactly ONE ASCII character, so the
    /// stored value is always a single ASCII char and the walk's
    /// `eq_ignore_ascii_case` is exactly "equal to the operand in one of its two
    /// ASCII cases". Binding both spellings keeps the leaf on
    /// `block_planning_priority_idx` instead of wrapping the column in
    /// `upper()`, which would forfeit the seek.
    fn priority(&mut self, op: CmpOp, value: &Value, b: &str) -> String {
        let alias = self.alias("bp");
        let from = format!("block_planning {alias}");
        let select = format!("{alias}.block_id");
        let owner = format!("{b}.block_id");
        let present = format!("{alias}.priority IS NOT NULL");
        match op {
            CmpOp::IsSet => format!("{owner} IN (SELECT {select} FROM {from} WHERE {present})"),
            CmpOp::IsNotSet => {
                format!("{owner} NOT IN (SELECT {select} FROM {from} WHERE {present})")
            }
            CmpOp::Eq | CmpOp::NotEq => {
                let Value::Text { text } = value else {
                    return "0".to_string();
                };
                let list = self.ascii_case_pair(text);
                let membership = if op == CmpOp::Eq { "IN" } else { "NOT IN" };
                format!(
                    "{owner} IN (SELECT {select} FROM {from} WHERE {present} AND {alias}.priority {membership} ({list}))"
                )
            }
            CmpOp::In | CmpOp::NotIn => {
                let Value::List { items } = value else {
                    return "0".to_string();
                };
                let mut spellings: Vec<String> = Vec::new();
                for item in items {
                    if let Value::Text { text } = item {
                        spellings.push(text.to_ascii_lowercase());
                        spellings.push(text.to_ascii_uppercase());
                    }
                }
                spellings.sort();
                spellings.dedup();
                if spellings.is_empty() {
                    return match op {
                        CmpOp::In => "0".to_string(),
                        _ => format!("{owner} IN (SELECT {select} FROM {from} WHERE {present})"),
                    };
                }
                let list = spellings
                    .into_iter()
                    .map(|text| self.bind(PhysicalQueryValue::Text(text)))
                    .collect::<Vec<_>>()
                    .join(", ");
                let membership = if op == CmpOp::In { "IN" } else { "NOT IN" };
                format!(
                    "{owner} IN (SELECT {select} FROM {from} WHERE {present} AND {alias}.priority {membership} ({list}))"
                )
            }
            _ => "0".to_string(),
        }
    }

    /// `scheduled` / `deadline` read `block_planning`. Presence is the TEXT
    /// column (a malformed `<2026-13-45 …>` has presence and no day, E1) and
    /// every ordering comparison is on the `*_day` ordinal, which is exactly
    /// what `eval_planning` does with `planning_day`.
    fn planning(&mut self, op: CmpOp, value: &Value, b: &str, field: &str) -> String {
        let alias = self.alias("bp");
        let from = format!("block_planning {alias}");
        let select = format!("{alias}.block_id");
        let owner = format!("{b}.block_id");
        let present = format!("{alias}.{field} IS NOT NULL");
        match op {
            CmpOp::IsSet => {
                return format!("{owner} IN (SELECT {select} FROM {from} WHERE {present})")
            }
            CmpOp::IsNotSet => {
                return format!("{owner} NOT IN (SELECT {select} FROM {from} WHERE {present})")
            }
            _ => {}
        }
        let column = format!("{alias}.{field}_day");
        let Some(test) = self.day_comparison(op, value, &column) else {
            return "0".to_string();
        };
        format!("{owner} IN (SELECT {select} FROM {from} WHERE {test})")
    }

    /// `refs` is OG's `:block/path-refs`: the row's own normalized refs, the
    /// refs of every ancestor **of the row that established the evaluation
    /// context**, and that context's page.
    ///
    /// **At the anchor** the context is the row itself, and the set is exactly
    /// what §5.8 materializes as `block_path_refs` — one table, one probe.
    ///
    /// **Inside a `children` predicate it is not.** `eval_block_leaf`'s
    /// `Rel::Children` arm calls `eval_block(pred, child, ancestor_refs, ctx)`
    /// with `ancestor_refs` passed DOWN UNCHANGED, and `dfs_path_refs::enter`
    /// fires BEFORE a node's own refs join the multiset — so the nested row is
    /// tested against
    ///
    /// > `own(nested)` ∪ `ancestors(anchor)` ∪ `{page}`
    ///
    /// which is the anchor's context, not the nested row's, at EVERY depth: two
    /// levels down the multiset is still the anchor's, because each level passed
    /// the same value on.
    ///
    /// That set is not `block_path_refs(nested)` (which also holds the anchor's
    /// own refs and each intervening parent's), and it must NOT be computed by
    /// subtracting the parent's own names from anything: the same name may reach
    /// the nested row from a grandparent, from the page, or from the row itself,
    /// and subtracting would delete a name the walk still sees. It is instead
    /// built from three STORED facts, unioned, never differenced:
    ///
    /// | Term | Source | Why it is exactly right |
    /// |---|---|---|
    /// | `own(nested)` | `block_own_refs` | R1's explicit own-reference facts — `BlockProjection::refs_norm`, the walk's own `own` |
    /// | `ancestors(anchor)` ∪ `{page}` | `block_path_refs(anchor.parent_block_id)` | the parent's closure IS `ancestors(anchor)` ∪ `{page}` by §5.8's definition, so the ancestor context needs no new table and no subtraction |
    /// | `{page}` | `pages.name_key` of the anchor's page | the anchor may be a ROOT block, where the middle term is empty and the page is still in the closure |
    ///
    /// `pages.name_key` is `refs::page_key`, which IS `refs::normalize` — the
    /// same fold `eval_refs` applies to `ctx.page_name` — and the empty guard
    /// reproduces `closure_names`' own `!name.is_empty()` filter, so a page whose
    /// name normalizes away contributes nothing on either engine.
    ///
    /// A block and its ancestors are always on ONE page, which is why this
    /// relation stays page-local ([`crate::query::ir::PageLocality`]) even
    /// though it reads three tables: references compare STORED NAMES and never
    /// traverse the reference graph.
    fn refs(&mut self, quant: Quant, pred: &Filter, scope: BlockScope<'_>) -> String {
        // The walk's fast path: for the ONE predicate shape v1 accepts, `Every`
        // answers membership exactly as `Any` does (`eval_refs`'s
        // `single_ref_name` arm). Reproduced rather than corrected, because
        // `walk == SQL` is the contract.
        let quant = match (quant, pred.ref_name()) {
            (Quant::Every, Some(_)) => Quant::Any,
            (quant, _) => quant,
        };
        if scope.is_anchor() {
            let alias = self.alias("r");
            let owner = format!("{}.block_id", scope.alias);
            return self.quantified(&owner, quant, |compiler, invert| {
                let column = format!("{alias}.normalized_name");
                let predicate = compiler.name_element(pred, &column, refs::normalize);
                compiler.exists_subquery(
                    &format!("{alias}.block_id"),
                    &format!("block_path_refs {alias}"),
                    &[],
                    predicate,
                    invert,
                )
            });
        }
        // Nested: one `exists` over the union of the three terms. `Any` is that
        // existence, `None` is its negation, and `Every` is "no element
        // VIOLATES", i.e. the same existence over the negated predicate —
        // `quantify`'s three answers, with the empty union giving `Any` false
        // and `Every` true (Q5) because an empty `OR` folds to `0`.
        let exists = |compiler: &mut Self, invert: bool| -> String {
            let mut arms: Vec<String> = Vec::new();
            // `own(nested)` — R1's explicit own-reference facts, seeked on the
            // `(block_id, normalized_name)` primary key.
            let own = compiler.alias("or");
            let owner = format!("{}.block_id", scope.alias);
            let predicate =
                compiler.name_element(pred, &format!("{own}.normalized_name"), refs::normalize);
            if let Some(sub) = compiler.exists_subquery(
                &format!("{own}.block_id"),
                &format!("block_own_refs {own}"),
                &[format!("{own}.block_id = {owner}")],
                predicate,
                invert,
            ) {
                arms.push(format!("{owner} IN ({sub})"));
            }
            // `ancestors(anchor)` ∪ `{page}` — the ANCHOR's parent's own §5.8
            // closure. J1: `parent_block_id` is nullable, and a root anchor has
            // no ancestor context at all, so the guard is what keeps this arm
            // two-valued under `NOT`.
            let ancestors = compiler.alias("ar");
            let parent = format!("{}.parent_block_id", scope.anchor);
            let predicate = compiler.name_element(
                pred,
                &format!("{ancestors}.normalized_name"),
                refs::normalize,
            );
            if let Some(sub) = compiler.exists_subquery(
                &format!("{ancestors}.block_id"),
                &format!("block_path_refs {ancestors}"),
                &[format!("{ancestors}.block_id = {parent}")],
                predicate,
                invert,
            ) {
                arms.push(format!("({parent} IS NOT NULL AND {parent} IN ({sub}))"));
            }
            // `{page}` — named separately because a ROOT anchor has no parent
            // row to carry it. `name_key <> ''` reproduces `closure_names`' own
            // empty-name filter, which the two ref tables get from their column
            // CHECK constraints and `pages` does not.
            let page = compiler.alias("pr");
            let page_owner = format!("{}.page_id", scope.anchor);
            let predicate =
                compiler.name_element(pred, &format!("{page}.name_key"), refs::normalize);
            if let Some(sub) = compiler.exists_subquery(
                &format!("{page}.page_id"),
                &format!("pages {page}"),
                &[
                    format!("{page}.page_id = {page_owner}"),
                    format!("{page}.name_key <> ''"),
                ],
                predicate,
                invert,
            ) {
                arms.push(format!("{page_owner} IN ({sub})"));
            }
            fold_or(arms)
        };
        match quant {
            Quant::Any => exists(self, false),
            Quant::None => fold_not(exists(self, false)),
            Quant::Every => fold_not(exists(self, true)),
        }
    }

    /// `tags` is the block's or page's own inline `#tag` / Org headline tags.
    /// `tags.tag_key` is `refs::page_key(tag)` because a tag IS a page reference
    /// (§3.2 K18), which is the same fold `eval_name_element` applies.
    fn tags(&mut self, quant: Quant, pred: &Filter, owner_alias: &str, owner_type: i64) -> String {
        let alias = self.alias("tg");
        let owner_column = if owner_type == OWNER_BLOCK {
            format!("{owner_alias}.block_id")
        } else {
            format!("{owner_alias}.page_id")
        };
        let owner_type_literal = self.bind(PhysicalQueryValue::Integer(owner_type));
        self.quantified(&owner_column, quant, |compiler, invert| {
            let column = format!("{alias}.tag_key");
            let predicate = compiler.name_element(pred, &column, refs::page_key);
            compiler.exists_subquery(
                &format!("{alias}.owner_id"),
                &format!("tags {alias}"),
                &[format!("{alias}.owner_type = {owner_type_literal}")],
                predicate,
                invert,
            )
        })
    }

    /// `children` are the block's DIRECT children (A1):
    /// `SELECT c.parent_block_id FROM blocks c WHERE c.parent_block_id IS NOT
    /// NULL AND <pred(c)>`. The `IS NOT NULL` is J1 — without it `NOT IN` over a
    /// column that holds NULLs is NULL, not false.
    fn children(&mut self, quant: Quant, pred: &Filter, scope: BlockScope<'_>) -> String {
        let alias = self.alias("c");
        let owner = format!("{}.block_id", scope.alias);
        // The child is a fresh block row that KEEPS this scope's ancestor
        // context, which is the whole content of `eval_block_leaf`'s
        // "passes `ancestor_refs` down unchanged" (see [`Compiler::refs`]).
        let child = scope.nested(&alias);
        self.quantified(&owner, quant, |compiler, invert| {
            compiler.relation_subquery(
                &format!("{alias}.parent_block_id"),
                &format!("blocks {alias}"),
                &[format!("{alias}.parent_block_id IS NOT NULL")],
                pred,
                Row::Block(child),
                invert,
            )
        })
    }

    /// The to-one `page` relation of a block row. All three quantifiers reduce
    /// to the predicate or its negation, exactly as `eval_block_leaf` does.
    fn page_relation(&mut self, quant: Quant, pred: &Filter, b: &str) -> String {
        let alias = self.alias("pg");
        let owner = format!("{b}.page_id");
        let hit = self.relation_subquery(
            &format!("{alias}.page_id"),
            &format!("pages {alias}"),
            &[],
            pred,
            Row::Page(&alias),
            false,
        );
        match (quant, hit) {
            (Quant::Any | Quant::Every, Some(hit)) => format!("{owner} IN ({hit})"),
            (Quant::None, Some(hit)) => format!("{owner} NOT IN ({hit})"),
            (Quant::Any | Quant::Every, None) => "0".to_string(),
            (Quant::None, None) => "1".to_string(),
        }
    }

    // -----------------------------------------------------------------------
    // Page-row leaves
    // -----------------------------------------------------------------------

    fn leaf_page(&mut self, leaf: &Leaf, p: &str) -> String {
        match leaf {
            Leaf::Attr { attr, op, value } => match attr {
                Attr::Name => self.page_name(*op, value, p),
                Attr::Journal => match (op, value) {
                    // `pages.text_kind`, not `journal_day IS NOT NULL`: a journal
                    // page whose stem does not parse has kind Journal and no day,
                    // and the walk reads the kind (`eval_page`'s `Attr::Journal`).
                    (CmpOp::Eq, Value::Bool { value: true }) => {
                        format!("{p}.text_kind = {TEXT_KIND_JOURNAL}")
                    }
                    (CmpOp::Eq, Value::Bool { value: false }) => {
                        format!("{p}.text_kind <> {TEXT_KIND_JOURNAL}")
                    }
                    _ => "0".to_string(),
                },
                Attr::Day => self.page_day(*op, value, p),
                Attr::Namespace => self.page_namespace(*op, value, p),
                _ => "0".to_string(),
            },
            Leaf::Rel { rel, quant, pred } => match rel {
                Rel::Props => self.props(*quant, pred, p, "page_id", OWNER_PAGE),
                // A page's own refs, its blocks and its tag table are not walked
                // by `eval_page`; its `_ => false` arm is reproduced here.
                _ => "0".to_string(),
            },
        }
    }

    /// `pages.name_key` is `refs::page_key(name)` — the same page-identity fold
    /// `eval_page_name` applies to both sides of its comparison.
    fn page_name(&mut self, op: CmpOp, value: &Value, p: &str) -> String {
        let column = format!("{p}.name_key");
        match (op, value) {
            (CmpOp::Eq, Value::Text { text }) => {
                let literal = self.bind(PhysicalQueryValue::Text(refs::page_key(text)));
                format!("{column} = {literal}")
            }
            (CmpOp::NotEq, Value::Text { text }) => {
                let literal = self.bind(PhysicalQueryValue::Text(refs::page_key(text)));
                format!("{column} <> {literal}")
            }
            (CmpOp::StartsWith, Value::Text { text }) => {
                // A range on the key column, which is what makes `(namespace X)`
                // and `page.name starts_with` seek `pages_name_key_idx` (§5.7).
                let prefix = page_prefix_key(text);
                self.prefix_range(&column, &prefix)
            }
            (CmpOp::Like, Value::Text { text }) => {
                let pattern = self.bind(PhysicalQueryValue::Text(canonical_fold(text)));
                format!("{column} LIKE {pattern} ESCAPE '\\'")
            }
            (CmpOp::In, Value::List { items }) => {
                match self.text_list(items, |text| refs::page_key(text)) {
                    Some(list) => format!("{column} IN ({list})"),
                    None => "0".to_string(),
                }
            }
            // `eval_page_name` has no other arm; `not in` on a page name is
            // false in the walk, so it is false here.
            _ => "0".to_string(),
        }
    }

    /// `page.day` reads `pages.journal_day`, the ONE journal-day answer
    /// (`JournalDays::day`) that also fills `PageEntry::date_key`, which is what
    /// the walk compares.
    fn page_day(&mut self, op: CmpOp, value: &Value, p: &str) -> String {
        let column = format!("{p}.journal_day");
        match op {
            CmpOp::IsSet => return format!("{column} IS NOT NULL"),
            CmpOp::IsNotSet => return format!("{column} IS NULL"),
            _ => {}
        }
        self.day_comparison(op, value, &column)
            .unwrap_or_else(|| "0".to_string())
    }

    /// The Tine-only `page.namespace` leaf: the immediate parent segment of the
    /// page-identity key (M20). There is no `namespace_key` column — the
    /// `name_key` range measured sufficient for the bounded `starts_with` form,
    /// and this unbounded form is the one §5.7's table already marks `no`.
    fn page_namespace(&mut self, op: CmpOp, value: &Value, p: &str) -> String {
        let column = format!("{p}.name_key");
        let has_parent = format!("instr({column}, '/') > 0");
        match op {
            CmpOp::IsSet => return has_parent,
            CmpOp::IsNotSet => return format!("instr({column}, '/') = 0"),
            _ => {}
        }
        // `name_key` is already fully lowercased, so the walk's ASCII-insensitive
        // comparison is equality against the ASCII-lowercased operand.
        let equals = |compiler: &mut Self, text: &str| -> String {
            let parent = text.to_ascii_lowercase();
            let head = compiler.bind(PhysicalQueryValue::Text(format!("{parent}/")));
            let width = parent.chars().count() + 1;
            format!(
                "substr({column}, 1, {width}) = {head} AND instr(substr({column}, {}), '/') = 0",
                width + 1
            )
        };
        match (op, value) {
            (CmpOp::Eq, Value::Text { text }) => {
                let test = equals(self, text);
                format!("({test})")
            }
            (CmpOp::NotEq, Value::Text { text }) => {
                let test = equals(self, text);
                format!("({has_parent} AND NOT ({test}))")
            }
            (CmpOp::In, Value::List { items }) => {
                let tests: Vec<String> = items
                    .iter()
                    .filter_map(|item| match item {
                        Value::Text { text } => Some(text.clone()),
                        _ => None,
                    })
                    .collect();
                if tests.is_empty() {
                    return "0".to_string();
                }
                let parts: Vec<String> = tests
                    .iter()
                    .map(|text| {
                        let test = equals(self, text);
                        format!("({test})")
                    })
                    .collect();
                format!("({})", parts.join(" OR "))
            }
            (CmpOp::NotIn, Value::List { items }) => {
                let tests: Vec<String> = items
                    .iter()
                    .filter_map(|item| match item {
                        Value::Text { text } => Some(text.clone()),
                        _ => None,
                    })
                    .collect();
                if tests.is_empty() {
                    return has_parent;
                }
                let parts: Vec<String> = tests
                    .iter()
                    .map(|text| {
                        let test = equals(self, text);
                        format!("({test})")
                    })
                    .collect();
                format!("({has_parent} AND NOT ({}))", parts.join(" OR "))
            }
            _ => "0".to_string(),
        }
    }

    // -----------------------------------------------------------------------
    // §3.3 — property elements, the two-level form
    // -----------------------------------------------------------------------

    /// The five property forms (§3.3), lowered as §5.1 requires: the presence
    /// probe over `properties` and the atom probe over `property_atoms`, each
    /// ONE subquery carrying its whole conjunction.
    ///
    /// `property Every` is presence ∧ "no atom violates" — the walk's
    /// `present && atoms.iter().all(matches)` — which is why it needs both
    /// subqueries and a generic `Every` needs only one.
    fn props(
        &mut self,
        quant: Quant,
        pred: &Filter,
        owner_alias: &str,
        owner_id_column: &str,
        owner_type: i64,
    ) -> String {
        // Every property leaf the parsers build carries the `key = 'k'`
        // conjunct; without it the leaf names no relation to quantify over.
        let Some(key) = pred.props_key() else {
            return "0".to_string();
        };
        let key_norm = property_key_norm(&key);
        let owner = format!("{owner_alias}.{owner_id_column}");
        let owner_type_literal = self.bind(PhysicalQueryValue::Integer(owner_type));
        let key_literal = self.bind(PhysicalQueryValue::Text(key_norm.clone()));
        let presence = {
            let alias = self.alias("pr");
            format!(
                "{owner} IN (SELECT {alias}.owner_id FROM properties {alias} \
                 WHERE {alias}.normalized_name = {key_literal} \
                 AND {alias}.owner_type = {owner_type_literal})"
            )
        };

        let Some(test) = pred.props_atom_test() else {
            // Bare presence: `prop('k') is not null` is `Any(key='k')` and
            // `is null` is `None(key='k')`; property `Every` carries presence.
            return match quant {
                Quant::Any | Quant::Every => presence,
                Quant::None => fold_not(presence),
            };
        };

        // `= ''` and `all-page-tags`'s `atom_count > 0` are properties of the
        // whole atom list, not of one atom, and are scoped by presence.
        if let Some(count) = self.atom_count_test(&test, &owner, &key_literal, &owner_type_literal)
        {
            let hit = fold_and(vec![presence, count]);
            return match quant {
                Quant::Any | Quant::Every => hit,
                Quant::None => fold_not(hit),
            };
        }

        let effective = self
            .inputs
            .registry
            .effective_type(&key_norm)
            .unwrap_or(ObservedType::Text);
        let atom_subquery = |compiler: &mut Self, invert: bool| -> Option<String> {
            let alias = compiler.alias("a");
            let predicate = compiler.atom_test(&test, &alias, effective);
            compiler.exists_subquery(
                &format!("{alias}.owner_id"),
                &format!("property_atoms {alias}"),
                &[
                    format!("{alias}.normalized_name = {key_literal}"),
                    format!("{alias}.owner_type = {owner_type_literal}"),
                ],
                predicate,
                invert,
            )
        };
        match quant {
            Quant::Any => match atom_subquery(self, false) {
                Some(sub) => format!("{owner} IN ({sub})"),
                None => "0".to_string(),
            },
            Quant::None => match atom_subquery(self, false) {
                Some(sub) => format!("{owner} NOT IN ({sub})"),
                None => "1".to_string(),
            },
            // Presence still has to hold: the walk's `present && all(...)` is
            // vacuously true over an empty atom list only when the property is
            // there at all.
            Quant::Every => match atom_subquery(self, true) {
                Some(sub) => format!("({presence} AND {owner} NOT IN ({sub}))"),
                None => presence,
            },
        }
    }

    /// `Some(<sql>)` when the property test reads only `atom_count`.
    fn atom_count_test(
        &mut self,
        test: &Filter,
        owner: &str,
        key_literal: &str,
        owner_type_literal: &str,
    ) -> Option<String> {
        let Filter::Leaf {
            leaf:
                Leaf::Attr {
                    attr: Attr::AtomCount,
                    op,
                    value: Value::Number { number },
                },
        } = test
        else {
            return None;
        };
        let comparison = match op {
            CmpOp::Eq => "=",
            CmpOp::NotEq => "<>",
            CmpOp::Gt => ">",
            CmpOp::Ge => ">=",
            CmpOp::Lt => "<",
            CmpOp::Le => "<=",
            _ => return None,
        };
        let alias = self.alias("ac");
        let bound = self.bind(PhysicalQueryValue::Real(*number));
        // A correlated COUNT rather than a quantifier: cardinality is a property
        // of the whole atom list, and the `(owner_type, owner_id,
        // normalized_name)` primary-key prefix makes it a seek.
        Some(format!(
            "(SELECT COUNT(*) FROM property_atoms {alias} \
             WHERE {alias}.owner_type = {owner_type_literal} \
             AND {alias}.owner_id = {owner} \
             AND {alias}.normalized_name = {key_literal}) {comparison} {bound}"
        ))
    }

    /// The predicate over ONE atom, coerced by the key's effective type (§6.3).
    /// Mirrors `eval_atom_test` / `eval_atom_value` clause for clause.
    fn atom_test(&mut self, test: &Filter, a: &str, effective: ObservedType) -> String {
        match test {
            Filter::True => "1".to_string(),
            Filter::False | Filter::Raw { .. } => "0".to_string(),
            Filter::And { items } if items.is_empty() => "1".to_string(),
            Filter::Or { items } if items.is_empty() => "0".to_string(),
            Filter::And { items } => fold_and(
                items
                    .iter()
                    .map(|item| self.atom_test(item, a, effective))
                    .collect(),
            ),
            Filter::Or { items } => fold_or(
                items
                    .iter()
                    .map(|item| self.atom_test(item, a, effective))
                    .collect(),
            ),
            Filter::Not { inner } => fold_not(self.atom_test(inner, a, effective)),
            Filter::Off { .. } => {
                debug_assert!(false, "Off must be removed before lowering (§3.5)");
                "1".to_string()
            }
            Filter::Leaf {
                leaf: Leaf::Attr { attr, op, value },
            } => match attr {
                Attr::Value => self.atom_value(*op, value, a, effective),
                // The leaf's own scoping conjunct, already applied above.
                Attr::Key => "1".to_string(),
                _ => "0".to_string(),
            },
            Filter::Leaf { .. } => "0".to_string(),
        }
    }

    fn atom_value(&mut self, op: CmpOp, value: &Value, a: &str, effective: ObservedType) -> String {
        if op == CmpOp::IsSet {
            return "1".to_string();
        }
        match effective {
            ObservedType::Number => self.atom_number(op, value, &format!("{a}.atom_num")),
            ObservedType::Date => self
                .day_comparison(op, value, &format!("{a}.atom_day"))
                .unwrap_or_else(|| "0".to_string()),
            // Text, ref and checkbox atoms all compare their NFC-lowercased key.
            _ => self.atom_text(op, value, &format!("{a}.atom_key")),
        }
    }

    /// `compare_number`, in SQL. `atom_num` is NULL for an atom that does not
    /// coerce, and an atom whose typed value is absent fails EVERY comparison
    /// including `!=` (K3) — which is exactly what the `IS NOT NULL` guard says.
    fn atom_number(&mut self, op: CmpOp, value: &Value, column: &str) -> String {
        let operand = |value: &Value| -> Option<f64> {
            match value {
                Value::Number { number } => Some(*number),
                Value::Text { text } => text.trim().parse::<f64>().ok().filter(|n| n.is_finite()),
                Value::Date { literal } => {
                    literal.trim().parse::<f64>().ok().filter(|n| n.is_finite())
                }
                _ => None,
            }
        };
        match (op, value) {
            (CmpOp::Between, Value::List { items }) if items.len() == 2 => {
                match (operand(&items[0]), operand(&items[1])) {
                    (Some(low), Some(high)) => {
                        let (low, high) = if low > high { (high, low) } else { (low, high) };
                        let low = self.bind(PhysicalQueryValue::Real(low));
                        let high = self.bind(PhysicalQueryValue::Real(high));
                        format!("({column} IS NOT NULL AND {column} BETWEEN {low} AND {high})")
                    }
                    _ => "0".to_string(),
                }
            }
            (CmpOp::In | CmpOp::NotIn, Value::List { items }) => {
                let bounds: Vec<f64> = items.iter().filter_map(operand).collect();
                if bounds.is_empty() {
                    return match op {
                        CmpOp::In => "0".to_string(),
                        // `not in ()` is vacuously true for a coercible atom.
                        _ => format!("({column} IS NOT NULL)"),
                    };
                }
                let list = bounds
                    .into_iter()
                    .map(|bound| self.bind(PhysicalQueryValue::Real(bound)))
                    .collect::<Vec<_>>()
                    .join(", ");
                let membership = if op == CmpOp::In { "IN" } else { "NOT IN" };
                format!("({column} IS NOT NULL AND {column} {membership} ({list}))")
            }
            (op, value) => {
                let Some(bound) = operand(value) else {
                    return "0".to_string();
                };
                let comparison = match op {
                    CmpOp::Eq => "=",
                    CmpOp::NotEq => "<>",
                    CmpOp::Lt => "<",
                    CmpOp::Le => "<=",
                    CmpOp::Gt => ">",
                    CmpOp::Ge => ">=",
                    _ => return "0".to_string(),
                };
                let bound = self.bind(PhysicalQueryValue::Real(bound));
                format!("({column} IS NOT NULL AND {column} {comparison} {bound})")
            }
        }
    }

    /// `compare_atom_text`, in SQL. `atom_key` is `NOT NULL`, so no null guard
    /// is needed — and adding one would hide a future nullable column.
    fn atom_text(&mut self, op: CmpOp, value: &Value, column: &str) -> String {
        let operand = |value: &Value| -> Option<String> {
            match value {
                Value::Text { text } => Some(atom_key(text)),
                Value::Number { number } => Some(atom_key(&format_number(*number))),
                Value::Date { literal } => Some(atom_key(literal)),
                Value::Bool { value } => Some(if *value {
                    "true".into()
                } else {
                    "false".into()
                }),
                _ => None,
            }
        };
        match (op, value) {
            (CmpOp::In | CmpOp::NotIn, Value::List { items }) => {
                let keys: Vec<String> = items.iter().filter_map(&operand).collect();
                if keys.is_empty() {
                    return match op {
                        CmpOp::In => "0".to_string(),
                        _ => "1".to_string(),
                    };
                }
                let list = keys
                    .into_iter()
                    .map(|key| self.bind(PhysicalQueryValue::Text(key)))
                    .collect::<Vec<_>>()
                    .join(", ");
                let membership = if op == CmpOp::In { "IN" } else { "NOT IN" };
                format!("{column} {membership} ({list})")
            }
            (CmpOp::Like, value) => match operand(value) {
                Some(pattern) => {
                    let pattern = self.bind(PhysicalQueryValue::Text(pattern));
                    format!("{column} LIKE {pattern} ESCAPE '\\'")
                }
                None => "0".to_string(),
            },
            (CmpOp::StartsWith, value) => match operand(value) {
                Some(prefix) => {
                    let pattern = self.bind(PhysicalQueryValue::Text(format!(
                        "{}%",
                        like_escape(&prefix)
                    )));
                    format!("{column} LIKE {pattern} ESCAPE '\\'")
                }
                None => "0".to_string(),
            },
            (op, value) => {
                let Some(key) = operand(value) else {
                    return "0".to_string();
                };
                let comparison = match op {
                    CmpOp::Eq => "=",
                    // K3: a text atom always coerces, so `!=` is plain
                    // inequality on the comparison key.
                    CmpOp::NotEq => "<>",
                    _ => return "0".to_string(),
                };
                let key = self.bind(PhysicalQueryValue::Text(key));
                format!("{column} {comparison} {key}")
            }
        }
    }

    // -----------------------------------------------------------------------
    // Shared comparison helpers
    // -----------------------------------------------------------------------

    /// `compare_day`, in SQL, over a NULLABLE day-ordinal column.
    ///
    /// The asymmetry is the walk's: `>=` and `<=` are `is_none_or` — an
    /// unresolvable bound imposes NO limit — while `>`, `<`, `=` and `!=` are
    /// `is_some_and` and are false without one.
    fn day_comparison(&mut self, op: CmpOp, value: &Value, column: &str) -> Option<String> {
        let today = self.inputs.today;
        let resolve = |value: &Value| -> Option<i64> {
            match value {
                Value::Date { literal } => crate::query::resolve_date_token(literal, today),
                Value::Number { number } => Some(*number as i64),
                _ => None,
            }
        };
        match (op, value) {
            (CmpOp::Between, Value::List { items }) if items.len() == 2 => {
                let (low, high) = (resolve(&items[0]), resolve(&items[1]));
                // OG's `build-between-two-arg` sorts its two resolved bounds.
                let (low, high) = match (low, high) {
                    (Some(low), Some(high)) if low > high => (Some(high), Some(low)),
                    pair => pair,
                };
                let mut clauses = vec![format!("{column} IS NOT NULL")];
                if let Some(low) = low {
                    let low = self.bind(PhysicalQueryValue::Integer(low));
                    clauses.push(format!("{column} >= {low}"));
                }
                if let Some(high) = high {
                    let high = self.bind(PhysicalQueryValue::Integer(high));
                    clauses.push(format!("{column} <= {high}"));
                }
                Some(format!("({})", clauses.join(" AND ")))
            }
            (CmpOp::Ge | CmpOp::Le, value) => {
                let Some(bound) = resolve(value) else {
                    // `is_none_or`: no bound, so any day passes.
                    return Some(format!("{column} IS NOT NULL"));
                };
                let comparison = if op == CmpOp::Ge { ">=" } else { "<=" };
                let bound = self.bind(PhysicalQueryValue::Integer(bound));
                Some(format!(
                    "({column} IS NOT NULL AND {column} {comparison} {bound})"
                ))
            }
            (CmpOp::Gt | CmpOp::Lt | CmpOp::Eq | CmpOp::NotEq, value) => {
                let bound = resolve(value)?;
                let comparison = match op {
                    CmpOp::Gt => ">",
                    CmpOp::Lt => "<",
                    CmpOp::Eq => "=",
                    _ => "<>",
                };
                let bound = self.bind(PhysicalQueryValue::Integer(bound));
                Some(format!(
                    "({column} IS NOT NULL AND {column} {comparison} {bound})"
                ))
            }
            _ => None,
        }
    }

    /// The predicate over a ref or tag element, whose only attribute is `name`.
    /// `normalize` is the fold the PRODUCER applied to the stored column, passed
    /// in so the two can never be spelled differently at one call site.
    fn name_element(
        &mut self,
        pred: &Filter,
        column: &str,
        normalize: fn(&str) -> String,
    ) -> String {
        match pred {
            Filter::True => "1".to_string(),
            Filter::False => "0".to_string(),
            Filter::And { items } if items.is_empty() => "1".to_string(),
            Filter::Or { items } if items.is_empty() => "0".to_string(),
            Filter::And { items } => {
                let parts: Vec<String> = items
                    .iter()
                    .map(|item| self.name_element(item, column, normalize))
                    .collect();
                format!("({})", parts.join(" AND "))
            }
            Filter::Or { items } => {
                let parts: Vec<String> = items
                    .iter()
                    .map(|item| self.name_element(item, column, normalize))
                    .collect();
                format!("({})", parts.join(" OR "))
            }
            Filter::Not { inner } => {
                let inner = self.name_element(inner, column, normalize);
                format!("(NOT {inner})")
            }
            Filter::Leaf {
                leaf:
                    Leaf::Attr {
                        attr: Attr::Name,
                        op: CmpOp::Eq,
                        value: Value::Text { text },
                    },
            } => {
                let literal = self.bind(PhysicalQueryValue::Text(normalize(text)));
                format!("{column} = {literal}")
            }
            // `eval_name_element` answers false for every other shape.
            _ => "0".to_string(),
        }
    }

    /// A literal prefix as a half-open range on an ordered key column — the form
    /// SQLite can seek (`name_key > ? AND name_key < ?`), unlike `LIKE 'p%'`,
    /// which it can only use with an ASCII-safe collation.
    fn prefix_range(&mut self, column: &str, prefix: &str) -> String {
        if prefix.is_empty() {
            return "1".to_string();
        }
        let Some(upper) = prefix_upper_bound(prefix) else {
            // The prefix ends at the last representable scalar value; a LIKE
            // keeps the leaf correct, and §5.7 only promises a plan for shapes
            // that can have one.
            let pattern = self.bind(PhysicalQueryValue::Text(format!(
                "{}%",
                like_escape(prefix)
            )));
            return format!("{column} LIKE {pattern} ESCAPE '\\'");
        };
        let low = self.bind(PhysicalQueryValue::Text(prefix.to_string()));
        let high = self.bind(PhysicalQueryValue::Text(upper));
        format!("({column} >= {low} AND {column} < {high})")
    }

    /// A bound `IN` list built from the `Text` items of a `List` value, with
    /// each item put through the column's own normalization. `None` when no item
    /// is a text literal, which is the walk's "matches nothing".
    fn text_list(&mut self, items: &[Value], normalize: impl Fn(&str) -> String) -> Option<String> {
        let normalized: Vec<String> = items
            .iter()
            .filter_map(|item| match item {
                Value::Text { text } => Some(normalize(text)),
                _ => None,
            })
            .collect();
        if normalized.is_empty() {
            return None;
        }
        Some(
            normalized
                .into_iter()
                .map(|text| self.bind(PhysicalQueryValue::Text(text)))
                .collect::<Vec<_>>()
                .join(", "),
        )
    }

    /// Both ASCII cases of one operand, as a bound list.
    fn ascii_case_pair(&mut self, text: &str) -> String {
        let lower = text.to_ascii_lowercase();
        let upper = text.to_ascii_uppercase();
        let mut spellings = vec![lower];
        if !spellings.contains(&upper) {
            spellings.push(upper);
        }
        spellings
            .into_iter()
            .map(|text| self.bind(PhysicalQueryValue::Text(text)))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

// ---------------------------------------------------------------------------
// §5.10 — the shared Match payload, and the candidate needle
// ---------------------------------------------------------------------------

/// What the compiler does with ONE `content match` payload.
///
/// Owned rather than borrowed so that reading the shared parse does not hold a
/// borrow of the compiler across the `&mut self` calls that consume it; the
/// clone is a handful of small strings per leaf, and it is the SAME parsed
/// value — not a second parse (I-12).
enum MatchProgram {
    /// `Empty` — a blank or exclusion-only query — or a payload the walk never
    /// collected. Both are false in `Matcher::matches`.
    AlwaysFalse,
    /// The whole-query `/pattern/` form, already restricted by
    /// `common_regex_pattern` at parse time. `compiled` is `None` for a pattern
    /// the regex engine rejected, which §4.3.2 retains as a leaf matching
    /// false — still a REGEX leaf for §5.10's plan classes, just one that needs
    /// no engine to answer. When it compiled, this carries a CLONE of the
    /// walk's own program, never a second `Matcher::parse` or `Regex::new`.
    Regex {
        compiled: Option<regex::Regex>,
    },
    Boolean(Vec<AndGroup>),
}

/// Read the shared parse for one `content match` payload.
fn match_program(compiled: &CompiledLeaves, source: &str) -> MatchProgram {
    match compiled.match_program(source) {
        None | Some(Matcher::Empty) => MatchProgram::AlwaysFalse,
        Some(Matcher::InvalidRegex(_)) => MatchProgram::Regex { compiled: None },
        Some(Matcher::Regex(regex)) => MatchProgram::Regex {
            compiled: Some(regex.clone()),
        },
        Some(Matcher::Boolean(groups)) => MatchProgram::Boolean(groups.clone()),
    }
}

/// The FTS candidate needle for one OR arm, or `None` when the arm has none
/// (SPEC §5.10, verbatim):
///
/// > scan positive folded terms in order, excluding NUL-bearing terms, split
/// > each with Rust `str::split_whitespace` (the producers' rule), and take its
/// > first whitespace-free run of at least three Unicode scalars. Use the first
/// > such run as the candidate needle, not the entire phrase.
///
/// **Never a negative term.** A negative term says the text does NOT contain
/// it; using it as a candidate bound would select exactly the rows the arm
/// rejects. Three scalars is the trigram tokenizer's own floor, not a tuning
/// constant: a shorter needle produces no token and would match nothing.
fn fts_candidate_needle(group: &AndGroup) -> Option<&str> {
    group
        .iter()
        .filter(|term| !term.negated && !term.text.contains('\0'))
        .find_map(|term| {
            term.text
                .split_whitespace()
                .find(|run| run.chars().count() >= 3)
        })
}

/// One FTS5 string literal holding `needle` as a single phrase: FTS5 quotes
/// with `"` and escapes an embedded `"` by doubling it. Quoting is what keeps
/// the needle a LITERAL rather than an expression — `-`, `*`, `(`, `:` and the
/// bare words `AND`/`OR`/`NOT` are query syntax outside quotes.
fn fts_phrase_literal(needle: &str) -> String {
    format!("\"{}\"", needle.replace('"', "\"\""))
}

/// §5.10's plan classes for every content leaf of one filter, depth-first.
///
/// A leaf that folds to the constant false contributes nothing: it reads no
/// row, so it has no plan. Every other content leaf gets exactly one class.
fn content_plans(filter: &Filter, inputs: &LoweringInputs<'_>) -> Vec<ContentPlan> {
    let mut out = Vec::new();
    filter.for_each_leaf(&mut |leaf| {
        let Leaf::Attr {
            attr: Attr::Content,
            op,
            value: Value::Text { text },
        } = leaf
        else {
            return;
        };
        match op {
            // Every regex leaf is a regex plan class, compiled or not: §4.3.2
            // makes regex an explicitly unindexed content predicate either way.
            // Only the invalid ones reach a statement in this wave.
            CmpOp::Regex => out.push(ContentPlan::Regex),
            CmpOp::Match => match match_program(inputs.compiled, text) {
                MatchProgram::AlwaysFalse => {}
                MatchProgram::Regex { .. } => out.push(ContentPlan::Regex),
                MatchProgram::Boolean(groups) => out.push(if !inputs.fts_ready {
                    ContentPlan::FtsBuilding
                } else if groups
                    .iter()
                    .all(|group| fts_candidate_needle(group).is_some())
                {
                    ContentPlan::Fts
                } else {
                    ContentPlan::ShortUnindexable
                }),
            },
            _ => {}
        }
    });
    out
}

// ---------------------------------------------------------------------------
// §5.7 — positive boundedness
// ---------------------------------------------------------------------------

/// Is this query *positively bounded* (SPEC §5.7)?
///
/// "After pushing `not` to the leaves and removing `Off`, its root conjunction
/// contains at least one **positive** leaf whose anchor bound is `yes`." The
/// polarity is threaded rather than the tree rewritten, which is the same
/// `reverse` carrier `prisma-engines`' visitor uses.
///
/// **The table below is exhaustive** (A6): a leaf/operator pair absent from it
/// is unbounded. "Every OG head" is not a category.
pub(crate) fn positively_bounded(
    filter: &Filter,
    anchor: Anchor,
    inputs: &LoweringInputs<'_>,
) -> bool {
    let row = match anchor {
        Anchor::Block => BoundRow::Block,
        Anchor::Page => BoundRow::Page,
    };
    bounded(filter, false, row, inputs)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BoundRow {
    Block,
    Page,
}

fn bounded(filter: &Filter, negated: bool, row: BoundRow, inputs: &LoweringInputs<'_>) -> bool {
    match filter {
        // A conjunction needs ONE bounded conjunct; a disjunction needs ALL of
        // its arms bounded, because the anchor is reached once per arm.
        Filter::And { items } if !negated => {
            items.iter().any(|item| bounded(item, false, row, inputs))
        }
        Filter::And { items } => {
            !items.is_empty() && items.iter().all(|item| bounded(item, true, row, inputs))
        }
        Filter::Or { items } if !negated => {
            !items.is_empty() && items.iter().all(|item| bounded(item, false, row, inputs))
        }
        Filter::Or { items } => items.iter().any(|item| bounded(item, true, row, inputs)),
        Filter::Not { inner } => bounded(inner, !negated, row, inputs),
        Filter::Leaf { leaf } => !negated && leaf_bounds(leaf, row, inputs),
        Filter::Off { .. } | Filter::True | Filter::False | Filter::Raw { .. } => false,
    }
}

fn leaf_bounds(leaf: &Leaf, row: BoundRow, inputs: &LoweringInputs<'_>) -> bool {
    match leaf {
        Leaf::Attr { attr, op, value } => match (row, attr) {
            // `blocks` has no content index, so every content operator but
            // `match` is unbounded: `starts_with` on `content` is not
            // range-lowerable, and regex is explicitly unindexed (§4.3.2).
            //
            // `match` bounds the anchor when the FTS index is READY and EVERY
            // retained OR arm supplies a candidate needle. One unbounded arm
            // makes the whole leaf unbounded — the arms are OR-ed, so the
            // anchor is reached once per arm and a single unbounded arm
            // enumerates it (§5.10, §5.7's `Or` rule).
            (BoundRow::Block, Attr::Content) => {
                *op == CmpOp::Match
                    && inputs.fts_ready
                    && matches!(value, Value::Text { text }
                    if match match_program(inputs.compiled, text) {
                        MatchProgram::Boolean(groups) => groups
                            .iter()
                            .all(|group| fts_candidate_needle(group).is_some()),
                        _ => false,
                    })
            }
            (BoundRow::Block, Attr::Task) => matches!(
                op,
                CmpOp::Eq | CmpOp::NotEq | CmpOp::In | CmpOp::NotIn | CmpOp::IsSet
            ),
            (BoundRow::Block, Attr::Priority) => matches!(
                op,
                CmpOp::Eq | CmpOp::NotEq | CmpOp::In | CmpOp::NotIn | CmpOp::IsSet
            ),
            // No `in`: the §4.2.3 matrix rejects set membership on dates (Y4).
            (BoundRow::Block, Attr::Scheduled | Attr::Deadline) => matches!(
                op,
                CmpOp::Eq
                    | CmpOp::Lt
                    | CmpOp::Le
                    | CmpOp::Gt
                    | CmpOp::Ge
                    | CmpOp::Between
                    | CmpOp::IsSet
            ),
            (BoundRow::Page, Attr::Name) => {
                matches!(op, CmpOp::Eq | CmpOp::In | CmpOp::StartsWith)
            }
            (BoundRow::Page, Attr::Day) => matches!(
                op,
                CmpOp::Eq
                    | CmpOp::Lt
                    | CmpOp::Le
                    | CmpOp::Gt
                    | CmpOp::Ge
                    | CmpOp::Between
                    | CmpOp::IsSet
            ),
            // `page.journal` reads `pages.text_kind`, which has no index — see
            // `leaf_page`. §5.7's table assumed it lowered to `journal_day IS
            // NOT NULL`, which would answer differently for a journal page whose
            // stem does not parse, so the leaf is conservatively unbounded here.
            (BoundRow::Page, Attr::Journal) => false,
            // No `namespace_key` column: the `name_key` range covers the bounded
            // form, and this one is `no` in §5.7's own table.
            (BoundRow::Page, Attr::Namespace) => false,
            _ => false,
        },
        Leaf::Rel { rel, quant, pred } => {
            // `none` and `every` are `NOT IN` complements: they never bound the
            // OUTER anchor, which needs another positive conjunct.
            if !matches!(quant, Quant::Any) && !matches!((row, rel), (_, Rel::Props)) {
                return false;
            }
            match (row, rel) {
                (BoundRow::Block, Rel::Refs) => pred.ref_name().is_some(),
                (BoundRow::Block, Rel::Tags) | (BoundRow::Page, Rel::Tags) => {
                    pred.ref_name().is_some()
                }
                // The key equality drives the probe, and any atom operator
                // filters inside that key's rows. Property `Every` carries the
                // presence `IN`, which is itself positive.
                (_, Rel::Props) => {
                    pred.props_key().is_some() && matches!(quant, Quant::Any | Quant::Every)
                }
                // A child predicate bounds the ANCHOR only when it is a
                // property of the CHILD alone: the subquery then drives
                // `blocks.parent_block_id` and the anchor is reached by key. A
                // nested `refs` is not such a property — it reads the anchor's
                // OWN ancestor context (see [`Compiler::refs`]), so its two
                // context arms correlate the subquery with the anchor and no
                // index can drive it. §5.7's table entry was written before
                // that family lowered; this is what it says now.
                (BoundRow::Block, Rel::Children) => {
                    !reads_anchor_context(pred) && bounded(pred, false, BoundRow::Block, inputs)
                }
                (BoundRow::Block, Rel::Page) => bounded(pred, false, BoundRow::Page, inputs),
                _ => false,
            }
        }
    }
}

/// Does this predicate, evaluated on a NESTED row, read the anchor's evaluation
/// context rather than the nested row alone?
///
/// Only `refs` does: [`Compiler::refs`]'s nested spelling probes the ANCHOR's
/// parent closure and the ANCHOR's page, at every depth, because that is what
/// `eval_block_leaf` passes down unchanged. Every other relation and attribute
/// is a fact about the row it is applied to. The match is exhaustive on `Rel`
/// so a future context-reading relation has to answer this question before the
/// crate compiles.
fn reads_anchor_context(filter: &Filter) -> bool {
    match filter {
        Filter::And { items } | Filter::Or { items } => items.iter().any(reads_anchor_context),
        Filter::Not { inner } | Filter::Off { inner } => reads_anchor_context(inner),
        Filter::True | Filter::False | Filter::Raw { .. } => false,
        Filter::Leaf { leaf } => match leaf {
            Leaf::Attr { .. } => false,
            Leaf::Rel { rel, pred, .. } => match rel {
                Rel::Refs => true,
                // A deeper `children` is still nested under the SAME anchor, so
                // a `refs` below it reads the same context.
                Rel::Children => reads_anchor_context(pred),
                Rel::Tags | Rel::Props | Rel::Blocks | Rel::Page => false,
            },
        },
    }
}

// ---------------------------------------------------------------------------
// Literal helpers
// ---------------------------------------------------------------------------

/// `AND` over already-lowered operands, folding the two constants.
///
/// **Folding is not an optimization here, it is a correctness-of-plan rule.** A
/// leaf whose operator does not apply to its column's effective type is FALSE
/// (SPEC 3.4), and handing SQLite `... AND 0` inside a subquery makes it plan a
/// covering scan for a predicate that provably selects nothing -- measured on
/// the anonymized corpus for `prop('score') > 5`, where `score` is a text key
/// and the numeric comparison is therefore unsatisfiable.
fn fold_and(parts: Vec<String>) -> String {
    if parts.iter().any(|part| part == "0") {
        return "0".to_string();
    }
    let kept: Vec<String> = parts.into_iter().filter(|part| part != "1").collect();
    match kept.len() {
        0 => "1".to_string(),
        1 => kept.into_iter().next().expect("one operand"),
        _ => format!("({})", kept.join(" AND ")),
    }
}

/// `OR` over already-lowered operands, folding the two constants.
fn fold_or(parts: Vec<String>) -> String {
    if parts.iter().any(|part| part == "1") {
        return "1".to_string();
    }
    let kept: Vec<String> = parts.into_iter().filter(|part| part != "0").collect();
    match kept.len() {
        0 => "0".to_string(),
        1 => kept.into_iter().next().expect("one operand"),
        _ => format!("({})", kept.join(" OR ")),
    }
}

/// `NOT` over an already-lowered operand, folding the two constants.
fn fold_not(inner: String) -> String {
    match inner.as_str() {
        "0" => "1".to_string(),
        "1" => "0".to_string(),
        _ => format!("(NOT {inner})"),
    }
}

/// Renumber the `?N` placeholders the statement actually kept, and drop the
/// values only a folded-away fragment referenced.
///
/// Parameters are POSITIONAL (I-22), so a fragment that bound a value and was
/// then folded away would leave an orphan slot and the driver would refuse the
/// whole statement. Every placeholder this compiler emits appears exactly once,
/// and no literal it emits contains a `?`, so one left-to-right pass is exact.
fn compact_parameters(
    sql: &str,
    params: &[PhysicalQueryValue],
) -> (String, Vec<PhysicalQueryValue>) {
    let mut out = String::with_capacity(sql.len());
    let mut kept: Vec<PhysicalQueryValue> = Vec::new();
    let bytes = sql.as_bytes();
    let mut at = 0usize;
    while at < bytes.len() {
        if bytes[at] != b'?' {
            let ch = sql[at..].chars().next().expect("char boundary");
            out.push(ch);
            at += ch.len_utf8();
            continue;
        }
        let mut end = at + 1;
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            end += 1;
        }
        if end == at + 1 {
            out.push('?');
            at += 1;
            continue;
        }
        let index: usize = sql[at + 1..end].parse().expect("placeholder digits");
        kept.push(params[index - 1].clone());
        out.push_str(&format!("?{}", kept.len()));
        at = end;
    }
    (out, kept)
}

/// Escape `%`, `_` and the escape character itself for a `LIKE … ESCAPE '\'`
/// pattern that must match literally.
fn like_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        if matches!(ch, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// The exclusive upper bound of the half-open range of strings starting with
/// `prefix`, under SQLite's BINARY collation (UTF-8 byte order is code-point
/// order, so incrementing the last scalar value is exactly right).
fn prefix_upper_bound(prefix: &str) -> Option<String> {
    let last = prefix.chars().next_back()?;
    let head: String = prefix[..prefix.len() - last.len_utf8()].to_string();
    let mut next = u32::from(last).checked_add(1)?;
    loop {
        if let Some(ch) = char::from_u32(next) {
            return Some(format!("{head}{ch}"));
        }
        next = next.checked_add(1)?;
    }
}

/// The page-identity fold applied to a PREFIX rather than to a whole name —
/// `eval_page_name`'s `page_prefix_key`, which keeps the trailing boundary slash
/// a namespace prefix carries its whole meaning in.
fn page_prefix_key(text: &str) -> String {
    match text.strip_suffix('/') {
        Some(head) => format!("{}/", refs::page_key(head)),
        None => refs::page_key(text),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::ir::{Source, ViewSettings};
    use crate::query::registry::Registry;

    /// No content leaf: the empty shared parse is what the walk would build for
    /// a filter carrying none, so the two engines still read the same map.
    static NO_COMPILED_LEAVES: std::sync::LazyLock<CompiledLeaves> =
        std::sync::LazyLock::new(|| CompiledLeaves::for_query(&Filter::True));

    fn inputs<'a>(registry: &'a Registry) -> LoweringInputs<'a> {
        LoweringInputs {
            today: JournalDate::from_ordinal(20260905),
            registry,
            masked_pages: &[],
            cutoff: None,
            compiled: &NO_COMPILED_LEAVES,
            fts_ready: true,
            result_set_rule: RESULT_SET_RULE,
        }
    }

    /// The lowering of one filter, with the shared Match parse the walk would
    /// build for it — never a second parse (I-12).
    fn lower_with(filter: Filter, anchor: Anchor, fts_ready: bool) -> SqlQuery {
        let registry = Registry::none().clone();
        let compiled = CompiledLeaves::for_query(&filter);
        let query = Query::new(anchor, filter, Source::Builder);
        let inputs = LoweringInputs {
            compiled: &compiled,
            fts_ready,
            ..inputs(&registry)
        };
        lower_query(&query, &inputs)
    }

    fn lower(filter: Filter, anchor: Anchor) -> SqlQuery {
        lower_with(filter, anchor, true)
    }

    fn og(source: &str) -> (Query, ViewSettings) {
        crate::query::parse_query_source(source, JournalDate::from_ordinal(20260905))
    }

    fn tql(source: &str) -> (Query, ViewSettings) {
        crate::query::parse_query_text(
            source,
            crate::query::QueryDialect::Tql,
            JournalDate::from_ordinal(20260905),
        )
    }

    /// §5.1's first rule and the false positive it exists to prevent: two
    /// property conjuncts are TWO subqueries, each carrying its own key AND its
    /// own atom test — never four independent probes whose results are joined
    /// after the fact.
    #[test]
    fn two_property_conjuncts_lower_to_two_undecomposed_subqueries() {
        let (query, _) = og("(and (property status open) (property priority done))");
        let statement = lower(query.evaluable_filter(), Anchor::Block);
        // Two conjuncts, each ONE subquery. §5.3's result-set rule reads the
        // match set back by name under [`ResultSetRule::MatchSetCteMaterialized`],
        // so the filter is compiled ONCE — the correlated spelling compiled the
        // same tree a second time in the parent's row scope and this count was 4.
        assert_eq!(
            statement.sql.matches("FROM property_atoms").count(),
            2,
            "one atom subquery per quantifier, per row scope: {}",
            statement.sql
        );
        // Neither subquery is decomposed: each carries its key AND its atom test.
        for fragment in statement.sql.split("FROM property_atoms").skip(1) {
            let head = fragment.split(')').next().unwrap_or_default();
            assert!(
                head.contains(".normalized_name = ") && head.contains(".atom_key = "),
                "a property subquery carries key and value together: {head}"
            );
        }
        // Each subquery carries its key and its value in ONE `WHERE`.
        for key in ["status", "priority"] {
            let key_at = statement
                .params
                .iter()
                .position(|value| *value == PhysicalQueryValue::Text(key.into()))
                .unwrap_or_else(|| panic!("{key} is bound: {:?}", statement.params));
            let _ = key_at;
        }
        for value in ["open", "done"] {
            assert!(
                statement
                    .params
                    .contains(&PhysicalQueryValue::Text(value.into())),
                "{value} is bound: {:?}",
                statement.params
            );
        }
    }

    /// §5.1's J1: a subquery over a nullable owner column carries `IS NOT NULL`,
    /// because `x NOT IN (SELECT c)` is NULL — not false — when any `c` is NULL.
    #[test]
    fn a_children_subquery_never_selects_a_null_owner() {
        let filter = Filter::rel(
            Rel::Children,
            Quant::None,
            Filter::attr(Attr::Task, CmpOp::Eq, Value::text("TODO")),
        );
        let statement = lower(filter, Anchor::Block);
        assert!(
            statement.sql.contains("c1.parent_block_id IS NOT NULL"),
            "{}",
            statement.sql
        );
        assert!(
            statement.sql.contains("NOT IN (SELECT"),
            "{}",
            statement.sql
        );
    }

    /// §5.1: a generic `Every` is the complement of its VIOLATION predicate, so
    /// an empty relation is true (K2).
    #[test]
    fn a_generic_every_is_the_complement_of_its_violation_predicate() {
        let filter = Filter::rel(
            Rel::Children,
            Quant::Every,
            Filter::attr(Attr::Task, CmpOp::Eq, Value::text("DONE")),
        );
        let statement = lower(filter, Anchor::Block);
        assert!(
            statement.sql.contains("b.block_id NOT IN (SELECT"),
            "{}",
            statement.sql
        );
        assert!(statement.sql.contains("(NOT "), "{}", statement.sql);
    }

    /// §3.3: a property `Every` is presence AND no violator — a generic `Every`
    /// alone would answer true for an owner that never spells the key.
    #[test]
    fn a_property_every_carries_its_presence_conjunct() {
        let filter = Filter::rel(
            Rel::Props,
            Quant::Every,
            Filter::and(vec![
                Filter::attr(Attr::Key, CmpOp::Eq, Value::text("type")),
                Filter::attr(Attr::Value, CmpOp::Eq, Value::text("book")),
            ]),
        );
        let statement = lower(filter, Anchor::Block);
        assert!(
            statement.sql.contains("FROM properties"),
            "presence probe: {}",
            statement.sql
        );
        assert!(
            statement.sql.contains("FROM property_atoms"),
            "violation probe: {}",
            statement.sql
        );
    }

    /// §5.2: every comparison on a nullable column carries its null guard, so
    /// the expression is two-valued and `NOT` over it is classical. The guard is
    /// written `IS NOT NULL AND` rather than `COALESCE(…, 0)` because only the
    /// former lets SQLite seek the index §5.7 requires — same function, one
    /// spelling that satisfies both rules.
    #[test]
    fn comparisons_on_nullable_columns_carry_a_two_valued_null_guard() {
        let filter = Filter::attr(Attr::Scheduled, CmpOp::Le, Value::date("today"));
        let statement = lower(filter, Anchor::Block);
        assert!(
            statement
                .sql
                .contains("(bp1.scheduled_day IS NOT NULL AND bp1.scheduled_day <= ?1)"),
            "{}",
            statement.sql
        );
        assert!(!statement.sql.contains("COALESCE"), "{}", statement.sql);
    }

    /// §5.5: every literal is a bound parameter. The statement text may not
    /// contain a value the caller supplied.
    #[test]
    fn values_are_bound_and_never_interpolated() {
        let (query, _) = og("(and (property type \"O'Brien; DROP TABLE blocks--\") [[Some Page]])");
        let statement = lower(query.evaluable_filter(), Anchor::Block);
        assert!(
            !statement.sql.contains("O'Brien"),
            "the hostile literal is bound, not spelled: {}",
            statement.sql
        );
        assert!(
            statement
                .params
                .iter()
                .any(|value| matches!(value, PhysicalQueryValue::Text(text) if text.contains("drop table"))),
            "{:?}",
            statement.params
        );
    }

    /// §3.4 + §5.7: a leaf whose operator does not apply to its key's effective
    /// type is FALSE, and a false conjunct folds the STATEMENT rather than being
    /// handed to SQLite as `… AND 0`.
    ///
    /// This is not tidiness. Measured on the anonymized corpus, `prop('score') >
    /// 5` where `score` is a text key produced `… AND 0` inside the atom
    /// subquery, and SQLite planned it as `SCAN a2 USING COVERING INDEX
    /// property_atoms_page_idx` — a full covering scan for a predicate that
    /// selects nothing, which is §5.7's own failure mode. There is no index that
    /// fixes it; the subquery has to not be asked.
    ///
    /// The second half is the trap folding sets: parameters are POSITIONAL, so a
    /// fragment that bound a value before folding away would leave a hole and
    /// the driver would refuse the statement.
    #[test]
    fn an_unsatisfiable_leaf_folds_the_statement_and_takes_its_parameters_with_it() {
        // `Registry::none()` gives every key the default effective type Text, so
        // `> 5` is the operator/type mismatch §3.4 answers false for.
        let registry = Registry::none().clone();
        let (query, _) = tql("prop('score') > 5");
        let statement = lower_query(&query, &inputs(&registry));
        assert!(
            statement.matches_nothing,
            "an unsatisfiable comparison makes the whole statement empty: {}",
            statement.sql
        );
        assert!(
            statement.sql.ends_with(" WHERE 0"),
            "no subquery is left to plan: {}",
            statement.sql
        );
        assert!(
            !statement.positively_bounded,
            "a statement that reads no row asks §5.7 for no index"
        );
        assert!(
            statement.params.is_empty(),
            "the folded-away fragments took their values with them: {:?}",
            statement.params
        );

        // The same fold under `None`, where the presence probe HAS bound its key
        // and owner type before the atom test folds: the quantifier becomes the
        // constant true and every one of those placeholders leaves with it.
        let (query, _) = tql("none(prop('k'), value > 5)");
        let statement = lower_query(&query, &inputs(&registry));
        assert!(
            !statement.sql.contains('?'),
            "no orphan placeholder survives: {}",
            statement.sql
        );
        assert!(statement.params.is_empty(), "{:?}", statement.params);
        assert!(
            !statement.matches_nothing,
            "`none` over an unsatisfiable test is true, not false: {}",
            statement.sql
        );

        // And the renumbering itself: what is left is `?1..?n` with no gap, in
        // order of appearance, matching the parameter list one for one.
        let (query, _) = og("(and (property status open) [[Some Page]])");
        let statement = lower(query.evaluable_filter(), Anchor::Block);
        let mut seen: Vec<usize> = Vec::new();
        let mut rest = statement.sql.as_str();
        while let Some(at) = rest.find('?') {
            rest = &rest[at + 1..];
            let end = rest
                .find(|c: char| !c.is_ascii_digit())
                .unwrap_or(rest.len());
            seen.push(rest[..end].parse().expect("digits"));
            rest = &rest[end..];
        }
        assert_eq!(
            seen,
            (1..=statement.params.len()).collect::<Vec<_>>(),
            "placeholders are dense, ordered, and exactly as many as the values: {}",
            statement.sql
        );
    }

    /// §5.6: `LIMIT cutoff + 1`, so the caller can tell "exactly the cutoff"
    /// from "more than the cutoff".
    #[test]
    fn a_cutoff_lowers_to_limit_cutoff_plus_one() {
        let registry = Registry::none().clone();
        let query = Query::new(Anchor::Block, Filter::True, Source::Builder);
        let mut inputs = inputs(&registry);
        inputs.cutoff = Some(50);
        let statement = lower_query(&query, &inputs);
        assert!(statement.sql.contains("LIMIT ?1"), "{}", statement.sql);
        assert_eq!(statement.params, vec![PhysicalQueryValue::Integer(51)]);
    }

    /// §5.7: the boundedness table is exhaustive, and negation never bounds.
    #[test]
    fn boundedness_follows_the_exhaustive_table() {
        let registry = Registry::none().clone();
        let inputs = inputs(&registry);
        let bounded_shapes = [
            "[[Project]]",
            "(task TODO)",
            "(priority A)",
            "(property type book)",
            "(and (task TODO) \"loose text\")",
        ];
        for source in bounded_shapes {
            let (query, _) = og(source);
            assert!(
                positively_bounded(&query.evaluable_filter(), Anchor::Block, &inputs),
                "{source} must be positively bounded"
            );
        }
        let unbounded_shapes = [
            "\"loose text\"",
            "(not (task TODO))",
            "(or (task TODO) \"loose text\")",
        ];
        for source in unbounded_shapes {
            let (query, _) = og(source);
            assert!(
                !positively_bounded(&query.evaluable_filter(), Anchor::Block, &inputs),
                "{source} must NOT be positively bounded"
            );
        }

        // A child predicate that is a property of the CHILD bounds the anchor;
        // one that reads the ANCHOR's ancestor context does not, at any depth.
        // The plan that forces this is measured in
        // `a_nested_refs_child_predicate_cannot_bound_its_anchor`.
        let child_task = Filter::rel(
            Rel::Children,
            Quant::Any,
            Filter::attr(Attr::Task, CmpOp::Eq, Value::text("DONE")),
        );
        assert!(positively_bounded(&child_task, Anchor::Block, &inputs));
        let child_refs = Filter::rel(Rel::Children, Quant::Any, Filter::page_ref("project"));
        assert!(!positively_bounded(&child_refs, Anchor::Block, &inputs));
        let deep_refs = Filter::rel(
            Rel::Children,
            Quant::Any,
            Filter::and(vec![
                Filter::attr(Attr::Task, CmpOp::Eq, Value::text("DONE")),
                Filter::rel(Rel::Children, Quant::Any, Filter::page_ref("project")),
            ]),
        );
        assert!(!positively_bounded(&deep_refs, Anchor::Block, &inputs));
        // At the ANCHOR the same leaf is still one keyed probe and still bounds.
        assert!(positively_bounded(
            &Filter::page_ref("project"),
            Anchor::Block,
            &inputs
        ));
    }

    /// §5.7: absence lowers to a complement and enumerates it, so `is null` and
    /// a negated leaf bound nothing even though their positive twins do.
    #[test]
    fn absence_and_negation_bound_nothing() {
        let registry = Registry::none().clone();
        let inputs = inputs(&registry);
        let absent = Filter::attr(Attr::Scheduled, CmpOp::IsNotSet, Value::None);
        assert!(!positively_bounded(&absent, Anchor::Block, &inputs));
        let present = Filter::attr(Attr::Scheduled, CmpOp::IsSet, Value::None);
        assert!(positively_bounded(&present, Anchor::Block, &inputs));
        assert!(!positively_bounded(
            &Filter::not(present),
            Anchor::Block,
            &inputs
        ));
    }

    // -----------------------------------------------------------------------
    // §5.10
    // -----------------------------------------------------------------------

    fn content_match(text: &str) -> Filter {
        Filter::attr(Attr::Content, CmpOp::Match, Value::text(text))
    }

    fn match_sql(text: &str, fts_ready: bool) -> SqlQuery {
        lower_with(content_match(text), Anchor::Block, fts_ready)
    }

    /// The one rule that decides this packet: the exact `instr` predicates are
    /// the final SQL conditions on EVERY path, so the trigram probe only
    /// narrows which rows are asked. A bound that replaced them would answer
    /// "different results, faster".
    #[test]
    fn the_candidate_bound_narrows_and_never_replaces_the_exact_predicate() {
        let bounded = match_sql("alpha", true);
        assert!(
            bounded.sql.contains("search_substring_fts")
                && bounded.sql.contains("MATCH ?")
                && bounded.sql.contains("instr(b.query_visible_folded, ?"),
            "{}",
            bounded.sql
        );
        assert!(bounded.positively_bounded);
        assert_eq!(bounded.content_plans, vec![ContentPlan::Fts]);
        // The needle is the FTS phrase literal, and the exact needle is the
        // parser's own folded term — two different bound values.
        assert!(bounded
            .params
            .contains(&PhysicalQueryValue::Text("\"alpha\"".to_string())));
        assert!(bounded
            .params
            .contains(&PhysicalQueryValue::Text("alpha".to_string())));
    }

    /// §5.10's acceptance corollary, at the compiler: `foobar` is reachable by
    /// `foo` and `oob` THROUGH the index and by `oo` WITHOUT it — the two-scalar
    /// term yields no trigram, and answering it is not optional.
    #[test]
    fn a_two_scalar_term_is_unbounded_rather_than_unanswered() {
        for needle in ["foo", "oob"] {
            let statement = match_sql(needle, true);
            assert!(statement.sql.contains("search_substring_fts"), "{needle}");
            assert_eq!(statement.content_plans, vec![ContentPlan::Fts]);
        }
        let short = match_sql("oo", true);
        assert!(
            !short.sql.contains("search_substring_fts")
                && short.sql.contains("instr(b.query_visible_folded, ?"),
            "{}",
            short.sql
        );
        assert!(!short.positively_bounded);
        assert_eq!(short.content_plans, vec![ContentPlan::ShortUnindexable]);
    }

    /// One unbounded OR arm makes the LEAF unbounded, and that is not a defect
    /// to work around: the anchor is reached once per arm.
    #[test]
    fn one_unbounded_or_arm_unbounds_the_whole_leaf() {
        let mixed = match_sql("oo OR alpha", true);
        assert!(!mixed.positively_bounded);
        assert_eq!(mixed.content_plans, vec![ContentPlan::ShortUnindexable]);
        // The bounded arm still gets its bound — bounds are per-arm. One
        // occurrence, because §5.3's CTE spelling compiles the filter once.
        assert_eq!(mixed.sql.matches("search_substring_fts").count(), 1);
        assert!(match_sql("beta OR alpha", true).positively_bounded);
    }

    /// §5.10: emptiness comes from the parsed `Term`, not from SQLite.
    /// `instr(text, '')` is 1 and `length` stops at NUL, so only the Rust side
    /// can reproduce `group_matches`' `!text.is_empty() && contains`.
    #[test]
    fn an_empty_term_is_false_and_an_empty_negative_term_is_true() {
        // A whitespace-only quoted phrase is NOT empty: it is a real needle the
        // exact column can hold and the collapsed FTS text cannot.
        let spaces = match_sql("\"   \"", true);
        assert!(!spaces.matches_nothing && !spaces.sql.contains("search_substring_fts"));
        assert!(spaces
            .params
            .contains(&PhysicalQueryValue::Text("   ".to_string())));
        assert_eq!(spaces.content_plans, vec![ContentPlan::ShortUnindexable]);

        // The empty term itself, against `group_matches`' own answer. It is
        // constructed rather than parsed because `Matcher::parse` drops an
        // empty TOKEN — but the rule has to hold for the parsed value it does
        // produce, whatever a future fold makes empty, and SQLite's `instr`
        // answers the opposite of it.
        let registry = Registry::none().clone();
        let inputs = inputs(&registry);
        let mut compiler = Compiler {
            inputs: &inputs,
            params: Vec::new(),
            next_alias: 0,
            regexes: Vec::new(),
        };
        for negated in [false, true] {
            let term = Term {
                text: String::new(),
                negated,
                quoted: false,
            };
            let group = vec![term.clone()];
            assert_eq!(
                compiler.match_term(&term, "b"),
                if negated { "1" } else { "0" },
                "an empty {}term",
                if negated { "negative " } else { "" }
            );
            // And the walk agrees, on text that contains everything and nothing.
            for text in ["", "anything at all"] {
                let matcher = Matcher::Boolean(vec![group.clone()]);
                assert_eq!(
                    matcher.matches(text, text),
                    negated,
                    "the walk's answer for an empty term over {text:?}"
                );
            }
        }
        assert!(compiler.params.is_empty(), "a constant binds nothing");
    }

    /// Exclusion-only input and an invalid regex are FALSE LEAVES (§5.10), so
    /// `not` over them is classically true — the existing truth rule, not a
    /// whole-query diagnostic.
    #[test]
    fn exclusion_only_and_invalid_regex_lower_to_a_false_leaf_under_not_too() {
        for (source, plans) in [
            ("-alpha", &[][..]),
            ("   ", &[][..]),
            // An invalid `/pattern/` is still a REGEX leaf for §5.10's plan
            // classes — it just needs no engine to answer false.
            ("/[unclosed/", &[ContentPlan::Regex][..]),
        ] {
            let statement = match_sql(source, true);
            assert!(statement.matches_nothing, "{source}: {}", statement.sql);
            assert_eq!(statement.content_plans, plans, "{source}");
            let negated = lower_with(Filter::not(content_match(source)), Anchor::Block, true);
            assert!(!negated.matches_nothing, "{source}: {}", negated.sql);
            assert!(!negated.positively_bounded, "{source}");
        }
    }

    /// The `fts-building` class (§5.10): the SAME exact predicates on the ready
    /// block columns, with no bound anywhere — never empty results, an error, a
    /// new walk route, or a rebuild request (I-13).
    #[test]
    fn a_building_index_omits_the_bounds_and_keeps_the_exact_predicates() {
        let building = match_sql("alpha beta OR gamma", false);
        assert!(
            !building.sql.contains("search_substring_fts")
                && !building.sql.contains("search_fts_owners")
                && !building.sql.contains("MATCH"),
            "{}",
            building.sql
        );
        // Three terms, compiled once (§5.3's CTE spelling).
        assert_eq!(building.sql.matches("instr(").count(), 3);
        assert!(!building.positively_bounded);
        assert_eq!(building.content_plans, vec![ContentPlan::FtsBuilding]);
        // Same predicates, same bound needles, as the ready lowering: only the
        // candidate bound differs.
        let ready = match_sql("alpha beta OR gamma", true);
        for term in ["alpha", "beta", "gamma"] {
            let value = PhysicalQueryValue::Text(term.to_string());
            assert!(building.params.contains(&value) && ready.params.contains(&value));
        }
    }

    /// A negative term never bounds anything: it says the text does NOT contain
    /// it, so using it as a candidate would select exactly the rejected rows.
    #[test]
    fn a_negative_term_is_negated_and_never_becomes_the_candidate() {
        let statement = match_sql("oo -draft", true);
        assert!(
            statement
                .sql
                .contains("(NOT (instr(b.query_visible_folded, ?"),
            "{}",
            statement.sql
        );
        assert!(
            !statement.sql.contains("search_substring_fts"),
            "the only three-scalar run is the NEGATIVE term: {}",
            statement.sql
        );
        assert!(!statement
            .params
            .contains(&PhysicalQueryValue::Text("\"draft\"".to_string())));
    }

    /// §5.10's needle rule, at the unit that owns it.
    #[test]
    fn the_candidate_needle_is_the_first_three_scalar_whitespace_free_run() {
        let needle = |source: &str| {
            let Matcher::Boolean(groups) = Matcher::parse(source) else {
                panic!("{source} is not a boolean query");
            };
            fts_candidate_needle(&groups[0]).map(str::to_owned)
        };
        // Not the whole phrase: the run survives the producers' whitespace
        // collapsing, the leading/repeated spaces need not.
        assert_eq!(needle("\"  alpha  beta\""), Some("alpha".to_string()));
        assert_eq!(needle("\"a b cde\""), Some("cde".to_string()));
        // Terms are scanned in order, and a term with no long-enough run is
        // skipped rather than ending the scan.
        assert_eq!(needle("ab cd efgh"), Some("efgh".to_string()));
        assert_eq!(needle("ab cd"), None);
        assert_eq!(needle("-longenough ab"), None);
        // Three SCALARS, not three bytes.
        assert_eq!(needle("日本語"), Some("日本語".to_string()));
        assert_eq!(needle("日本"), None);
        // A NUL-bearing term supplies no bound at all.
        let nul = vec![Term {
            text: "abc\0def".to_string(),
            negated: false,
            quoted: false,
        }];
        assert_eq!(fts_candidate_needle(&nul), None);
    }

    /// The needle crosses the boundary as ONE FTS5 phrase literal, so `-`,
    /// `*`, `(` and a bare `OR` inside it are text and not query syntax.
    #[test]
    fn the_fts_needle_is_quoted_as_one_literal_with_doubled_quotes() {
        assert_eq!(fts_phrase_literal("say\"hi"), "\"say\"\"hi\"");
        assert_eq!(fts_phrase_literal("a OR b"), "\"a OR b\"");
        assert_eq!(fts_phrase_literal("-x*"), "\"-x*\"");
    }

    /// §4.3.2's regex predicate, at the compiler. A VALID pattern in EITHER
    /// spelling reaches a statement as `tine_query_regex(<bound id>, <exact
    /// visible text>)`; an INVALID one still needs no engine and is a false
    /// leaf. The pattern itself never appears in the SQL.
    #[test]
    fn a_valid_regex_lowers_to_the_bound_predicate_over_the_exact_visible_text() {
        for source in ["content regexp '[a-z]+'", "content match '/[a-z]+/'"] {
            let (query, _) = tql(source);
            let statement = lower_with(query.evaluable_filter(), Anchor::Block, true);
            assert!(
                statement
                    .sql
                    .contains("b.block_id IN (SELECT bt1.block_id FROM block_text bt1 \
                     WHERE (bt1.block_id = b.block_id AND tine_query_regex(?1, bt1.query_visible)))"),
                "{source}: {}",
                statement.sql
            );
            // The EXACT column, never the folded one a case-insensitive
            // comparison would use.
            assert!(
                !statement.sql.contains("query_visible_folded"),
                "{source}: {}",
                statement.sql
            );
            // The pattern is a table row keyed by a bound ID, not SQL text.
            assert!(
                !statement.sql.contains("[a-z]"),
                "{source}: {}",
                statement.sql
            );
            assert_eq!(statement.params, vec![PhysicalQueryValue::Integer(1)]);
            assert_eq!(statement.regexes.bindings.len(), 1, "{source}");
            assert_eq!(
                statement.content_plans,
                vec![ContentPlan::Regex],
                "{source}"
            );
            // Explicitly unindexed (§4.3.2): a regex never bounds the anchor.
            assert!(!statement.positively_bounded, "{source}");
            assert!(!statement.matches_nothing, "{source}");
        }
        let invalid = lower_with(
            Filter::attr(Attr::Content, CmpOp::Regex, Value::text("[unclosed")),
            Anchor::Block,
            true,
        );
        assert!(invalid.matches_nothing);
        assert_eq!(invalid.content_plans, vec![ContentPlan::Regex]);
        assert!(invalid.regexes.is_empty(), "a false leaf binds no program");
    }

    /// The compiled-regex table is de-duplicated by PATTERN SOURCE, so the same
    /// pattern written twice is one ID — and, decisively, §5.3's correlated
    /// spelling compiling the whole filter a SECOND time in the parent's row
    /// scope reuses the first pass's IDs instead of growing a parallel table
    /// whose second half nothing would install.
    #[test]
    fn one_pattern_is_one_binding_however_many_times_it_is_compiled() {
        let registry = Registry::none().clone();
        let filter = Filter::and(vec![
            Filter::attr(Attr::Content, CmpOp::Regex, Value::text("alpha")),
            Filter::attr(Attr::Content, CmpOp::Regex, Value::text("beta")),
            Filter::attr(Attr::Content, CmpOp::Regex, Value::text("alpha")),
        ]);
        let compiled = CompiledLeaves::for_query(&filter);
        let query = Query::new(Anchor::Block, filter, Source::Builder);
        for rule in [
            ResultSetRule::MatchSetCteMaterialized,
            ResultSetRule::CorrelatedProbe,
        ] {
            let inputs = LoweringInputs {
                compiled: &compiled,
                result_set_rule: rule,
                ..inputs(&registry)
            };
            let statement = lower_query(&query, &inputs);
            assert_eq!(
                statement.regexes.bindings.len(),
                2,
                "{rule:?}: two distinct patterns: {}",
                statement.sql
            );
            // Every ID the statement names is one the program binds.
            let predicate = statement.regexes.predicate();
            for id in 1..=2u64 {
                assert!(predicate(id, "alpha beta").is_ok(), "{rule:?} id {id}");
            }
            assert!(
                predicate(3, "alpha").is_err(),
                "{rule:?}: an unbound id fails"
            );
        }
    }

    /// The program is the WALK's compiled value, and its equality is the
    /// patterns it binds — not a pointer, which would make two identical
    /// lowerings compare unequal, and not a `Debug` line carrying user text.
    #[test]
    fn the_regex_program_compares_by_pattern_and_never_prints_one() {
        let (query, _) = tql("content regexp 'secret-\\d+'");
        let filter = query.evaluable_filter();
        let first = lower_with(filter.clone(), Anchor::Block, true);
        let second = lower_with(filter, Anchor::Block, true);
        assert_eq!(first, second, "two lowerings of one filter are equal");
        assert_eq!(
            format!("{:?}", first.regexes),
            "QueryRegexProgram { bindings: 1 }"
        );
        assert!(
            !format!("{first:?}").contains("secret-"),
            "the pattern text never reaches a Debug line"
        );
        // And the program answers with the SAME program the walk runs.
        let predicate = first.regexes.predicate();
        assert_eq!(predicate(1, "secret-42").unwrap(), true);
        assert_eq!(predicate(1, "secret-").unwrap(), false);
    }

    /// §3.2's nested-`refs` context, at the compiler: the set a nested row is
    /// tested against is its OWN refs, the ANCHOR's ancestors' and the page —
    /// three unioned stored facts, never `block_path_refs(<nested row>)` and
    /// never a subtraction.
    #[test]
    fn refs_inside_a_children_predicate_reads_the_anchors_ancestor_context() {
        let registry = Registry::none().clone();
        let query = Query::new(
            Anchor::Block,
            Filter::rel(Rel::Children, Quant::Any, Filter::page_ref("Project")),
            Source::Builder,
        );
        let statement = lower_query(&query, &inputs(&registry));
        // The nested row contributes ONLY its own refs.
        assert!(
            statement.sql.contains("block_own_refs or2")
                && statement.sql.contains("or2.block_id = c1.block_id"),
            "{}",
            statement.sql
        );
        // The ancestor context is the ANCHOR's parent's closure, and it is the
        // anchor `b` that is named there — never the child `c1`.
        assert!(
            statement.sql.contains(
                "(b.parent_block_id IS NOT NULL AND b.parent_block_id IN \
                 (SELECT ar3.block_id FROM block_path_refs ar3 \
                 WHERE (ar3.block_id = b.parent_block_id AND ar3.normalized_name = ?2)))"
            ),
            "{}",
            statement.sql
        );
        // The page is named separately, because a ROOT anchor has no parent row
        // to carry it.
        assert!(
            statement.sql.contains(
                "b.page_id IN (SELECT pr4.page_id FROM pages pr4 \
             WHERE (pr4.page_id = b.page_id AND pr4.name_key <> '' AND pr4.name_key = ?3))"
            ),
            "{}",
            statement.sql
        );
        // Nothing reads the nested row's own materialized closure.
        assert!(
            !statement
                .sql
                .contains("block_path_refs ar3 WHERE (ar3.block_id = c1"),
            "{}",
            statement.sql
        );
        // One bound value per arm, and the SAME page-identity fold on all three
        // — never a literal spelled into the statement.
        assert_eq!(
            statement.params,
            vec![PhysicalQueryValue::Text("project".to_string()); 3]
        );

        // Two levels down the context is STILL the anchor's: `c2` is the
        // grandchild, and the ancestor and page arms both name `b`.
        let deep = Query::new(
            Anchor::Block,
            Filter::rel(
                Rel::Children,
                Quant::Any,
                Filter::rel(Rel::Children, Quant::Any, Filter::page_ref("Project")),
            ),
            Source::Builder,
        );
        let statement = lower_query(&deep, &inputs(&registry));
        assert!(
            statement.sql.contains("or3.block_id = c2.block_id")
                && statement.sql.contains("ar4.block_id = b.parent_block_id")
                && statement.sql.contains("pr5.page_id = b.page_id"),
            "the grandchild's context is the anchor's, not its parent's: {}",
            statement.sql
        );

        // The anchor's OWN `refs` leaf is unchanged: one probe of the one table
        // §5.8 materializes for exactly this question.
        let top = Query::new(Anchor::Block, Filter::page_ref("Project"), Source::Builder);
        let statement = lower_query(&top, &inputs(&registry));
        assert!(
            statement.sql.contains("FROM block_path_refs r1"),
            "{}",
            statement.sql
        );
        assert!(
            !statement.sql.contains("block_own_refs"),
            "the anchor needs no union: {}",
            statement.sql
        );
    }

    /// The three quantifiers of a nested `refs` leaf, which is where an
    /// existence built from a UNION could quietly stop matching `quantify`:
    /// `Any` is that existence, `None` its negation, and a general `Every` is
    /// "no element violates" — while the single-name `Every` is membership, the
    /// walk's own `single_ref_name` fast path.
    #[test]
    fn a_nested_refs_quantifier_is_the_walks_own_three_answers() {
        let registry = Registry::none().clone();
        let nested = |quant: Quant, pred: Filter| {
            let query = Query::new(
                Anchor::Block,
                Filter::rel(
                    Rel::Children,
                    Quant::Any,
                    Filter::rel(Rel::Refs, quant, pred),
                ),
                Source::Builder,
            );
            lower_query(&query, &inputs(&registry)).sql
        };
        let name = || Filter::attr(Attr::Name, CmpOp::Eq, Value::text("Project"));
        // Single name: `Every` IS `Any`, so the two lower identically.
        assert_eq!(nested(Quant::Any, name()), nested(Quant::Every, name()));
        // `None` is that same existence, negated.
        assert!(nested(Quant::None, name()).contains("(NOT ("));
        // A general `Every` negates the ELEMENT predicate instead.
        let general = Filter::or(vec![
            name(),
            Filter::attr(Attr::Name, CmpOp::Eq, Value::text("Other")),
        ]);
        let every = nested(Quant::Every, general.clone());
        assert!(
            every.contains("(NOT (or") || every.contains("NOT ("),
            "{every}"
        );
        assert_ne!(every, nested(Quant::Any, general));
    }

    /// §5.9: the overlay-masked page ids leave the statement, so the masked read
    /// and the overlay walk cannot both answer for one page.
    #[test]
    fn masked_pages_leave_the_statement() {
        let registry = Registry::none().clone();
        let masked = [[7u8; 16]];
        let mut inputs = inputs(&registry);
        inputs.masked_pages = &masked;
        let query = Query::new(Anchor::Block, Filter::page_ref("x"), Source::Builder);
        let statement = lower_query(&query, &inputs);
        // The mask sits on the ANCHOR, which under §5.3's CTE spelling is the
        // materialized match set: a block and its parent are always on the same
        // page, so masking inside `m` would be unobservable, and staying outside
        // keeps the masked statement the same predicate it is today.
        assert!(
            statement.sql.contains("m.page_id NOT IN ("),
            "{}",
            statement.sql
        );
        assert!(statement
            .params
            .contains(&PhysicalQueryValue::Blob(vec![7u8; 16])));
    }

    #[test]
    fn a_prefix_range_is_half_open_and_survives_the_last_scalar_value() {
        assert_eq!(prefix_upper_bound("proj/"), Some("proj0".to_string()));
        assert_eq!(prefix_upper_bound("ab"), Some("ac".to_string()));
        // U+D7FF is the last scalar before the surrogate block; the next valid
        // scalar is U+E000, not U+D800.
        assert_eq!(prefix_upper_bound("\u{d7ff}"), Some("\u{e000}".to_string()));
        assert_eq!(prefix_upper_bound(&format!("{}", char::MAX)), None);
    }

    #[test]
    fn like_escape_protects_the_three_pattern_characters() {
        assert_eq!(like_escape("100%_a\\b"), "100\\%\\_a\\\\b");
    }
}
