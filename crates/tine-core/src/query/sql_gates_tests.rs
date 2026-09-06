//! The two gates SPEC §5 makes this wave's acceptance bar, and the harness both
//! of them run on.
//!
//! * **`walk == SQL`, always.** The lowering and the walk answer the same
//!   question over the same graph, so a difference is a failure of this wave and
//!   never a documented divergence (I-19, I-12). The comparison is at the
//!   PRODUCT level — the block ids a query returns, `tree/filter-top-level-blocks`
//!   included — because that is what a user sees.
//! * **The plan gate (§5.7).** A positively bounded query must reach its anchor
//!   table by `SEARCH`, never `SCAN`, and every bounded relation subquery must
//!   show an index. It runs through `tine-storage`'s `explain_query_plan`
//!   accessor, so it is an ordinary repository test rather than a scratch
//!   harness.
//!
//! **Assert the semantics, not the string.** `blocks` is a rowid table with a
//! BLOB primary key, so SQLite spells the anchor probe `SEARCH b USING
//! [COVERING] INDEX sqlite_autoindex_blocks_1 (block_id=?)` and NEVER `SEARCH b
//! USING PRIMARY KEY` — that spelling is for INTEGER-PK and `WITHOUT ROWID`
//! tables. Same access path, different text.
//!
//! The fast corpus below is the permanent one. `~/research/logseq-anonymized` is
//! an acceptance gate rather than an optional extra (AGENTS §4 tier 2), reached
//! through the `#[ignore]`d twins; when the real graph disagrees with the fast
//! corpus that is a CORPUS DEFECT, and the fix is to extract the minimal shape
//! into the fixture below. No corpus content is read into an assertion message,
//! a receipt or any other artifact — only aggregate counts and plan strings.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tine_storage::sqlite::{PhysicalProjectionQueryReader, PhysicalQueryValue};
use uuid::Uuid;

use crate::date::JournalDate;
use crate::model::Graph;
use crate::query::ir::{Anchor, Bounds, QueryRows};
use crate::query::sql::{
    lower_query, ContentPlan, LoweringInputs, QueryRegexProgram, ResultSetRule, SqlQuery,
    RESULT_SET_RULE,
};
use crate::query::QueryDialect;

/// The Direct Files projection worker is a process-wide singleton per graph and
/// the tests below each start one; serialize them as the neighbouring
/// `direct_projection` tests do.
static GATE_LOCK: Mutex<()> = Mutex::new(());

/// A poisoned lock means a NEIGHBOURING gate failed, which must not turn this
/// gate's own result into a second, misleading failure.
fn serialize() -> std::sync::MutexGuard<'static, ()> {
    GATE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn scratch(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("tine-query-sql-{tag}-{}", Uuid::new_v4()))
}

/// A graph plus the ready Direct Files projection built from it by the
/// PRODUCTION producer — never by a re-implementation in the test, which would
/// prove only that the test agrees with itself.
struct Corpus {
    graph: Graph,
    reader: PhysicalProjectionQueryReader,
    root: PathBuf,
    owns_root: bool,
}

impl Drop for Corpus {
    fn drop(&mut self) {
        if self.owns_root {
            let _ = std::fs::remove_dir_all(&self.root);
        } else {
            // A real corpus is the user's directory: only the projection this
            // test wrote beside it, in its own temp dir, is removed.
            let _ = std::fs::remove_dir_all(self.projection_dir());
        }
    }
}

impl Corpus {
    fn projection_dir(&self) -> PathBuf {
        std::env::temp_dir().join(format!(
            "tine-query-sql-projection-{}",
            self.root
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default()
        ))
    }

    fn open(root: PathBuf, owns_root: bool) -> Corpus {
        let graph = Graph::open(&root);
        graph.warm_cache();
        let projection_dir = std::env::temp_dir().join(format!(
            "tine-query-sql-projection-{}",
            root.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default()
        ));
        std::fs::create_dir_all(&projection_dir).expect("projection scratch");
        let path = projection_dir.join("projection.sqlite");
        graph
            .attach_direct_projection(path.clone())
            .expect("the projection worker starts");
        let started = Instant::now();
        while !graph.direct_projection_ready_test() {
            assert!(
                started.elapsed() < Duration::from_secs(300),
                "the Direct Files projection did not converge"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        let reader = PhysicalProjectionQueryReader::open(&path).expect("the read-only seam opens");
        Corpus {
            graph,
            reader,
            root,
            owns_root,
        }
    }

    fn today(&self) -> JournalDate {
        JournalDate::today()
    }

    /// The walk's answer: the block ids (or page names) one query returns.
    fn walk(&self, source: &str, dialect: QueryDialect) -> BTreeSet<String> {
        let result = crate::query::run_query_result(
            &self.graph,
            source,
            dialect,
            Bounds {
                max_rows: usize::MAX,
                max_bytes: usize::MAX,
            },
        );
        match result.rows {
            QueryRows::Block { groups } => groups
                .into_iter()
                .flat_map(|group| group.blocks.into_iter().map(|block| block.id))
                .collect(),
            QueryRows::Page { pages } => pages
                .into_iter()
                .map(|page| crate::refs::page_key(&page.name))
                .collect(),
        }
    }

    /// The EXISTING FTS-building signal, read on the SAME materialized read as
    /// the query and separately from projection readiness (§5.10). A freshly
    /// created projection is published `phase = 1` and maintains its FTS rows
    /// inline; `phase = 0` is the transient building state, where the change
    /// rows go to `search_fts_outbox` and the FTS tables stay empty.
    fn fts_ready(&self) -> bool {
        let rows = self
            .reader
            .run_projection_query(
                "SELECT phase FROM search_fts_build WHERE singleton = 1",
                &[],
            )
            .expect("the FTS build marker is readable through the seam");
        matches!(
            rows.first().and_then(|row| row.first()),
            Some(PhysicalQueryValue::Integer(1))
        )
    }

    /// How many block rows the substring FTS holds. A ready path that measured
    /// an EMPTY index would prove nothing, so the gates assert this is nonzero
    /// before trusting a candidate bound.
    fn substring_fts_rows(&self) -> i64 {
        let rows = self
            .reader
            .run_projection_query("SELECT COUNT(*) FROM search_substring_fts", &[])
            .expect("the substring FTS is readable through the seam");
        match rows.first().and_then(|row| row.first()) {
            Some(PhysicalQueryValue::Integer(count)) => *count,
            other => panic!("COUNT(*) is an integer, got {other:?}"),
        }
    }

    /// The block on one FIXTURE page whose source text contains `needle`, in the
    /// same spelling `sql` returns.
    ///
    /// Only ever called with `write_fast_corpus`'s own lines: it names a block
    /// by text so a nested-`refs` expectation can be READ, and no real-corpus
    /// content reaches an assertion through it.
    fn block_id_containing(&self, page: &str, needle: &str) -> String {
        let rows = self
            .reader
            .run_projection_query(
                "SELECT b.block_id FROM blocks b \
                 JOIN pages p ON p.page_id = b.page_id \
                 JOIN block_text t ON t.block_id = b.block_id \
                 WHERE p.name = ?1 AND instr(t.content, ?2) > 0",
                &[
                    PhysicalQueryValue::Text(page.to_string()),
                    PhysicalQueryValue::Text(needle.to_string()),
                ],
            )
            .expect("the fixture block is readable through the seam");
        assert_eq!(
            rows.len(),
            1,
            "{page}/{needle:?} must name exactly one fixture block"
        );
        match rows[0].first() {
            Some(PhysicalQueryValue::Blob(id)) => Uuid::from_slice(id)
                .expect("a 16-byte block id")
                .to_string(),
            other => panic!("a block row selects its id, got {other:?}"),
        }
    }

    /// One page's id, read through the seam — the value §5.9's overlay masking
    /// binds.
    fn page_id(&self, name: &str) -> [u8; 16] {
        let rows = self
            .reader
            .run_projection_query(
                "SELECT page_id FROM pages WHERE name = ?1",
                &[PhysicalQueryValue::Text(name.to_string())],
            )
            .expect("the page id is readable through the seam");
        match rows.first().and_then(|row| row.first()) {
            Some(PhysicalQueryValue::Blob(id)) => {
                id.as_slice().try_into().expect("a 16-byte page id")
            }
            other => panic!("{name} has no page row: {other:?}"),
        }
    }

    /// Every block id on one page, in the same spelling `sql` returns.
    fn block_ids_on_page(&self, name: &str) -> BTreeSet<String> {
        let rows = self
            .reader
            .run_projection_query(
                "SELECT b.block_id FROM blocks b JOIN pages p ON p.page_id = b.page_id \
                 WHERE p.name = ?1",
                &[PhysicalQueryValue::Text(name.to_string())],
            )
            .expect("the page's blocks are readable through the seam");
        rows.into_iter()
            .map(|row| match row.first() {
                Some(PhysicalQueryValue::Blob(id)) => Uuid::from_slice(id)
                    .expect("a 16-byte block id")
                    .to_string(),
                other => panic!("a block row selects its id, got {other:?}"),
            })
            .collect()
    }

    /// One lowered statement, with the SAME shared Match parse the walk builds
    /// for this execution (§5.10) — never a second `Matcher::parse`.
    fn lower(
        &self,
        source: &str,
        dialect: QueryDialect,
        fts_ready: bool,
        masked_pages: &[[u8; 16]],
    ) -> (Anchor, SqlQuery) {
        self.lower_as(source, dialect, fts_ready, masked_pages, RESULT_SET_RULE)
    }

    /// The same lowering under one named spelling of §5.3's result-set rule, so
    /// the two can be compared and timed against each other.
    fn lower_as(
        &self,
        source: &str,
        dialect: QueryDialect,
        fts_ready: bool,
        masked_pages: &[[u8; 16]],
        result_set_rule: ResultSetRule,
    ) -> (Anchor, SqlQuery) {
        let today = self.today();
        let (query, _view) = crate::query::parse_query_text(source, dialect, today);
        let registry = self.graph.property_registry();
        let compiled = crate::query::eval::CompiledLeaves::for_query(&query.evaluable_filter());
        let inputs = LoweringInputs {
            today,
            registry: &registry,
            masked_pages,
            cutoff: None,
            compiled: &compiled,
            fts_ready,
            result_set_rule,
        };
        (query.anchor, lower_query(&query, &inputs))
    }

    /// Install §4.3.2's compiled-regex table for the statement about to run.
    ///
    /// Unconditional, exactly as §5.9's dispatch does it: this reader is REUSED
    /// by every shape in a gate, so a statement's table must REPLACE the
    /// previous one's rather than be added to it. Installing the empty program
    /// is what proves a later statement cannot answer through a stale ID.
    fn bind_regexes(&self, regexes: &QueryRegexProgram) {
        self.reader
            .set_query_regex_predicate(regexes.predicate())
            .expect("the regex predicate installs on the read-only seam");
    }

    /// The lowering's answer over the same graph, through the D-15 seam.
    fn sql(&self, source: &str, dialect: QueryDialect) -> BTreeSet<String> {
        self.sql_with(source, dialect, self.fts_ready(), &[])
    }

    fn sql_with(
        &self,
        source: &str,
        dialect: QueryDialect,
        fts_ready: bool,
        masked_pages: &[[u8; 16]],
    ) -> BTreeSet<String> {
        self.sql_as(source, dialect, fts_ready, masked_pages, RESULT_SET_RULE)
    }

    fn sql_as(
        &self,
        source: &str,
        dialect: QueryDialect,
        fts_ready: bool,
        masked_pages: &[[u8; 16]],
        result_set_rule: ResultSetRule,
    ) -> BTreeSet<String> {
        let (anchor, statement) =
            self.lower_as(source, dialect, fts_ready, masked_pages, result_set_rule);
        self.bind_regexes(&statement.regexes);
        let rows = self
            .reader
            .run_projection_query(&statement.sql, &statement.params)
            .unwrap_or_else(|error| {
                panic!("the lowered statement must run: {error}\n{}", statement.sql)
            });
        rows.into_iter()
            .map(|row| match (anchor, row.first()) {
                (Anchor::Block, Some(PhysicalQueryValue::Blob(id))) => Uuid::from_slice(id)
                    .expect("a 16-byte block id")
                    .to_string(),
                (Anchor::Page, _) => match row.get(1) {
                    Some(PhysicalQueryValue::Text(name)) => crate::refs::page_key(name),
                    other => panic!("a page row selects its name, got {other:?}"),
                },
                (_, other) => panic!("a block row selects its id, got {other:?}"),
            })
            .collect()
    }

    /// `(plan, positively_bounded, matches_nothing)`.
    fn explain(&self, source: &str, dialect: QueryDialect) -> (Vec<String>, bool, bool) {
        let (_anchor, statement) = self.lower(source, dialect, self.fts_ready(), &[]);
        self.bind_regexes(&statement.regexes);
        // The parameters are bound for the EXPLAIN too: with `sqlite_stat4`
        // present the planner may choose differently for a bound value than for
        // an unbound one, and an explain that left them out would measure a
        // statement nobody runs.
        let plan = self
            .reader
            .explain_query_plan(&statement.sql, &statement.params)
            .expect("the plan is available");
        (
            plan,
            statement.positively_bounded,
            statement.matches_nothing,
        )
    }
}

/// The permanent fast corpus. Every shape §5's tables name has a row here, and a
/// disagreement found on the real graph is extracted INTO this function.
fn write_fast_corpus(root: &Path) {
    std::fs::create_dir_all(root.join("pages")).expect("pages");
    std::fs::create_dir_all(root.join("journals")).expect("journals");

    // Refs (own, ancestor, page), tags, children, and the top-level-root rule:
    // `child under project` matches `[[Project]]` through its ANCESTOR, and is
    // dropped from the result because its parent matched too.
    std::fs::write(
        root.join("pages/refs.md"),
        "- root mentions [[Project]]\n\
         \t- child under project\n\
         \t\t- grandchild under project\n\
         - a #inline-tag line\n\
         - plain line with no reference at all\n",
    )
    .expect("refs page");

    // Tasks, priorities and planning, including the two shapes `tasks` alone
    // cannot answer: a markerless `[#A]` and a markerless `SCHEDULED:`.
    std::fs::write(
        root.join("pages/tasks.md"),
        "- TODO [#A] marked and prioritised\n\
         \t- DONE nested done\n\
         - DOING plain doing\n\
         - [#B] markerless priority\n\
         - markerless schedule\n  SCHEDULED: <2026-06-28 Sun>\n\
         - malformed schedule\n  SCHEDULED: <2026-13-45 Xxx>\n\
         - deadline only\n  DEADLINE: <2026-07-01 Wed>\n\
         - LATER [#a] lowercase priority letter\n",
    )
    .expect("tasks page");

    // Properties: repeated keys, case-varying keys, comma lists, a key-only
    // block, typed values, and an owner with no properties at all (the sparse
    // row that makes a NULL comparison visible).
    std::fs::write(
        root.join("pages/props.md"),
        "type:: Page\n\
         tags:: Genre, Reference\n\
         \n\
         - k:: a\n\
         - k:: a\n\
         - K:: b\n\
         - k:: a, c\n\
         - status:: open\n\
         - priority:: done\n\
         - score:: 12\n\
         - due:: [[Jun 28th, 2026]]\n\
         - blank::\n\
         - a block with no properties\n",
    )
    .expect("props page");

    // Namespaces and page-name ranges.
    std::fs::write(root.join("pages/Proj.md"), "- the namespace parent\n").expect("Proj");
    std::fs::write(
        root.join("pages/Proj%2FAlpha.md"),
        "- inside the namespace\n",
    )
    .expect("Proj/Alpha");
    std::fs::write(
        root.join("pages/Proj%2FAlpha%2FDeep.md"),
        "- two levels down\n",
    )
    .expect("Proj/Alpha/Deep");

    // Content: repeated whitespace and a line break, which `searchable_text`
    // collapses and `query_visible` does not.
    std::fs::write(
        root.join("pages/content.md"),
        "- alpha  beta\n- alpha beta\n- ALPHA BETA gamma\n- 100% literal_underscore\n",
    )
    .expect("content page");

    // §5.10's own acceptance list, as text. Every line here exists to make one
    // named property of `content match` DECIDABLE on this corpus rather than
    // assertable only against a real graph:
    //
    // * `foobar` must be found by `foo` (a trigram), by `oob` (a trigram that
    //   crosses no token boundary the word tokenizer would respect) and by `oo`
    //   (no trigram at all, so no bound and the exact predicate alone);
    // * three consecutive spaces and a mid-line tab survive in
    //   `query_visible_folded` and do NOT survive the FTS producers' whitespace
    //   collapsing, so a phrase term must be matched exactly and bounded by a
    //   whitespace-free RUN of itself;
    // * punctuation, an embedded double quote and an embedded control character
    //   must not turn a candidate needle into FTS query syntax or drop it;
    // * the precomposed and decomposed spellings of `Café` are the same string
    //   after `canonical_fold`, on both the column and the needle.
    std::fs::write(
        root.join("pages/search.md"),
        "- foobar in one token\n\
         - gamma   delta with three spaces\n\
         - punctuated: (paren) [brack] fine\n\
         - quoted say\"hi inside a word\n\
         - Caf\u{e9} precomposed here\n\
         - Cafe\u{301} decomposed here\n\
         - control\u{7}chars needle here\n\
         - tab\tseparated words\n\
         - multi line one\n  continued second line\n\
         - alpha draft note\n\
         \t- nested needle under alpha draft\n",
    )
    .expect("search page");

    // §5.3's TRANSITIVE case, the one the two result-set spellings have to be
    // checked against rather than reasoned about: a block whose GRANDPARENT
    // matches but whose parent does not. `tree/filter-top-level-blocks` drops a
    // match whose IMMEDIATE parent matched, so both `nested outer` and `nested
    // inner` are results and the middle line is not. A `refs` predicate cannot
    // produce this shape (the path-refs closure makes every descendant of a
    // match a match), which is why it is spelled with a task marker.
    std::fs::write(
        root.join("pages/nesting.md"),
        "- TODO nested outer\n\
         \t- plain middle with no marker\n\
         \t\t- TODO nested inner\n",
    )
    .expect("nesting page");

    // §3.2's nested-`refs` context, spelled so every rule of it is DECIDABLE
    // here rather than only on a real graph. `eval_block_leaf` passes the
    // ANCHOR's ancestor multiset through each `children` quantifier unchanged,
    // so a nested block's `refs` closure is its OWN refs, the ANCHOR's
    // ancestors' refs, and the page — never the nested block's own materialized
    // `block_path_refs` closure, and never that closure with the parent's names
    // subtracted. Each group below makes one of those differences visible:
    //
    // * `alpha root` owns `[[Project]]` and its children do NOT see it (the
    //   anchor is a root, so the ancestor context is empty). A lowering that
    //   read `block_path_refs(child)` would answer the opposite.
    // * `middle names [[Shared]]` owns `shared` AND inherits `shared` from its
    //   root, so its child still sees `shared` through the GRANDPARENT. A
    //   lowering that subtracted the parent's own names would lose it.
    // * the two-level `root → middle → deep` chain is where the context must
    //   stay the ANCHOR's: at depth two `deep` still sees only its own refs and
    //   the page, so `any(children, any(children, ref('Shared')))` is FALSE.
    // * `#nested-tag` is an own ref that arrives as a tag, and the page name
    //   `nested-refs` is in every block's closure at every depth.
    std::fs::write(
        root.join("pages/nested-refs.md"),
        "- alpha root [[Project]]\n\
         \t- child names [[Other]]\n\
         \t\t- grandchild names [[Project]]\n\
         \t- second child with no refs\n\
         - bare root with no refs of its own\n\
         \t- lone child names [[Project]]\n\
         - shared root names [[Shared]]\n\
         \t- middle names [[Shared]]\n\
         \t\t- deep child with no refs\n\
         - tagged root\n\
         \t- child #nested-tag here\n",
    )
    .expect("nested refs page");

    // §4.3.2's regex predicate, over the EXACT visible text. Every line here
    // exists to separate `block_text.query_visible` from the two columns beside
    // it: `blocks.query_visible_folded` is lower-cased and NFC-folded, and
    // `searchable_text` collapses runs of whitespace and line breaks. A regex
    // answered from either of those would disagree with the walk, which reads
    // `BlockProjection::visible`.
    std::fs::write(
        root.join("pages/regex.md"),
        "- SHOUTING case survives here\n\
         - lowercase only line\n\
         - digits 12345 inline\n\
         - spaced   out   words\n\
         - anchored start of a line\n\
         - Uppercase\u{c9}clair accented\n\
         - regex needle alpha here\n\
         - regex needle beta here\n\
         \t- nested regex needle here\n",
    )
    .expect("regex page");

    // Journals: one that parses, and one whose stem does not.
    std::fs::write(
        root.join("journals/2026_06_28.md"),
        "- a journal entry [[Project]]\n",
    )
    .expect("journal");
    std::fs::write(root.join("journals/not_a_date.md"), "- an unparsed stem\n")
        .expect("odd journal");
}

/// Every query shape this wave lowers, in both dialects where both spell it.
/// The two engines must agree on every one of them.
const IDENTITY_SHAPES: &[(&str, QueryDialect)] = &[
    // refs — the flagship leaf, through the ancestor closure and the page
    ("[[Project]]", QueryDialect::Og),
    ("(page-ref Project)", QueryDialect::Og),
    ("#inline-tag", QueryDialect::Og),
    ("ref('Project')", QueryDialect::Tql),
    ("not ref('Project')", QueryDialect::Tql),
    ("tag('inline-tag')", QueryDialect::Tql),
    ("not tag('inline-tag')", QueryDialect::Tql),
    // tasks, priorities, planning
    ("(task TODO)", QueryDialect::Og),
    ("(task TODO DOING)", QueryDialect::Og),
    ("(not (task DONE))", QueryDialect::Og),
    ("(priority A)", QueryDialect::Og),
    ("(priority A B)", QueryDialect::Og),
    ("task = 'todo'", QueryDialect::Tql),
    ("task != 'DONE'", QueryDialect::Tql),
    ("task in ('TODO', 'DOING')", QueryDialect::Tql),
    ("task not in ('DONE')", QueryDialect::Tql),
    ("task is not null", QueryDialect::Tql),
    ("task is null", QueryDialect::Tql),
    ("priority = 'a'", QueryDialect::Tql),
    ("priority != 'A'", QueryDialect::Tql),
    ("priority is not null", QueryDialect::Tql),
    ("priority is null", QueryDialect::Tql),
    ("scheduled is not null", QueryDialect::Tql),
    ("scheduled is null", QueryDialect::Tql),
    ("deadline is not null", QueryDialect::Tql),
    ("scheduled = '2026-06-28'", QueryDialect::Tql),
    ("scheduled >= '2026-06-01'", QueryDialect::Tql),
    ("scheduled < '2026-07-01'", QueryDialect::Tql),
    ("scheduled != '2026-06-28'", QueryDialect::Tql),
    (
        "scheduled between '2026-06-01' and '2026-07-01'",
        QueryDialect::Tql,
    ),
    ("deadline > '2026-06-30'", QueryDialect::Tql),
    // properties — every §3.3 form
    ("(property status open)", QueryDialect::Og),
    ("(property k a)", QueryDialect::Og),
    ("(property k c)", QueryDialect::Og),
    ("(property k b)", QueryDialect::Og),
    ("(property score 12)", QueryDialect::Og),
    ("(property blank)", QueryDialect::Og),
    ("(property type Page)", QueryDialect::Og),
    ("(page-property type Page)", QueryDialect::Og),
    ("(page-tags Genre)", QueryDialect::Og),
    ("(all-page-tags)", QueryDialect::Og),
    (
        "(and (property status open) (property priority done))",
        QueryDialect::Og,
    ),
    ("prop('k') is not null", QueryDialect::Tql),
    ("prop('k') is null", QueryDialect::Tql),
    ("prop('k') = 'a'", QueryDialect::Tql),
    ("prop('k') != 'a'", QueryDialect::Tql),
    ("prop('k') in ('a', 'c')", QueryDialect::Tql),
    ("prop('k') not in ('a')", QueryDialect::Tql),
    ("prop('k') like 'a%'", QueryDialect::Tql),
    ("prop('k') = ''", QueryDialect::Tql),
    ("prop('blank') = ''", QueryDialect::Tql),
    ("prop('score') > 5", QueryDialect::Tql),
    ("prop('score') <= 12", QueryDialect::Tql),
    ("prop('score') = 12", QueryDialect::Tql),
    ("prop('score') between 1 and 20", QueryDialect::Tql),
    ("every(prop('k'), value = 'a')", QueryDialect::Tql),
    ("every(prop('k'), value != 'zzz')", QueryDialect::Tql),
    ("none(prop('k'), value = 'a')", QueryDialect::Tql),
    ("any(prop('k'), value like 'a%')", QueryDialect::Tql),
    ("page_prop('type') = 'page'", QueryDialect::Tql),
    ("page_prop('type') is not null", QueryDialect::Tql),
    ("page_tag('Genre')", QueryDialect::Tql),
    // content
    ("\"alpha beta\"", QueryDialect::Og),
    ("content like '%alpha%'", QueryDialect::Tql),
    ("content = 'alpha  beta'", QueryDialect::Tql),
    ("content != 'alpha beta'", QueryDialect::Tql),
    ("content like 'alpha%'", QueryDialect::Tql),
    ("content like '100\\%%'", QueryDialect::Tql),
    ("content like '%literal\\_underscore%'", QueryDialect::Tql),
    // §5.10 — `content match`, in both dialects. Every one of these is
    // compared against the walk, which is the ONLY definition of what they
    // mean; the SQL side may reach fewer rows on the way but never a different
    // answer.
    ("(search \"alpha\")", QueryDialect::Og),
    ("(search \"foo\")", QueryDialect::Og),
    ("(search \"oob\")", QueryDialect::Og),
    ("(search \"oo\")", QueryDialect::Og),
    ("(search \"foo bar\")", QueryDialect::Og),
    ("(search \"alpha OR needle\")", QueryDialect::Og),
    ("(search \"alpha -draft\")", QueryDialect::Og),
    ("content match 'alpha'", QueryDialect::Tql),
    ("content match 'ALPHA'", QueryDialect::Tql),
    ("not content match 'alpha'", QueryDialect::Tql),
    ("content match 'foo'", QueryDialect::Tql),
    ("content match 'oob'", QueryDialect::Tql),
    ("content match 'oo'", QueryDialect::Tql),
    ("content match 'foo bar'", QueryDialect::Tql),
    ("content match 'oo OR alpha'", QueryDialect::Tql),
    ("content match 'alpha OR needle'", QueryDialect::Tql),
    ("content match 'alpha -draft'", QueryDialect::Tql),
    ("content match 'alpha -zzz'", QueryDialect::Tql),
    // Phrases whose whitespace the FTS producers collapse and the exact column
    // keeps: the candidate needle is a whitespace-free RUN of the phrase, and
    // the phrase itself still has to be matched exactly.
    ("content match '\" alpha\"'", QueryDialect::Tql),
    ("content match '\"alpha  beta\"'", QueryDialect::Tql),
    ("content match '\"alpha beta\"'", QueryDialect::Tql),
    ("content match '\"gamma   delta\"'", QueryDialect::Tql),
    ("content match '\"   \"'", QueryDialect::Tql),
    ("content match '\"tab\tseparated\"'", QueryDialect::Tql),
    ("content match '\"one continued\"'", QueryDialect::Tql),
    ("content match '\"line one\"'", QueryDialect::Tql),
    // Punctuation, an embedded quote, a control character and Unicode folding —
    // the candidate-superset pins §5.10 names. A false negative here is a
    // failure of this packet, not a tuning parameter.
    ("content match '(paren)'", QueryDialect::Tql),
    ("content match '[brack]'", QueryDialect::Tql),
    ("content match 'say\"hi'", QueryDialect::Tql),
    ("content match 'ol\u{7}ch'", QueryDialect::Tql),
    ("content match 'chars'", QueryDialect::Tql),
    ("content match 'caf\u{e9}'", QueryDialect::Tql),
    ("content match 'Cafe\u{301}'", QueryDialect::Tql),
    // Exclusion-only input, a blank query and an invalid regex are FALSE
    // LEAVES, and `not` over one is classically true (§3.4/§5.10).
    ("content match '-alpha'", QueryDialect::Tql),
    ("content match '   '", QueryDialect::Tql),
    ("not content match '-alpha'", QueryDialect::Tql),
    ("content regexp '['", QueryDialect::Tql),
    ("(content-regex \"[\")", QueryDialect::Og),
    ("content match '/[/'", QueryDialect::Tql),
    ("any(children, content match 'needle')", QueryDialect::Tql),
    ("(and (task TODO) (search \"alpha\"))", QueryDialect::Og),
    // page attributes, reached from the block anchor through `page`
    ("(page refs)", QueryDialect::Og),
    ("(namespace Proj)", QueryDialect::Og),
    ("(journal)", QueryDialect::Og),
    ("(between '2026-06-01' '2026-07-01')", QueryDialect::Og),
    ("page.name = 'refs'", QueryDialect::Tql),
    ("page.name != 'refs'", QueryDialect::Tql),
    ("page.name in ('refs', 'tasks')", QueryDialect::Tql),
    ("page.name like 'proj/%'", QueryDialect::Tql),
    ("page.name like '%roj%'", QueryDialect::Tql),
    ("page.journal = true", QueryDialect::Tql),
    ("page.journal = false", QueryDialect::Tql),
    ("page.day is not null", QueryDialect::Tql),
    ("page.day is null", QueryDialect::Tql),
    ("page.day >= '2026-01-01'", QueryDialect::Tql),
    ("page.namespace = 'proj'", QueryDialect::Tql),
    ("page.namespace = 'proj/alpha'", QueryDialect::Tql),
    ("page.namespace is not null", QueryDialect::Tql),
    ("page.namespace is null", QueryDialect::Tql),
    ("page.namespace != 'proj'", QueryDialect::Tql),
    // page-anchored rows
    ("@page and journal = true", QueryDialect::Tql),
    ("@page and journal = false", QueryDialect::Tql),
    ("@page and name like 'proj/%'", QueryDialect::Tql),
    ("@page and name = 'props'", QueryDialect::Tql),
    ("@page and day >= '2026-01-01'", QueryDialect::Tql),
    ("@page and day is not null", QueryDialect::Tql),
    ("@page and prop('type') is not null", QueryDialect::Tql),
    ("@page and prop('type') = 'page'", QueryDialect::Tql),
    ("@page and namespace = 'proj'", QueryDialect::Tql),
    ("@page and not name = 'refs'", QueryDialect::Tql),
    // `blocks` is a page relation the walk answers false for; the lowering
    // reproduces that rather than inventing an answer.
    ("@page and any(blocks, task = 'TODO')", QueryDialect::Tql),
    // relations and boolean composition, including the two `every` polarities
    ("any(children, task = 'DONE')", QueryDialect::Tql),
    ("none(children, task = 'DONE')", QueryDialect::Tql),
    ("every(children, task = 'DONE')", QueryDialect::Tql),
    ("every(children, not task = 'DONE')", QueryDialect::Tql),
    ("any(children, content like '%nested%')", QueryDialect::Tql),
    (
        "(or (and (task TODO) (priority A)) (property status open))",
        QueryDialect::Og,
    ),
    ("(and (task TODO) (not [[Project]]))", QueryDialect::Og),
    ("(or [[Project]] (task DOING))", QueryDialect::Og),
    // §3.2's nested `refs`: the ancestor context a `children` quantifier passes
    // down is the ANCHOR's, unchanged, at every depth. See `write_fast_corpus`'s
    // `nested-refs` page for which line makes which rule decidable.
    ("any(children, ref('Project'))", QueryDialect::Tql),
    ("none(children, ref('Project'))", QueryDialect::Tql),
    ("every(children, ref('Project'))", QueryDialect::Tql),
    ("not any(children, ref('Project'))", QueryDialect::Tql),
    ("any(children, not ref('Project'))", QueryDialect::Tql),
    ("any(children, ref('Other'))", QueryDialect::Tql),
    ("any(children, ref('Shared'))", QueryDialect::Tql),
    ("none(children, ref('Shared'))", QueryDialect::Tql),
    ("every(children, ref('Shared'))", QueryDialect::Tql),
    // Two levels down, where the context must still be the ANCHOR's.
    (
        "any(children, any(children, ref('Shared')))",
        QueryDialect::Tql,
    ),
    (
        "any(children, any(children, ref('Project')))",
        QueryDialect::Tql,
    ),
    (
        "any(children, none(children, ref('Shared')))",
        QueryDialect::Tql,
    ),
    // The page name is in every closure, at every depth.
    ("any(children, ref('nested-refs'))", QueryDialect::Tql),
    ("every(children, ref('nested-refs'))", QueryDialect::Tql),
    ("none(children, ref('nested-refs'))", QueryDialect::Tql),
    (
        "any(children, any(children, ref('nested-refs')))",
        QueryDialect::Tql,
    ),
    // An own ref that arrives as a tag, and the tag relation beside it.
    ("any(children, ref('nested-tag'))", QueryDialect::Tql),
    ("any(children, tag('nested-tag'))", QueryDialect::Tql),
    // A nested `refs` composed with another leaf and with a page relation.
    (
        "any(children, ref('Project') and content like '%grandchild%')",
        QueryDialect::Tql,
    ),
    (
        "any(children, ref('Project') or task = 'TODO')",
        QueryDialect::Tql,
    ),
    (
        "any(children, page.name = 'nested-refs' and ref('Shared'))",
        QueryDialect::Tql,
    ),
    // `blocks` is a page relation the walk answers false for, nested predicate
    // and all — the lowering reproduces that rather than inventing an answer.
    ("@page and any(blocks, ref('Project'))", QueryDialect::Tql),
    (
        "@page and any(blocks, any(children, ref('Project')))",
        QueryDialect::Tql,
    ),
    // §4.3.2 — a VALID regex, in both spellings, over the EXACT visible text.
    ("content regexp 'needle'", QueryDialect::Tql),
    ("(content-regex \"needle\")", QueryDialect::Og),
    ("content regexp 'SHOUTING'", QueryDialect::Tql),
    ("content regexp 'shouting'", QueryDialect::Tql),
    ("content regexp '[A-Z]{3,}'", QueryDialect::Tql),
    ("content regexp '^anchored'", QueryDialect::Tql),
    ("content regexp 'anchored$'", QueryDialect::Tql),
    ("content regexp '[0-9]+'", QueryDialect::Tql),
    ("content regexp 'spaced\\s{3}out'", QueryDialect::Tql),
    ("content regexp 'gamma\\s+delta'", QueryDialect::Tql),
    ("content regexp 'line one\\s+continued'", QueryDialect::Tql),
    // Unicode, case-sensitively, on text the folded column would have changed.
    ("content regexp 'Uppercase\u{c9}clair'", QueryDialect::Tql),
    ("content regexp 'uppercase\u{e9}clair'", QueryDialect::Tql),
    ("content regexp 'Caf\u{e9}'", QueryDialect::Tql),
    ("content regexp 'Cafe\u{301}'", QueryDialect::Tql),
    // The whole-query `/pattern/` form of `content match`, in both dialects.
    ("content match '/needle/'", QueryDialect::Tql),
    ("(search \"/needle/\")", QueryDialect::Og),
    ("content match '/[A-Z]{3,}/'", QueryDialect::Tql),
    // Nested booleans, several patterns in one statement, both syntaxes
    // together, and a regex under a `children` quantifier.
    ("not content regexp 'needle'", QueryDialect::Tql),
    (
        "(and (content-regex \"needle\") (content-regex \"alpha\"))",
        QueryDialect::Og,
    ),
    (
        "(and (content-regex \"needle\") (search \"/beta/\"))",
        QueryDialect::Og,
    ),
    (
        "(or (content-regex \"needle\") (task TODO))",
        QueryDialect::Og,
    ),
    (
        "(and (task TODO) (content-regex \"marked\"))",
        QueryDialect::Og,
    ),
    (
        "(not (and (content-regex \"needle\") (task TODO)))",
        QueryDialect::Og,
    ),
    ("any(children, content regexp 'needle')", QueryDialect::Tql),
    (
        "any(children, ref('nested-refs') and content regexp 'needle')",
        QueryDialect::Tql,
    ),
];

/// The `walk == SQL` acceptance gate, over the permanent fast corpus.
#[test]
fn the_walk_and_the_lowering_answer_every_shape_identically() {
    let _serial = serialize();
    let root = scratch("identity");
    write_fast_corpus(&root);
    let corpus = Corpus::open(root, true);
    let differences = compare_every_shape(&corpus);
    assert!(
        differences.is_empty(),
        "the walk and the lowering disagree:\n{}",
        differences.join("\n")
    );
}

/// The same gate over the anonymized graph (AGENTS §4 tier 2). A disagreement
/// here is a CORPUS DEFECT in the fixture above: extract the minimal shape into
/// `write_fast_corpus`, never weaken the gate. Only shape sources and counts are
/// printed — no page name, block text or property value.
#[test]
#[ignore = "acceptance gate over a real corpus: set TINE_QUERY_IDENTITY_GRAPH"]
fn the_walk_and_the_lowering_agree_over_a_real_corpus() {
    let _serial = serialize();
    let Some(root) = std::env::var_os("TINE_QUERY_IDENTITY_GRAPH") else {
        eprintln!("skipped: set TINE_QUERY_IDENTITY_GRAPH to a corpus directory");
        return;
    };
    let corpus = Corpus::open(PathBuf::from(&root), false);
    let mut rows = 0usize;
    for (source, dialect) in IDENTITY_SHAPES {
        rows += corpus.sql(source, *dialect).len();
    }
    let differences = compare_every_shape(&corpus);
    eprintln!(
        "walk_sql_identity_over_a_real_corpus shapes={} matched_rows={rows} disagreements={}",
        IDENTITY_SHAPES.len(),
        differences.len()
    );
    assert!(
        differences.is_empty(),
        "the walk and the lowering disagree on a real graph (shape sources only):\n{}",
        differences.join("\n")
    );
}

/// Compares both engines on every shape, returning ONE line per disagreement.
/// The line names the query source and the two cardinalities and nothing else,
/// so it is safe to print for a real corpus.
fn compare_every_shape(corpus: &Corpus) -> Vec<String> {
    let mut differences = Vec::new();
    for (source, dialect) in IDENTITY_SHAPES {
        let sql = corpus.sql(source, *dialect);
        let walk = corpus.walk(source, *dialect);
        if walk != sql {
            differences.push(format!(
                "{source}: walk={} sql={} only_in_walk={} only_in_sql={}",
                walk.len(),
                sql.len(),
                walk.difference(&sql).count(),
                sql.difference(&walk).count()
            ));
        }
    }
    differences
}

/// A gate that cannot fail is not a gate: the fast corpus must actually produce
/// matches, or `walk == SQL` would be `{} == {}` for every shape.
#[test]
fn the_fast_corpus_answers_every_shape_it_can_and_matches_something() {
    let _serial = serialize();
    let root = scratch("coverage");
    write_fast_corpus(&root);
    let corpus = Corpus::open(root, true);
    let mut nonempty = 0usize;
    for (source, dialect) in IDENTITY_SHAPES {
        if !corpus.sql(source, *dialect).is_empty() {
            nonempty += 1;
        }
    }
    assert!(
        nonempty * 2 >= IDENTITY_SHAPES.len(),
        "only {nonempty} of {} shapes match anything; the corpus is too thin to prove identity",
        IDENTITY_SHAPES.len()
    );
}

/// The shapes §5.7's table calls positively bounded, plus the two presence
/// probes the dossier names as a hard stop, plus the two controls §5.7 predicts
/// will NOT be bounded.
const PLAN_SHAPES: &[(&str, QueryDialect)] = &[
    ("[[Project]]", QueryDialect::Og),
    ("#inline-tag", QueryDialect::Og),
    ("tag('inline-tag')", QueryDialect::Tql),
    ("(task TODO)", QueryDialect::Og),
    ("(property status open)", QueryDialect::Og),
    (
        "(and (property status open) (property priority done))",
        QueryDialect::Og,
    ),
    ("(priority A)", QueryDialect::Og),
    ("scheduled is not null", QueryDialect::Tql),
    ("deadline is not null", QueryDialect::Tql),
    ("scheduled >= '2026-06-01'", QueryDialect::Tql),
    ("deadline < '2026-07-01'", QueryDialect::Tql),
    ("prop('score') > 5", QueryDialect::Tql),
    ("prop('k') = 'a'", QueryDialect::Tql),
    ("every(prop('k'), value = 'a')", QueryDialect::Tql),
    ("any(children, task = 'DONE')", QueryDialect::Tql),
    ("page.name = 'refs'", QueryDialect::Tql),
    ("@page and name like 'proj/%'", QueryDialect::Tql),
    ("@page and day >= '2026-01-01'", QueryDialect::Tql),
    ("@page and prop('type') is not null", QueryDialect::Tql),
    // §5.10: an indexable `content match` IS positively bounded, and no blanket
    // FTS exception permits a permanent full scan of one.
    ("content match 'alpha'", QueryDialect::Tql),
    ("(search \"needle\")", QueryDialect::Og),
    // R2's newly-lowered families. A nested `refs` is deliberately ABSENT: it
    // reads the anchor's own ancestor context, so its subquery is correlated
    // with the anchor and §5.7 now classifies it unbounded — the measurement
    // that established that is
    // [`a_nested_refs_child_predicate_cannot_bound_its_anchor`].
    //
    // Regex is explicitly unindexed (§4.3.2), so it never bounds an anchor by
    // itself — these are the shapes where it rides ALONGSIDE a bounded conjunct.
    // The bound must come from the other leaf and the regex must not cost the
    // anchor its index probe: that is precisely what a plan gate can see and an
    // identity gate cannot.
    ("(and (task TODO) (content-regex \"needle\"))", QueryDialect::Og),
    ("ref('Project') and content regexp 'needle'", QueryDialect::Tql),
];

/// §5.10's plan classes and the shape that produces each. They are recorded
/// SEPARATELY from the indexed case — as classes, not as failures and not as a
/// blanket content exemption.
const CONTENT_PLAN_SHAPES: &[(&str, QueryDialect, ContentPlan)] = &[
    ("content match 'alpha'", QueryDialect::Tql, ContentPlan::Fts),
    ("(search \"needle\")", QueryDialect::Og, ContentPlan::Fts),
    // No positive term yields a three-scalar whitespace-free run.
    (
        "content match 'oo'",
        QueryDialect::Tql,
        ContentPlan::ShortUnindexable,
    ),
    (
        "content match '\"   \"'",
        QueryDialect::Tql,
        ContentPlan::ShortUnindexable,
    ),
    // One unbounded OR arm unbounds the leaf; the other arm keeps its bound.
    (
        "content match 'oo OR alpha'",
        QueryDialect::Tql,
        ContentPlan::ShortUnindexable,
    ),
    // §4.3.2's explicitly unindexed predicate. BOTH regex spellings and BOTH
    // pattern validities lower to a statement, and all four are the same
    // unindexed class: a valid pattern is not a better plan than an invalid one,
    // it just answers instead of being false.
    ("content regexp 'needle'", QueryDialect::Tql, ContentPlan::Regex),
    ("content match '/needle/'", QueryDialect::Tql, ContentPlan::Regex),
    ("content regexp '['", QueryDialect::Tql, ContentPlan::Regex),
    ("content match '/[/'", QueryDialect::Tql, ContentPlan::Regex),
];

/// Every content shape of `IDENTITY_SHAPES`, for the gates that compare the two
/// FTS readiness states against each other and against the walk.
const CONTENT_SHAPES: &[(&str, QueryDialect)] = &[
    ("(search \"alpha\")", QueryDialect::Og),
    ("(search \"foo\")", QueryDialect::Og),
    ("(search \"oob\")", QueryDialect::Og),
    ("(search \"oo\")", QueryDialect::Og),
    ("(search \"foo bar\")", QueryDialect::Og),
    ("(search \"alpha OR needle\")", QueryDialect::Og),
    ("(search \"alpha -draft\")", QueryDialect::Og),
    ("content match 'alpha'", QueryDialect::Tql),
    ("content match 'ALPHA'", QueryDialect::Tql),
    ("not content match 'alpha'", QueryDialect::Tql),
    ("content match 'foo'", QueryDialect::Tql),
    ("content match 'oob'", QueryDialect::Tql),
    ("content match 'oo'", QueryDialect::Tql),
    ("content match 'foo bar'", QueryDialect::Tql),
    ("content match 'oo OR alpha'", QueryDialect::Tql),
    ("content match 'alpha OR needle'", QueryDialect::Tql),
    ("content match 'alpha -draft'", QueryDialect::Tql),
    ("content match 'alpha -zzz'", QueryDialect::Tql),
    ("content match '\" alpha\"'", QueryDialect::Tql),
    ("content match '\"alpha  beta\"'", QueryDialect::Tql),
    ("content match '\"alpha beta\"'", QueryDialect::Tql),
    ("content match '\"gamma   delta\"'", QueryDialect::Tql),
    ("content match '\"   \"'", QueryDialect::Tql),
    ("content match '\"tab\tseparated\"'", QueryDialect::Tql),
    ("content match '\"one continued\"'", QueryDialect::Tql),
    ("content match '\"line one\"'", QueryDialect::Tql),
    ("content match '(paren)'", QueryDialect::Tql),
    ("content match '[brack]'", QueryDialect::Tql),
    ("content match 'say\"hi'", QueryDialect::Tql),
    ("content match 'ol\u{7}ch'", QueryDialect::Tql),
    ("content match 'chars'", QueryDialect::Tql),
    ("content match 'caf\u{e9}'", QueryDialect::Tql),
    ("content match 'Cafe\u{301}'", QueryDialect::Tql),
    ("content match '-alpha'", QueryDialect::Tql),
    ("content match '   '", QueryDialect::Tql),
    ("not content match '-alpha'", QueryDialect::Tql),
    ("content regexp '['", QueryDialect::Tql),
    ("(content-regex \"[\")", QueryDialect::Og),
    ("content match '/[/'", QueryDialect::Tql),
    ("any(children, content match 'needle')", QueryDialect::Tql),
    ("(and (task TODO) (search \"alpha\"))", QueryDialect::Og),
];

/// §5.10's central safety rule, made falsifiable: **a candidate bound may only
/// ever over-approximate.** The two lowerings of the SAME shape — one with the
/// trigram bound, one without — must select exactly the same rows, and both
/// must equal the walk. A bound that excluded one true match shows up here as a
/// difference, not as a slightly faster answer.
///
/// This is the test that makes `foobar` findable by `foo`, `oob` AND `oo`, and
/// it is why `search_fts`'s word-token matches are not substituted for
/// substrings.
#[test]
fn the_fts_candidate_bound_never_excludes_a_true_match() {
    let _serial = serialize();
    let root = scratch("candidate-superset");
    write_fast_corpus(&root);
    let corpus = Corpus::open(root, true);
    // A ready path measured against an EMPTY index would prove nothing.
    assert!(
        corpus.fts_ready(),
        "the fixture projection must publish a READY FTS index"
    );
    assert!(
        corpus.substring_fts_rows() > 0,
        "the substring FTS must actually hold rows, or the bound is vacuous"
    );
    let mut differences = Vec::new();
    let mut bounded_shapes = 0usize;
    for (source, dialect) in CONTENT_SHAPES {
        let bounded = corpus.sql_with(source, *dialect, true, &[]);
        let exact = corpus.sql_with(source, *dialect, false, &[]);
        let walk = corpus.walk(source, *dialect);
        if bounded != exact {
            differences.push(format!(
                "{source}: the candidate bound changed the answer — bounded={} exact={} \
                 only_in_exact={}",
                bounded.len(),
                exact.len(),
                exact.difference(&bounded).count()
            ));
        }
        if walk != bounded {
            differences.push(format!(
                "{source}: walk={} sql={} only_in_walk={}",
                walk.len(),
                bounded.len(),
                walk.difference(&bounded).count()
            ));
        }
        let (_anchor, statement) = corpus.lower(source, *dialect, true, &[]);
        if statement.sql.contains("search_substring_fts") {
            bounded_shapes += 1;
        }
    }
    assert!(
        differences.is_empty(),
        "the candidate bound is not a superset:\n{}",
        differences.join("\n")
    );
    // And the bound is actually exercised, so the comparison above is not two
    // identical unbounded statements agreeing with each other.
    assert!(
        bounded_shapes >= CONTENT_SHAPES.len() / 2,
        "only {bounded_shapes} of {} content shapes reached the index",
        CONTENT_SHAPES.len()
    );
}

/// The transient `fts-building` class (§5.10): with the index still building,
/// the SAME exact predicates run on the ready block columns. Not empty results,
/// not an error, not a new walk route — and the statement never names an FTS
/// table, so a deliberately EMPTY building index cannot change the answer.
/// Nothing here requests a rebuild or waits for one (I-13).
#[test]
fn a_building_fts_index_answers_every_content_shape_from_the_ready_columns() {
    let _serial = serialize();
    let root = scratch("fts-building");
    write_fast_corpus(&root);
    let corpus = Corpus::open(root, true);
    let mut differences = Vec::new();
    for (source, dialect) in CONTENT_SHAPES {
        let (_anchor, statement) = corpus.lower(source, *dialect, false, &[]);
        if statement.sql.contains("search_substring_fts")
            || statement.sql.contains("search_fts_owners")
            || statement.sql.contains("MATCH ")
        {
            differences.push(format!("{source}: the building path still asks the index"));
            continue;
        }
        let sql = corpus.sql_with(source, *dialect, false, &[]);
        let walk = corpus.walk(source, *dialect);
        if walk != sql {
            differences.push(format!(
                "{source}: walk={} sql={} only_in_walk={}",
                walk.len(),
                sql.len(),
                walk.difference(&sql).count()
            ));
        }
    }
    assert!(
        differences.is_empty(),
        "the building path does not answer from the ready columns:\n{}",
        differences.join("\n")
    );
}

/// The same two §5.10 comparisons over the anonymized graph (AGENTS §4 tier 2):
/// walk vs SQL with the FTS index READY, and walk vs SQL with the index
/// BUILDING. A disagreement here is a CORPUS DEFECT in `write_fast_corpus` —
/// extract the minimal shape into the fixture, never weaken the gate. Only
/// shape sources and counts are printed; no page name, block text or property
/// value leaves the corpus.
#[test]
#[ignore = "acceptance gate over a real corpus: set TINE_QUERY_IDENTITY_GRAPH"]
fn the_content_operators_agree_with_the_walk_over_a_real_corpus() {
    let _serial = serialize();
    let Some(root) = std::env::var_os("TINE_QUERY_IDENTITY_GRAPH") else {
        eprintln!("skipped: set TINE_QUERY_IDENTITY_GRAPH to a corpus directory");
        return;
    };
    let corpus = Corpus::open(PathBuf::from(&root), false);
    assert!(
        corpus.fts_ready(),
        "the real corpus must publish a READY FTS"
    );
    let fts_rows = corpus.substring_fts_rows();
    assert!(fts_rows > 0, "the real corpus substring FTS is empty");
    let mut differences = Vec::new();
    let mut bounded = 0usize;
    let mut rows = 0usize;
    let mut classes = std::collections::BTreeMap::<String, usize>::new();
    for (source, dialect) in CONTENT_SHAPES {
        let ready = corpus.sql_with(source, *dialect, true, &[]);
        let building = corpus.sql_with(source, *dialect, false, &[]);
        let walk = corpus.walk(source, *dialect);
        rows += ready.len();
        for (label, answer) in [("ready", &ready), ("building", &building)] {
            if &walk != answer {
                differences.push(format!(
                    "{source} [{label}]: walk={} sql={} only_in_walk={} only_in_sql={}",
                    walk.len(),
                    answer.len(),
                    walk.difference(answer).count(),
                    answer.difference(&walk).count()
                ));
            }
        }
        let (_anchor, statement) = corpus.lower(source, *dialect, true, &[]);
        if statement.positively_bounded {
            bounded += 1;
        }
        for plan in &statement.content_plans {
            *classes.entry(format!("{plan:?}")).or_default() += 1;
        }
    }
    eprintln!(
        "content_identity_over_a_real_corpus shapes={} bounded={bounded} matched_rows={rows} \
         substring_fts_rows={fts_rows} plan_classes={classes:?} disagreements={}",
        CONTENT_SHAPES.len(),
        differences.len()
    );
    assert!(
        differences.is_empty(),
        "the walk and the lowering disagree on content over a real graph \
         (shape sources only):\n{}",
        differences.join("\n")
    );
}

/// §5.9's overlay masking composes with a content leaf and its candidate bound:
/// the masked page's rows leave the statement, and every other row still
/// answers exactly as the walk does. The masked read and the overlay walk
/// cannot both answer for one page.
#[test]
fn a_masked_overlay_page_leaves_a_content_result() {
    let _serial = serialize();
    let root = scratch("masked-content");
    write_fast_corpus(&root);
    let corpus = Corpus::open(root, true);
    let masked = corpus.page_id("search");
    for (source, dialect) in CONTENT_SHAPES {
        let all = corpus.sql_with(source, *dialect, true, &[]);
        let visible = corpus.sql_with(source, *dialect, true, std::slice::from_ref(&masked));
        let on_masked = corpus.block_ids_on_page("search");
        assert!(
            visible.is_subset(&all),
            "{source}: masking may only remove rows"
        );
        let expected: BTreeSet<String> = all.difference(&on_masked).cloned().collect();
        assert_eq!(
            visible, expected,
            "{source}: exactly the masked page leaves"
        );
    }
}

/// §5.10 asks for the short/unindexable, regex and transient-building plan
/// classes to be recorded SEPARATELY from the indexed case. A gate that could
/// not name them would have to choose between failing them and exempting every
/// content predicate.
#[test]
fn the_content_plan_classes_are_recorded_separately_from_the_indexed_case() {
    let _serial = serialize();
    let root = scratch("content-plans");
    write_fast_corpus(&root);
    let corpus = Corpus::open(root, true);
    for (source, dialect, expected) in CONTENT_PLAN_SHAPES {
        let (_anchor, statement) = corpus.lower(source, *dialect, true, &[]);
        assert_eq!(
            statement.content_plans,
            vec![*expected],
            "{source} is a {expected:?} content path"
        );
        // The same leaf, with the index still building, is the transient class
        // on every shape that would otherwise reach it.
        let (_anchor, building) = corpus.lower(source, *dialect, false, &[]);
        let expected_building = match expected {
            ContentPlan::Regex => ContentPlan::Regex,
            _ => ContentPlan::FtsBuilding,
        };
        assert_eq!(
            building.content_plans,
            vec![expected_building],
            "{source} while the index builds"
        );
    }
}

/// §5.3's result-set rule has two spellings and they are the SAME predicate.
///
/// `filter(row)` is a pure function of the row, so "the parent matches" and "the
/// parent is in the match set" cannot differ — but that is an argument, and the
/// dossier's rule is that the TRANSITIVE case (a block whose grandparent matches
/// while its parent does not) is checked against the walk rather than reasoned
/// about. This gate runs every identity shape through BOTH spellings and asserts
/// all three answers agree, so adopting one of them can never become a semantic
/// change.
#[test]
fn the_two_result_set_spellings_answer_identically() {
    let _serial = serialize();
    let root = scratch("result-set-rule");
    write_fast_corpus(&root);
    let corpus = Corpus::open(root, true);
    let fts_ready = corpus.fts_ready();
    let mut disagreements = Vec::new();
    let mut compared = 0usize;
    const SPELLINGS: [ResultSetRule; 3] = [
        ResultSetRule::CorrelatedProbe,
        ResultSetRule::MatchSetCte,
        ResultSetRule::MatchSetCteMaterialized,
    ];
    for (source, dialect) in IDENTITY_SHAPES {
        let walk = corpus.walk(source, *dialect);
        let answers: Vec<_> = SPELLINGS
            .iter()
            .map(|rule| corpus.sql_as(source, *dialect, fts_ready, &[], *rule))
            .collect();
        compared += 1;
        for (rule, rows) in SPELLINGS.iter().zip(&answers) {
            // The `MATERIALIZED` keyword is SQLite 3.35+. A build whose bundled
            // SQLite refuses it must fail HERE, loudly, rather than ship a query
            // path that errors on every block query.
            if *rows != walk {
                disagreements.push(format!(
                    "{source} under {rule:?}: walk={} sql={}",
                    walk.len(),
                    rows.len()
                ));
            }
        }
    }
    assert!(
        disagreements.is_empty(),
        "the two spellings of §5.3's result-set rule are not the same predicate:\n{}",
        disagreements.join("\n")
    );
    // The transitive case, named explicitly so a fixture edit that removed it
    // would fail here rather than silently stop testing it.
    let transitive = corpus.walk("(task TODO)", QueryDialect::Og);
    let nested = corpus.block_ids_on_page("nesting");
    assert_eq!(
        transitive.intersection(&nested).count(),
        2,
        "the nesting fixture must contribute a grandparent match AND a grandchild \
         match whose own parent does not match"
    );
    assert!(compared * 2 >= IDENTITY_SHAPES.len());
}

/// §3.2's nested-`refs` context, on REAL ROWS rather than on emitted text.
///
/// The `walk == SQL` gate above already compares both engines on every nested
/// shape, but two engines can agree by being wrong together, and this fixture's
/// whole point is that three plausible lowerings answer DIFFERENTLY on it. Each
/// assertion below is the one a specific wrong lowering fails:
///
/// * reading `block_path_refs(<nested row>)` — the child of `alpha root` would
///   see its parent's `[[Project]]`;
/// * subtracting the parent's own names from that closure — `middle names
///   [[Shared]]`'s child would stop seeing `shared`, which reaches it from the
///   GRANDPARENT as well;
/// * carrying the immediate parent's context down instead of the ANCHOR's —
///   the two-level shape would match.
#[test]
fn a_nested_refs_leaf_reads_the_anchor_context_and_never_a_subtraction() {
    let _serial = serialize();
    let root = scratch("nested-refs");
    write_fast_corpus(&root);
    let corpus = Corpus::open(root, true);
    let page = "nested-refs";
    let on_page = corpus.block_ids_on_page(page);
    let block = |needle: &str| corpus.block_id_containing(page, needle);
    let here = |source: &str| -> BTreeSet<String> {
        let answer = corpus.sql(source, QueryDialect::Tql);
        assert_eq!(
            answer,
            corpus.walk(source, QueryDialect::Tql),
            "{source}: walk and SQL must already agree"
        );
        answer.intersection(&on_page).cloned().collect()
    };

    // The anchor's OWN refs are NOT in its children's context: `alpha root`
    // owns `[[Project]]`, is a root (so the context is empty), and neither of
    // its children names Project.
    let projects = here("any(children, ref('Project'))");
    assert!(
        !projects.contains(&block("alpha root")),
        "the anchor's own refs are not visible to its children"
    );
    // `child names [[Other]]` DOES match: its own child names Project. And
    // `bare root` matches through its child's own ref. Both survive §5.3's
    // suppression because neither parent matched.
    assert_eq!(
        projects,
        BTreeSet::from([block("[[Other]]"), block("bare root")]),
        "exactly the anchors whose child sees Project"
    );

    // A name reaching the nested row from the GRANDPARENT survives, even though
    // the anchor owns the same name. The content conjunct pins the anchor to
    // `middle`, so §5.3 cannot drop it in favour of its parent.
    assert_eq!(
        here("any(children, ref('Shared')) and content like '%middle%'"),
        BTreeSet::from([block("middle names")]),
        "the grandparent's name is still in the deep child's context"
    );

    // Two levels down the context is STILL the anchor's, so nothing matches:
    // `deep child` owns no ref, and `shared root`'s ancestors are empty.
    assert!(
        here("any(children, any(children, ref('Shared')))").is_empty(),
        "the context does not become the intervening parent's"
    );

    // The page name is in every closure at every depth — the arm a root anchor
    // has no parent row to carry.
    assert_eq!(
        here("any(children, ref('nested-refs'))"),
        BTreeSet::from([
            block("alpha root"),
            block("bare root"),
            block("shared root"),
            block("tagged root"),
        ]),
        "every anchor with a child sees its own page name through that child"
    );
    assert_eq!(
        here("any(children, any(children, ref('nested-refs')))"),
        BTreeSet::from([block("alpha root"), block("shared root")]),
        "and so does every grandchild"
    );

    // An own ref that arrives as a tag is an own ref.
    assert_eq!(
        here("any(children, ref('nested-tag'))"),
        BTreeSet::from([block("tagged root")])
    );

    // The quantifiers, on the same rows: `none` is the complement of `any`
    // BEFORE suppression, so the two are compared through the walk rather than
    // by set arithmetic here — what this pins is that all three answer at all
    // and that `every` over a childless block is vacuously true.
    for source in [
        "none(children, ref('Project'))",
        "every(children, ref('Project'))",
        "every(children, ref('nested-refs'))",
        "any(children, not ref('Project'))",
    ] {
        assert_eq!(
            corpus.sql(source, QueryDialect::Tql),
            corpus.walk(source, QueryDialect::Tql),
            "{source}"
        );
    }
    assert!(
        here("every(children, ref('Project'))").contains(&block("deep child")),
        "a childless block satisfies `every` vacuously (Q5)"
    );
}

/// §4.3.2's regex predicate, on REAL ROWS: the text it sees is the EXACT
/// visible text, and the compiled-regex table is scoped to ONE statement.
///
/// The three columns a regex could plausibly read differ on this fixture —
/// `query_visible` keeps case, accents and whitespace runs; `query_visible_folded`
/// lower-cases and NFC-folds; `searchable_text` collapses whitespace — so each
/// assertion here fails for a lowering that read the wrong one.
#[test]
fn a_regex_predicate_reads_the_exact_visible_text_through_a_statement_scoped_table() {
    let _serial = serialize();
    let root = scratch("regex");
    write_fast_corpus(&root);
    let corpus = Corpus::open(root, true);
    let page = "regex";
    let on_page = corpus.block_ids_on_page(page);
    let block = |needle: &str| corpus.block_id_containing(page, needle);
    let here = |source: &str, dialect: QueryDialect| -> BTreeSet<String> {
        let answer = corpus.sql(source, dialect);
        assert_eq!(
            answer,
            corpus.walk(source, dialect),
            "{source}: walk and SQL must already agree"
        );
        answer.intersection(&on_page).cloned().collect()
    };

    // Case is preserved: the folded column would answer these two the other way
    // round.
    assert_eq!(
        here("content regexp 'SHOUTING'", QueryDialect::Tql),
        BTreeSet::from([block("SHOUTING case")])
    );
    assert!(here("content regexp 'shouting'", QueryDialect::Tql).is_empty());
    // …and so are accents, which `canonical_fold` normalizes.
    assert_eq!(
        here("content regexp 'Uppercase\u{c9}clair'", QueryDialect::Tql),
        BTreeSet::from([block("clair accented")])
    );
    assert!(here("content regexp 'uppercase\u{e9}clair'", QueryDialect::Tql).is_empty());
    // …and whitespace runs, which `searchable_text` collapses.
    assert_eq!(
        here("content regexp 'spaced\\s{3}out'", QueryDialect::Tql),
        BTreeSet::from([block("spaced   out")])
    );
    // Both spellings of the same pattern reach the same rows.
    assert_eq!(
        here("content regexp 'needle alpha'", QueryDialect::Tql),
        here("(content-regex \"needle alpha\")", QueryDialect::Og),
    );
    assert_eq!(
        here("content match '/needle alpha/'", QueryDialect::Tql),
        here("content regexp 'needle alpha'", QueryDialect::Tql),
    );
    // A regex nested under a `children` quantifier, and one composed with a
    // nested `refs` leaf — the two features of this packet in one statement.
    assert!(!here("any(children, content regexp 'nested regex')", QueryDialect::Tql).is_empty());
    for source in [
        "any(children, ref('regex') and content regexp 'nested regex')",
        "not content regexp 'needle'",
        "(and (content-regex \"needle\") (search \"/alpha/\"))",
    ] {
        let dialect = if source.starts_with('(') {
            QueryDialect::Og
        } else {
            QueryDialect::Tql
        };
        assert!(
            !corpus.sql(source, dialect).is_empty(),
            "{source} must match something to be a gate"
        );
        assert_eq!(
            corpus.sql(source, dialect),
            corpus.walk(source, dialect),
            "{source}"
        );
    }

    // **The table is statement-scoped, on a REUSED connection.** Every
    // assertion above already ran several statements through one reader, which
    // is the consecutive-execution case; what is left to prove is that a table
    // cannot outlive its statement. Install one statement's program, then the
    // EMPTY program a regex-free statement installs, and the first statement's
    // ID no longer answers — it FAILS the read rather than matching nothing.
    let (_anchor, statement) = corpus.lower("content regexp 'SHOUTING'", QueryDialect::Tql, true, &[]);
    assert_eq!(statement.regexes.bindings.len(), 1);
    corpus.bind_regexes(&statement.regexes);
    assert_eq!(
        corpus
            .reader
            .run_projection_query(&statement.sql, &statement.params)
            .expect("the installed program answers")
            .len(),
        1
    );
    corpus.bind_regexes(&QueryRegexProgram::default());
    let stale = corpus
        .reader
        .run_projection_query(&statement.sql, &statement.params);
    assert!(
        stale.is_err(),
        "an ID the installed table does not name must fail the read, not match nothing"
    );
    // And an ordinary statement is unaffected by either installation.
    assert_eq!(
        corpus.sql("(task TODO)", QueryDialect::Og),
        corpus.walk("(task TODO)", QueryDialect::Og)
    );
}

/// §5.7's plan gate. **A failing plan gate is information, not an obstacle:**
/// nothing here reclassifies a leaf or relaxes an assertion to make a plan pass.
#[test]
fn a_positively_bounded_query_searches_its_anchor_and_indexes_its_subqueries() {
    let _serial = serialize();
    let root = scratch("plan");
    write_fast_corpus(&root);
    let corpus = Corpus::open(root, true);
    let (failures, vacuous) = measure_plans(&corpus);
    assert!(
        failures.is_empty(),
        "§5.7 plan gate failures:\n{}",
        failures.join("\n")
    );
    // The fast corpus is written so that EVERY plan shape has rows to find. If
    // one folds to the empty statement here, the gate has stopped measuring it
    // and the fixture — not the assertion — is what has to change.
    assert!(
        vacuous.is_empty(),
        "the fast corpus no longer makes these plan shapes satisfiable, so the gate \
         is not measuring them:\n{}",
        vacuous.join("\n")
    );
}

/// The same gate on the anonymized graph, where the row counts are real and the
/// planner's choices are the ones that matter (AGENTS §4 tier 2).
#[test]
#[ignore = "plan gate over a real corpus: set TINE_QUERY_IDENTITY_GRAPH"]
fn the_plan_gate_holds_over_a_real_corpus() {
    let _serial = serialize();
    let Some(root) = std::env::var_os("TINE_QUERY_IDENTITY_GRAPH") else {
        eprintln!("skipped: set TINE_QUERY_IDENTITY_GRAPH to a corpus directory");
        return;
    };
    let corpus = Corpus::open(PathBuf::from(&root), false);
    let (failures, vacuous) = measure_plans(&corpus);
    for (source, dialect) in PLAN_SHAPES {
        let (plan, bounded, nothing) = corpus.explain(source, *dialect);
        let tag = if nothing {
            "vacuous"
        } else if bounded {
            "bounded"
        } else {
            "unbounded"
        };
        eprintln!("plan[{tag}] {source} :: {}", plan.join(" | "));
    }
    // A shape whose predicate is unsatisfiable ON THIS CORPUS is reported, not
    // asserted: the key does not exist here with the type the operator needs, so
    // the statement reads no row and there is no index for SQLite to choose.
    // Every such shape is measured for real on the fast corpus, where it does
    // have rows — this is a property of the graph, not a relaxed gate.
    for line in &vacuous {
        eprintln!("plan[skipped] {line}");
    }
    assert!(
        failures.is_empty(),
        "§5.7 plan gate failures on a real graph:\n{}",
        failures.join("\n")
    );
}

/// **§5.7's table, corrected by measurement.** `(BoundRow::Block, Rel::Children)`
/// used to read "bounded iff the child predicate is", which was true while every
/// child predicate was a property of the CHILD. A nested `refs` is not: it reads
/// the ANCHOR's ancestor closure and the ANCHOR's page, so the child subquery is
/// correlated with the anchor and SQLite has nothing to drive.
///
/// This test records BOTH halves of that — the classification and the plan that
/// forces it — so the entry cannot drift back to a claim the planner refuses.
/// The self-contained child predicate next to it is the control: same relation,
/// same quantifier, still bounded, still index-driven.
#[test]
fn a_nested_refs_child_predicate_cannot_bound_its_anchor() {
    let _serial = serialize();
    let root = scratch("nested-refs-plan");
    write_fast_corpus(&root);
    let corpus = Corpus::open(root, true);

    let (plan, bounded, nothing) = corpus.explain("any(children, ref('Project'))", QueryDialect::Tql);
    assert!(!nothing, "the fixture must make this shape satisfiable");
    assert!(
        !bounded,
        "a child predicate that reads the anchor's context bounds nothing: {}",
        plan.join(" | ")
    );
    // The reason, not just the verdict: the anchor is enumerated and the child
    // subquery is re-run per anchor row.
    assert!(
        plan.iter().any(|step| step == "SCAN b" || step.starts_with("SCAN b ")),
        "the anchor must be the thing being enumerated here: {}",
        plan.join(" | ")
    );
    assert!(
        plan.iter().any(|step| step.contains("CORRELATED")),
        "the child subquery must be correlated with the anchor: {}",
        plan.join(" | ")
    );
    // The three context arms still each reach their own table by key — the
    // correlation is what costs the anchor its probe, not a missing index.
    assert!(
        !plan.iter().any(|step| step.starts_with("SCAN or")
            || step.starts_with("SCAN ar")
            || step.starts_with("SCAN pr")),
        "each ancestor-context arm must still be a keyed probe: {}",
        plan.join(" | ")
    );

    // Control: the same relation and quantifier over a predicate that IS a
    // property of the child alone stays bounded.
    let (control, bounded, nothing) = corpus.explain("any(children, task = 'DONE')", QueryDialect::Tql);
    assert!(!nothing);
    assert!(
        bounded,
        "a self-contained child predicate still bounds its anchor: {}",
        control.join(" | ")
    );
}

/// `(failures, shapes that provably read nothing on this corpus)`.
fn measure_plans(corpus: &Corpus) -> (Vec<String>, Vec<String>) {
    let mut failures = Vec::new();
    let mut vacuous = Vec::new();
    for (source, dialect) in PLAN_SHAPES {
        let (plan, bounded, nothing) = corpus.explain(source, *dialect);
        if nothing {
            vacuous.push(format!(
                "{source}: unsatisfiable on this corpus (the key's effective type \
                 rejects the operator), so the statement reads no row"
            ));
            continue;
        }
        if !bounded {
            failures.push(format!(
                "{source}: §5.7's table calls this bounded and the classifier does not"
            ));
            continue;
        }
        // The anchor alias is `b` for a block row and `p` for a page row; the
        // statement's own FROM decides which. Assert SEARCH on it, never SCAN —
        // the SPELLING of the probe is SQLite's business (a BLOB primary key is
        // reached through `sqlite_autoindex_blocks_1`, never through the words
        // "PRIMARY KEY").
        let anchor = if plan.iter().any(|step| step.contains(" blocks AS b ")) {
            "b"
        } else {
            "p"
        };
        if plan.iter().any(|step| {
            step.starts_with(&format!("SCAN {anchor} ")) || *step == format!("SCAN {anchor}")
        }) {
            failures.push(format!(
                "{source}: the anchor is SCANned: {}",
                plan.join(" | ")
            ));
            continue;
        }
        if !plan
            .iter()
            .any(|step| step.starts_with(&format!("SEARCH {anchor} ")))
        {
            failures.push(format!(
                "{source}: no SEARCH on the anchor alias {anchor}: {}",
                plan.join(" | ")
            ));
            continue;
        }
        // Every relation subquery of a bounded query must reach its own table by
        // an index — a facet-table scan is accepted only where §5.7 says so
        // (`task != 'DONE'` scans the small `tasks` table), and none of the
        // shapes above is one.
        //
        // **The rule this gate enforces is: never SCAN a BASE table where a
        // positive index exists.** Two steps read as `SCAN` and are refused by
        // neither clause of that rule, so each is named here rather than
        // silently tolerated:
        //
        // * `SCAN <alias> VIRTUAL TABLE INDEX …` — FTS5 always spells its own
        //   vtab probe that way. **That step IS the index probe**, not a
        //   base-table enumeration.
        // * `SCAN m` — the materialized match set of §5.3's CTE spelling
        //   ([`ResultSetRule::MatchSetCte`]). **`m` is not a base table**: its
        //   own population is the indexed filter, which this same plan shows
        //   above it, so scanning it enumerates the ANSWER and not the graph.
        //   A `SCAN blocks`/`SCAN pages` is still a failure under either
        //   spelling.
        let scans: Vec<&String> = plan
            .iter()
            .filter(|step| {
                step.starts_with("SCAN ")
                    && !step.starts_with(&format!("SCAN {anchor}"))
                    && !step.contains("VIRTUAL TABLE INDEX")
                    && !(*step == "SCAN m" || step.starts_with("SCAN m "))
            })
            .collect();
        if !scans.is_empty() {
            failures.push(format!(
                "{source}: a bounded relation subquery scans: {}",
                plan.join(" | ")
            ));
        }
    }
    (failures, vacuous)
}

/// The paired-base performance receipt (AGENTS §4): the SAME queries, on the
/// SAME machine, in one session, over the anonymized graph — the walk and the
/// SQL path side by side. A synthetic page cannot answer "is this faster on my
/// graph", so this reports only what it measured and on which corpus.
#[test]
#[ignore = "paired-base performance receipt: set TINE_QUERY_IDENTITY_GRAPH"]
fn the_walk_and_the_lowering_are_timed_against_each_other_on_a_real_corpus() {
    let _serial = serialize();
    let Some(root) = std::env::var_os("TINE_QUERY_IDENTITY_GRAPH") else {
        eprintln!("skipped: set TINE_QUERY_IDENTITY_GRAPH to a corpus directory");
        return;
    };
    let corpus = Corpus::open(PathBuf::from(&root), false);
    const REPEATS: u32 = 5;
    eprintln!("paired_base_query_receipt corpus=real repeats={REPEATS}");
    for (source, dialect) in PLAN_SHAPES {
        measure_shape(&corpus, source, *dialect, REPEATS, true);
    }
    // §5.10's three not-index-bounded classes are reported SEPARATELY from the
    // indexed case, and the transient class is measured on the SAME projection
    // rather than on a second corpus, so the two numbers are comparable.
    eprintln!("paired_base_content_receipt corpus=real repeats={REPEATS}");
    for (source, dialect) in CONTENT_SHAPES {
        measure_shape(&corpus, source, *dialect, REPEATS, true);
        measure_shape(&corpus, source, *dialect, REPEATS, false);
    }
}

/// **Policy question 1, settled by measurement.** §5.3's result-set rule has two
/// spellings; P1-b measured the correlated one at 5.4× SLOWER than the walk on
/// `any(children, task = 'DONE')` and showed it was not a plan defect. The rule
/// the dossier fixes in advance, so the number decides and not the argument:
/// adopt the CTE spelling iff it is faster on `any(children, task = 'DONE')` AND
/// regresses no other `PLAN_SHAPES` entry by more than 10%.
///
/// The verdict this printed is recorded in P1-d's receipt and pinned in code by
/// [`RESULT_SET_RULE`]; rerunning this test is how it stays falsifiable.
#[test]
#[ignore = "result-set-rule decision table: set TINE_QUERY_IDENTITY_GRAPH"]
fn the_two_result_set_spellings_are_timed_against_each_other_on_a_real_corpus() {
    let _serial = serialize();
    let Some(root) = std::env::var_os("TINE_QUERY_IDENTITY_GRAPH") else {
        eprintln!("skipped: set TINE_QUERY_IDENTITY_GRAPH to a corpus directory");
        return;
    };
    let corpus = Corpus::open(PathBuf::from(&root), false);
    // The dossier's protocol says five repeats; the default here is 51, and
    // `TINE_RESULT_SET_RULE_REPEATS=5` reproduces the protocol exactly.
    //
    // More samples were tried because five made the printed VERDICT a coin
    // flip, and they did not fix it — which is itself the finding. Over eight
    // independent sessions the decisive shape is stable at 0.54-0.56x, while
    // the "no entry regresses past 10%" half of the rule is decided by
    // sub-millisecond shapes: `deadline is not null` (~100-125 µs) reported
    // 0.89/0.91/0.93/1.01/1.09/1.10/1.14/1.15, and twice a shape with an EMPTY
    // result set "regressed" on a 1-3 µs difference (7 -> 8 µs, 8 -> 11 µs).
    // The verdict logic below is deliberately left EXACTLY as the dossier wrote
    // it, unflattered, so that what it prints is the literal rule's answer and
    // not a threshold moved until it agreed with the code. See P1-d's receipt:
    // adopting the materialized spelling is a recorded lane decision on the
    // decisive shape's 2.3 ms saving, NOT a clean pass of both halves.
    let repeats: u32 = std::env::var("TINE_RESULT_SET_RULE_REPEATS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(51)
        .max(5);
    let fts_ready = corpus.fts_ready();
    eprintln!("result_set_rule_decision corpus=real repeats={repeats}");
    let mut regressions = Vec::new();
    let mut decisive: Option<(u128, u128)> = None;
    for (source, dialect) in PLAN_SHAPES {
        let Some(probe) = time_rule(
            &corpus,
            source,
            *dialect,
            fts_ready,
            repeats,
            ResultSetRule::CorrelatedProbe,
        ) else {
            continue;
        };
        let Some(cte) = time_rule(
            &corpus,
            source,
            *dialect,
            fts_ready,
            repeats,
            ResultSetRule::MatchSetCte,
        ) else {
            continue;
        };
        let mat = time_rule(
            &corpus,
            source,
            *dialect,
            fts_ready,
            repeats,
            ResultSetRule::MatchSetCteMaterialized,
        );
        let ratio = cte.1 as f64 / probe.1.max(1) as f64;
        let mat_ratio = mat.map_or(f64::NAN, |mat| mat.1 as f64 / probe.1.max(1) as f64);
        eprintln!(
            "result_set_rule shape={source:?} rows={} probe_us={} cte_us={} mat_us={} \
             cte/probe={ratio:.2} mat/probe={mat_ratio:.2}",
            probe.0,
            probe.1,
            cte.1,
            mat.map_or(0, |mat| mat.1)
        );
        // The candidate is the SINGLE-EVALUATION spelling, which on SQLite means
        // the materialized one: an ordinary CTE is inlined and still evaluates
        // the filter twice, which is why both are timed and only one of them can
        // answer the policy question.
        let Some(candidate) = mat else { continue };
        if *source == "any(children, task = \'DONE\')" {
            decisive = Some((probe.1, candidate.1));
        } else if mat_ratio > 1.10 {
            regressions.push(format!(
                "{source}: {mat_ratio:.2}× (probe {} → materialized {})",
                probe.1, candidate.1
            ));
        }
    }
    let verdict = match decisive {
        Some((probe, candidate)) if candidate < probe && regressions.is_empty() => {
            "ADOPT MatchSetCteMaterialized"
        }
        Some((probe, candidate)) if candidate >= probe => {
            "KEEP CorrelatedProbe (the decisive shape did not improve)"
        }
        Some(_) => "KEEP CorrelatedProbe (another PLAN_SHAPES entry regressed >10%)",
        None => "INCONCLUSIVE (the decisive shape did not lower on this corpus)",
    };
    eprintln!("result_set_rule verdict={verdict} in_code={RESULT_SET_RULE:?}");
    for line in &regressions {
        eprintln!("result_set_rule regression {line}");
    }
}

/// **The paired-base perf receipt, extended with §5.9's DISPATCHED path.**
///
/// The sibling above times the COMPILER against the walk at the gate boundary:
/// it stops at the block ids the statement returns. This one times what a USER
/// waits for — `Graph::run_query_bounded`, the production entry point, which
/// lowers, runs the statement, loads exactly the result's pages, and builds the
/// same DTO rows the walk builds — against `query::run_query_bounded`, the pure
/// walk, on the same graph in the same session.
///
/// It also asserts the two are byte-identical INCLUDING order on every shape it
/// times, which is the ordered identity comparison on a real corpus rather than
/// on a fixture: `signature` here is the serialized result, so a reordered page
/// list or a reordered block within one page fails it.
///
/// Both memos are dropped before each dispatched sample, so this compares two
/// computations and never a computation against a cache hit.
#[test]
#[ignore = "paired walk/dispatch receipt: set TINE_QUERY_IDENTITY_GRAPH"]
fn the_walk_and_the_dispatch_are_timed_against_each_other_on_a_real_corpus() {
    let _serial = serialize();
    let Some(root) = std::env::var_os("TINE_QUERY_IDENTITY_GRAPH") else {
        eprintln!("skipped: set TINE_QUERY_IDENTITY_GRAPH to a corpus directory");
        return;
    };
    let corpus = Corpus::open(PathBuf::from(&root), false);
    const REPEATS: u32 = 5;
    eprintln!("dispatch_paired corpus=real repeats={REPEATS}");
    let mut disagreements = Vec::new();
    for (source, dialect) in PLAN_SHAPES {
        // The production entry point parses OG; a TQL shape reaches the same IR
        // through a different door and is timed by the compiler-level sibling.
        if *dialect != QueryDialect::Og {
            continue;
        }
        let walk = crate::query::run_query_bounded(&corpus.graph, source, usize::MAX, usize::MAX);
        let walk_us = {
            let start = Instant::now();
            for _ in 0..REPEATS {
                std::hint::black_box(crate::query::run_query_bounded(
                    &corpus.graph,
                    source,
                    usize::MAX,
                    usize::MAX,
                ));
            }
            (start.elapsed() / REPEATS).as_micros()
        };
        corpus.graph.clear_query_memos_test();
        let dispatched = corpus
            .graph
            .run_query_bounded(source, usize::MAX, usize::MAX);
        let dispatch_us = {
            let start = Instant::now();
            for _ in 0..REPEATS {
                corpus.graph.clear_query_memos_test();
                std::hint::black_box(corpus.graph.run_query_bounded(
                    source,
                    usize::MAX,
                    usize::MAX,
                ));
            }
            (start.elapsed() / REPEATS).as_micros()
        };
        let rows: usize = walk.groups.iter().map(|group| group.blocks.len()).sum();
        let ordered_equal = serde_json::to_vec(dispatched.groups.as_ref()).unwrap()
            == serde_json::to_vec(&walk.groups).unwrap()
            && (dispatched.total, dispatched.exceeded) == (walk.total, walk.exceeded);
        if !ordered_equal {
            disagreements.push(format!(
                "{source}: walk {} pages/{rows} rows vs dispatch {} pages",
                walk.groups.len(),
                dispatched.groups.len()
            ));
        }
        eprintln!(
            "dispatch_paired shape={source:?} pages={} rows={rows} walk_us={walk_us} \
             dispatch_us={dispatch_us} dispatch/walk={:.2} ordered_equal={ordered_equal}",
            walk.groups.len(),
            dispatch_us as f64 / walk_us.max(1) as f64,
        );
    }
    assert!(
        disagreements.is_empty(),
        "the dispatched result must equal the walk's INCLUDING order:\n{}",
        disagreements.join("\n")
    );
}

/// One `(rows, µs)` pair for one shape under one spelling, warmed once.
fn time_rule(
    corpus: &Corpus,
    source: &str,
    dialect: QueryDialect,
    fts_ready: bool,
    repeats: u32,
    rule: ResultSetRule,
) -> Option<(usize, u128)> {
    let first = corpus.sql_as(source, dialect, fts_ready, &[], rule);
    let start = Instant::now();
    for _ in 0..repeats {
        let _ = corpus.sql_as(source, dialect, fts_ready, &[], rule);
    }
    Some((first.len(), (start.elapsed() / repeats).as_micros()))
}

/// One paired-base line: the walk and the SQL path for the same query, on the
/// same corpus, in the same session. `plan` names §5.10's class so an
/// unindexable or transient path is never read as the indexed one.
fn measure_shape(
    corpus: &Corpus,
    source: &str,
    dialect: QueryDialect,
    repeats: u32,
    fts_ready: bool,
) {
    // Warm both sides once so neither pays for the other's first-touch cost.
    let first = corpus.sql_with(source, dialect, fts_ready, &[]);
    let _ = corpus.walk(source, dialect);
    let walk_start = Instant::now();
    for _ in 0..repeats {
        let _ = corpus.walk(source, dialect);
    }
    let walk = walk_start.elapsed() / repeats;
    let sql_start = Instant::now();
    for _ in 0..repeats {
        let _ = corpus.sql_with(source, dialect, fts_ready, &[]);
    }
    let sql = sql_start.elapsed() / repeats;
    let (_anchor, statement) = corpus.lower(source, dialect, fts_ready, &[]);
    let plan = if statement.content_plans.is_empty() {
        if statement.positively_bounded {
            "indexed".to_string()
        } else {
            "unbounded".to_string()
        }
    } else {
        format!("{:?}", statement.content_plans)
    };
    eprintln!(
        "paired_base shape={source:?} plan={plan} bounded={} rows={} walk_us={} sql_us={}",
        statement.positively_bounded,
        first.len(),
        walk.as_micros(),
        sql.as_micros()
    );
}

// ---------------------------------------------------------------------------
// DB1: the before/after statement measurement
// ---------------------------------------------------------------------------

/// The shapes the before/after measurement times. Chosen so the comparison is
/// not made on empty results alone: broad shapes (most of the graph matches),
/// selective ones (a handful of rows), one page-anchored shape whose statement
/// this packet does not touch at all, and one that is empty on most corpora.
const MEASURE_SHAPES: &[(&str, QueryDialect)] = &[
    // broad — the candidate stage is where the removed page decoration cost
    // whatever it cost, so a shape with many candidates has to be in the list.
    ("(task TODO)", QueryDialect::Og),
    ("task is not null", QueryDialect::Tql),
    ("(journal)", QueryDialect::Og),
    ("content match 'the'", QueryDialect::Tql),
    // selective
    ("[[Project]]", QueryDialect::Og),
    ("(priority A)", QueryDialect::Og),
    ("scheduled is not null", QueryDialect::Tql),
    ("(property status open)", QueryDialect::Og),
    ("any(children, task = 'DONE')", QueryDialect::Tql),
    // the control shape of the packet: `@page` output is unchanged, so this one
    // must measure as noise and its two statements must be byte-identical.
    ("@page and day >= '2026-01-01'", QueryDialect::Tql),
    // typically empty
    ("prop('k') = 'a'", QueryDialect::Tql),
];

/// Where the captured baseline statements live. The measurement reads it; the
/// dump below writes it.
fn baseline_statements_path() -> Option<PathBuf> {
    std::env::var_os("TINE_QUERY_STATEMENT_BASELINE").map(PathBuf::from)
}

/// **Capture the statements this compiler emits today**, so a later build can
/// be timed against them without keeping a second production compiler alive
/// merely to benchmark it (the artifact is the "before", not a code path).
///
/// Run this at the base commit, then edit, then run the measurement below.
#[test]
#[ignore = "capture a measurement baseline: set TINE_QUERY_IDENTITY_GRAPH and TINE_QUERY_STATEMENT_BASELINE"]
fn dump_the_lowered_statements_as_a_measurement_baseline() {
    let _serial = serialize();
    let (Some(root), Some(out)) = (
        std::env::var_os("TINE_QUERY_IDENTITY_GRAPH"),
        baseline_statements_path(),
    ) else {
        eprintln!("skipped: set TINE_QUERY_IDENTITY_GRAPH and TINE_QUERY_STATEMENT_BASELINE");
        return;
    };
    let corpus = Corpus::open(PathBuf::from(&root), false);
    let fts_ready = corpus.fts_ready();
    let mut captured: std::collections::BTreeMap<String, String> =
        std::collections::BTreeMap::new();
    for (source, dialect) in MEASURE_SHAPES {
        let (_anchor, statement) = corpus.lower(source, *dialect, fts_ready, &[]);
        captured.insert((*source).to_string(), statement.sql);
    }
    std::fs::write(
        &out,
        serde_json::to_string_pretty(&captured).expect("the capture serializes"),
    )
    .expect("the baseline artifact is writable");
    eprintln!(
        "statement_baseline shapes={} path={}",
        captured.len(),
        out.display()
    );
}

/// One arm's samples for one shape.
struct Arm {
    label: &'static str,
    sql: String,
    samples: Vec<u128>,
    identities: BTreeSet<String>,
    columns: usize,
}

/// **The before/after measurement (DB1).** The captured baseline statement and
/// the statement this build emits are run on the SAME projection, in ONE
/// process, alternating which goes first across the rounds, with a third arm
/// that is the baseline statement AGAIN — the control. The control's distance
/// from the baseline is the noise floor this machine can resolve; a
/// before/after difference inside it is reported as inconclusive rather than as
/// a win.
///
/// Identity is compared per shape (the anchor column of every returned row), so
/// a statement that got faster by answering a different question fails here.
#[test]
#[ignore = "before/after statement measurement: set TINE_QUERY_IDENTITY_GRAPH and TINE_QUERY_STATEMENT_BASELINE"]
fn the_baseline_and_current_statements_are_measured_against_each_other() {
    let _serial = serialize();
    let (Some(root), Some(baseline_path)) = (
        std::env::var_os("TINE_QUERY_IDENTITY_GRAPH"),
        baseline_statements_path(),
    ) else {
        eprintln!("skipped: set TINE_QUERY_IDENTITY_GRAPH and TINE_QUERY_STATEMENT_BASELINE");
        return;
    };
    let baseline: std::collections::BTreeMap<String, String> = serde_json::from_str(
        &std::fs::read_to_string(&baseline_path).expect("the baseline artifact is readable"),
    )
    .expect("the baseline artifact parses");
    let corpus = Corpus::open(PathBuf::from(&root), false);
    let fts_ready = corpus.fts_ready();
    // Nine is the floor the packet sets; more rounds cost milliseconds here and
    // narrow the control spread, which is the only thing that decides whether a
    // difference is reportable at all.
    let rounds: u32 = std::env::var("TINE_STATEMENT_MEASURE_ROUNDS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(21)
        .max(9);
    // Each sample runs the statement this many times, so a sub-timer-resolution
    // shape is still measured rather than rounded to zero.
    let inner: u32 = std::env::var("TINE_STATEMENT_MEASURE_INNER")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(5)
        .max(1);
    eprintln!("statement_measure corpus=real rounds={rounds} inner={inner}");
    let mut disagreements = Vec::new();
    for (source, dialect) in MEASURE_SHAPES {
        let Some(before_sql) = baseline.get(*source) else {
            disagreements.push(format!("{source}: absent from the captured baseline"));
            continue;
        };
        let (anchor, statement) = corpus.lower(source, *dialect, fts_ready, &[]);
        corpus.bind_regexes(&statement.regexes);
        // The two statements bind the SAME values in the SAME order — this
        // packet changes projection lists and one join, never a bound value —
        // and this check is what makes reusing today's parameters for the
        // captured statement safe rather than assumed.
        assert_eq!(
            highest_placeholder(before_sql),
            statement.params.len(),
            "{source}: the captured statement binds a different number of values"
        );
        let params = statement.params.clone();
        let mut arms = [
            Arm {
                label: "before",
                sql: before_sql.clone(),
                samples: Vec::new(),
                identities: BTreeSet::new(),
                columns: 0,
            },
            Arm {
                label: "after",
                sql: statement.sql.clone(),
                samples: Vec::new(),
                identities: BTreeSet::new(),
                columns: 0,
            },
            Arm {
                label: "control",
                sql: before_sql.clone(),
                samples: Vec::new(),
                identities: BTreeSet::new(),
                columns: 0,
            },
        ];
        // Warm every arm once so none of them pays another's first-touch cost.
        for arm in arms.iter_mut() {
            let (identities, columns) = run_arm(&corpus, &arm.sql, &params, anchor);
            arm.identities = identities;
            arm.columns = columns;
        }
        for round in 0..rounds {
            // Alternate the order every round: three arms, rotated, so no arm
            // sits permanently in the warmest or the coldest slot.
            let order: [usize; 3] = match round % 3 {
                0 => [0, 1, 2],
                1 => [1, 2, 0],
                _ => [2, 0, 1],
            };
            for index in order {
                let start = Instant::now();
                for _ in 0..inner {
                    let rows = corpus
                        .reader
                        .run_projection_query(&arms[index].sql, &params)
                        .expect("the measured statement runs");
                    std::hint::black_box(rows.len());
                }
                arms[index]
                    .samples
                    .push((start.elapsed() / inner).as_micros());
            }
        }
        for arm in arms.iter_mut() {
            arm.samples.sort_unstable();
        }
        let before = median(&arms[0].samples);
        let after = median(&arms[1].samples);
        let control = median(&arms[2].samples);
        if arms[0].identities != arms[1].identities {
            disagreements.push(format!(
                "{source}: before returned {} rows and after {}",
                arms[0].identities.len(),
                arms[1].identities.len()
            ));
        }
        // The control is the same bytes as `before`, so its distance from
        // `before` is what this machine cannot tell apart. One microsecond is
        // added for the timer itself.
        let noise = control.abs_diff(before).max(1);
        let delta = after.abs_diff(before);
        let verdict = if delta <= noise {
            "inconclusive"
        } else if after < before {
            "faster"
        } else {
            "SLOWER"
        };
        eprintln!(
            "statement_measure shape={source:?} rows={} cols_before={} cols_after={} \
             before_us={before} after_us={after} control_us={control} \
             after/before={:.3} control/before={:.3} verdict={verdict}",
            arms[1].identities.len(),
            arms[0].columns,
            arms[1].columns,
            after as f64 / before.max(1) as f64,
            control as f64 / before.max(1) as f64,
        );
    }
    assert!(
        disagreements.is_empty(),
        "the captured and the current statement must answer identically:\n{}",
        disagreements.join("\n")
    );
}

/// The anchor identities one statement returns, and how many columns its rows
/// carry. No corpus text reaches the caller: block ids are UUIDs and a page's
/// identity is taken as its normalized key.
fn run_arm(
    corpus: &Corpus,
    sql: &str,
    params: &[PhysicalQueryValue],
    anchor: Anchor,
) -> (BTreeSet<String>, usize) {
    let rows = corpus
        .reader
        .run_projection_query(sql, params)
        .unwrap_or_else(|error| panic!("the measured statement must run: {error}\n{sql}"));
    let columns = rows.first().map_or(0, Vec::len);
    let identities = rows
        .into_iter()
        .map(|row| match (anchor, row.first()) {
            (Anchor::Block, Some(PhysicalQueryValue::Blob(id))) => Uuid::from_slice(id)
                .expect("a 16-byte block id")
                .to_string(),
            (Anchor::Page, Some(PhysicalQueryValue::Blob(id))) => {
                Uuid::from_slice(id).expect("a 16-byte page id").to_string()
            }
            (_, other) => panic!("the anchor column is an identity, got {other:?}"),
        })
        .collect();
    (identities, columns)
}

/// The largest `?n` in a statement — the number of values it binds.
fn highest_placeholder(sql: &str) -> usize {
    let mut highest = 0usize;
    let mut rest = sql;
    while let Some(at) = rest.find('?') {
        rest = &rest[at + 1..];
        let end = rest
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(rest.len());
        if let Ok(index) = rest[..end].parse::<usize>() {
            highest = highest.max(index);
        }
        rest = &rest[end..];
    }
    highest
}

fn median(sorted: &[u128]) -> u128 {
    if sorted.is_empty() {
        return 0;
    }
    sorted[sorted.len() / 2]
}

/// **DB1's guard: an ordinary block query decorates nothing.**
///
/// Two obligations, both of which the previous compiler failed:
///
/// * the block row is `(block_id, page_id, path)` — three columns. The retired
///   `pages.name` and `pages.text_kind` were carried to no consumer at all;
/// * the match set is populated `FROM blocks b`, with no unconditional join to
///   `pages`. That join probed the pages primary key once per CANDIDATE row —
///   before matching and before parent suppression — to fetch columns the match
///   never reads. The one remaining `JOIN pages` routes the ANSWER to its path.
///
/// This is not a statement-style assertion: it pins the exact unnecessary work,
/// and the executed rows are read back through the seam so the shape is proven
/// on real rows and not only in the emitted text.
#[test]
fn a_block_query_selects_three_columns_and_never_decorates_its_candidates() {
    let _serial = serialize();
    let root = scratch("no-candidate-decoration");
    write_fast_corpus(&root);
    let corpus = Corpus::open(root, true);
    let fts_ready = corpus.fts_ready();

    // A shape with a page predicate is deliberately included: its `pages`
    // access must be its OWN scoped subquery, so removing the candidate-stage
    // join cannot have moved a page predicate's table access anywhere.
    for source in ["(task TODO)", "(journal)", "[[Project]]"] {
        let (anchor, statement) = corpus.lower(source, QueryDialect::Og, fts_ready, &[]);
        assert_eq!(anchor, Anchor::Block, "{source}");
        assert!(
            statement.sql.starts_with(
                "WITH m(block_id, page_id, parent_block_id) AS MATERIALIZED \
                 (SELECT b.block_id, b.page_id, b.parent_block_id FROM blocks b WHERE "
            ),
            "the match set reads blocks alone: {}",
            statement.sql
        );
        assert!(
            statement.sql.contains(
                "SELECT m.block_id, m.page_id, p.path \
                 FROM m JOIN pages p ON p.page_id = m.page_id WHERE "
            ),
            "the answer is routed to its path once, and carries nothing else: {}",
            statement.sql
        );
        assert_eq!(
            statement.sql.matches("JOIN pages").count(),
            1,
            "exactly one join to pages, and it is the routing one: {}",
            statement.sql
        );
        assert!(
            !statement.sql.contains("p.name") && !statement.sql.contains("p.text_kind"),
            "no page decoration survives in a block statement: {}",
            statement.sql
        );

        // The rows themselves, through the seam.
        let rows = corpus
            .reader
            .run_projection_query(&statement.sql, &statement.params)
            .expect("the guarded statement runs");
        assert!(!rows.is_empty(), "{source} must match rows to be a guard");
        for row in &rows {
            assert_eq!(row.len(), 3, "{source}: {row:?}");
            match (&row[0], &row[1], &row[2]) {
                (
                    PhysicalQueryValue::Blob(block_id),
                    PhysicalQueryValue::Blob(page_id),
                    PhysicalQueryValue::Text(path),
                ) => {
                    assert_eq!(block_id.len(), 16, "{source}");
                    assert_eq!(page_id.len(), 16, "{source}");
                    assert!(path.ends_with(".md"), "{source}");
                }
                other => panic!("{source}: the row contract is (blob, blob, text): {other:?}"),
            }
        }
    }

    // The correlated spelling keeps the same three columns — the packet removed
    // the unused fields from the OUTPUT, which is not a property of one
    // result-set spelling.
    let (_anchor, probe) = corpus.lower_as(
        "(task TODO)",
        QueryDialect::Og,
        fts_ready,
        &[],
        ResultSetRule::CorrelatedProbe,
    );
    assert!(
        probe
            .sql
            .starts_with("SELECT b.block_id, b.page_id, p.path FROM blocks b JOIN pages p "),
        "{}",
        probe.sql
    );
    assert!(
        !probe.sql.contains("p.name") && !probe.sql.contains("p.text_kind"),
        "{}",
        probe.sql
    );

    // `@page` output is untouched by this packet: four columns, name and kind
    // included, because the page result construction reads them.
    let (anchor, page) = corpus.lower("@page and journal = true", QueryDialect::Tql, fts_ready, &[]);
    assert_eq!(anchor, Anchor::Page);
    assert!(
        page.sql
            .starts_with("SELECT p.page_id, p.name, p.text_kind, p.journal_day FROM pages p "),
        "{}",
        page.sql
    );
    let page_rows = corpus
        .reader
        .run_projection_query(&page.sql, &page.params)
        .expect("the page statement runs");
    assert!(!page_rows.is_empty());
    for row in &page_rows {
        assert_eq!(row.len(), 4, "the page row shape is unchanged: {row:?}");
    }
}
