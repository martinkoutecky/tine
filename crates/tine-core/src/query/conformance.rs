//! The §3.4 truth table, evaluated end to end by the walk.
//!
//! Every leaf is two-valued: an absent optional attribute makes any comparison
//! on it false, an atom that does not coerce fails the comparison, `Any` over an
//! empty relation is false and `Every` over an empty relation is true. The table
//! below is one graph plus one row per (filter, row) case; the expectation is
//! the set of blocks the query returns.
//!
//! SPEC §3.4 asks for this table to be evaluated **by the walk and by the
//! lowering**, asserting identical truth values. The lowering is P1; the walk
//! half is here, and the lowering must adopt this same table rather than write
//! a second one.

use std::fs;
use std::path::PathBuf;

use crate::date::JournalDate;
use crate::model::Graph;
use crate::query::atom::CompareMode;
use crate::query::ir::{Anchor, Bounds, QueryRows};
use crate::query::{
    parse_query_text, print::print_tql, run_query_bounded, run_query_result, QueryDialect,
};

/// One page of the truth-table graph, written verbatim so the fixture is
/// readable as Logseq Markdown (I-4).
struct Page {
    path: &'static str,
    text: &'static str,
}

/// The rows the table quantifies over. Every optional block attribute appears
/// absent and present; `children` appears empty, non-empty-all-satisfying and
/// non-empty-mixed; a property appears absent, present-and-empty, present with a
/// coercible atom and present with an uncoercible one.
const PAGES: &[Page] = &[
    Page {
        path: "pages/Rows.md",
        // A marker with neither priority nor planning.
        text: "\
- TODO plain marker
- [#A] priority without a marker
- SCHEDULED: <2026-07-29 Wed>
- DEADLINE: <2026-08-01 Sat>
- TODO marked and scheduled
  SCHEDULED: <2026-07-30 Thu>
- bare SCHEDULED: with no date
- `SCHEDULED: <2026-07-29 Wed>` inside inline code
- malformed planning
  SCHEDULED: <2026-13-45 Xxx>
",
    },
    Page {
        path: "pages/Children.md",
        text: "\
- leaf parent
- all children done
\t- DONE one
\t- DONE two
- mixed children
\t- DONE one
\t- TODO two
- TODO root that violates the child predicate
",
    },
    Page {
        path: "pages/Props.md",
        text: "\
- absent property
- blank property
  size::
- numeric property
  size:: 5
- text property
  size:: large
- mixed numeric and text
  size:: 5, large
",
    },
    Page {
        path: "pages/Names.md",
        text: "- a block on a named page\n",
    },
    Page {
        path: "pages/Tagged.md",
        text: "tags:: alpha, beta\n\n- a block on a tagged page\n",
    },
    Page {
        path: "pages/Proj%2FSub.md",
        text: "- a block under the namespace\n",
    },
];

const JOURNAL: Page = Page {
    path: "journals/2026_07_29.md",
    text: "- a journal block\n",
};

fn graph(label: &str) -> (Graph, PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "tine-query-conformance-{label}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("pages")).expect("pages");
    fs::create_dir_all(dir.join("journals")).expect("journals");
    for page in PAGES.iter().chain(std::iter::once(&JOURNAL)) {
        fs::write(dir.join(page.path), page.text).expect("page");
    }
    let graph = Graph::open(&dir);
    graph.warm_cache();
    (graph, dir)
}

/// The first line of every block the query returns, sorted, so a case reads as
/// the set of rows for which the leaf is true.
fn rows(graph: &Graph, query: &str) -> Vec<String> {
    let bounded = run_query_bounded(graph, query, usize::MAX, usize::MAX);
    let mut out: Vec<String> = bounded
        .groups
        .iter()
        .flat_map(|group| group.blocks.iter())
        .map(|block| {
            block
                .raw
                .lines()
                .next()
                .unwrap_or_default()
                .trim()
                .to_string()
        })
        .collect();
    out.sort();
    out
}

fn case(graph: &Graph, query: &str, expected: &[&str]) {
    let mut expected: Vec<String> = expected.iter().map(|text| text.to_string()).collect();
    expected.sort();
    assert_eq!(rows(graph, query), expected, "{query}");
}

#[test]
fn optional_block_attributes_are_two_valued() {
    let (graph, dir) = graph("attrs");
    // Presence and absence of `task`.
    case(
        &graph,
        "(task TODO)",
        &[
            "TODO plain marker",
            "TODO marked and scheduled",
            "TODO root that violates the child predicate",
            "TODO two",
        ],
    );
    // `[#A]` without a marker is still a priority row (§3.2 parity fixture).
    case(&graph, "(priority A)", &["[#A] priority without a marker"]);
    // **Presence IS a projected timestamp (G2).** The bare `SCHEDULED:` with no
    // date and the one inside inline code are NOT matches; the malformed date
    // has presence and no day (E1).
    case(
        &graph,
        "(scheduled)",
        &[
            "SCHEDULED: <2026-07-29 Wed>",
            "TODO marked and scheduled",
            "malformed planning",
        ],
    );
    case(&graph, "(deadline)", &["DEADLINE: <2026-08-01 Sat>"]);
    // A day comparison drops the malformed row: presence without a day.
    case(
        &graph,
        "(between scheduled 2026-07-29 2026-07-31)",
        &["SCHEDULED: <2026-07-29 Wed>", "TODO marked and scheduled"],
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn property_leaves_are_two_valued_over_atoms() {
    let (graph, dir) = graph("props");
    // Presence: the blank value has presence and zero atoms.
    case(
        &graph,
        "(property size)",
        &[
            "blank property",
            "numeric property",
            "text property",
            "mixed numeric and text",
        ],
    );
    // An atom equality reaches only the rows with that atom.
    case(
        &graph,
        "(property size 5)",
        &["numeric property", "mixed numeric and text"],
    );
    // Absent → false, never "unknown".
    case(&graph, "(property missing anything)", &[]);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn a_leaf_block_satisfies_every_over_its_empty_children() {
    let (graph, dir) = graph("children");
    // OData: `Any` is false and `Every` true on an empty collection (Q5). The
    // root block that VIOLATES the predicate sits in the same graph, which is
    // the J1 guard: it must not make every other row's `every` false.
    let (query, view) = parse_query_text(
        "every(children, task = 'DONE')",
        QueryDialect::Tql,
        JournalDate::today(),
    );
    assert!(!query.is_invalid(), "{:?}", query.diagnostics);
    // Printed and re-parsed on purpose: the conformance case is the ROUND-TRIP
    // of the filter, not just the parse.
    let result = run_query_result(
        &graph,
        &print_tql(&query),
        QueryDialect::Tql,
        Bounds::unbounded(),
    );
    let QueryRows::Block { groups } = &result.rows else {
        panic!("a block-anchored query returns block rows");
    };
    let mut matched: Vec<String> = groups
        .iter()
        .flat_map(|group| group.blocks.iter())
        .map(|block| {
            block
                .raw
                .lines()
                .next()
                .unwrap_or_default()
                .trim()
                .to_string()
        })
        .collect();
    matched.sort();
    assert!(
        matched.contains(&"leaf parent".to_string()),
        "a leaf block satisfies `every` over its empty children: {matched:?}"
    );
    assert!(
        matched.contains(&"all children done".to_string()),
        "every child satisfies the predicate: {matched:?}"
    );
    assert!(
        !matched.contains(&"mixed children".to_string()),
        "one violating child falsifies `every`: {matched:?}"
    );
    let _ = view;
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn any_over_children_is_false_on_a_leaf_block() {
    let (graph, dir) = graph("any-children");
    let result = run_query_result(
        &graph,
        "any(children, task = 'TODO')",
        QueryDialect::Tql,
        Bounds::unbounded(),
    );
    let QueryRows::Block { groups } = &result.rows else {
        panic!("a block-anchored query returns block rows");
    };
    let matched: Vec<String> = groups
        .iter()
        .flat_map(|group| group.blocks.iter())
        .map(|block| {
            block
                .raw
                .lines()
                .next()
                .unwrap_or_default()
                .trim()
                .to_string()
        })
        .collect();
    assert_eq!(matched, vec!["mixed children".to_string()], "{matched:?}");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn the_page_anchor_returns_page_rows_and_reads_no_document() {
    let (graph, dir) = graph("page-anchor");
    let (query, view) = parse_query_text(
        "@page and name like 'proj/%'",
        QueryDialect::Tql,
        JournalDate::today(),
    );
    assert_eq!(query.anchor, Anchor::Page);
    let result = crate::query::run_query_result_over(
        &crate::query::GraphQueryPages(&graph),
        &query,
        &view,
        Bounds::unbounded(),
    );
    let QueryRows::Page { pages } = &result.rows else {
        panic!("a page-anchored query returns page rows");
    };
    let names: Vec<&str> = pages.iter().map(|page| page.name.as_str()).collect();
    assert_eq!(names, vec!["Proj/Sub"], "{pages:?}");
    // `(namespace Proj)` must not match the namespace's own page: the boundary
    // slash is part of the prefix, not a separator the key fold may drop.
    let (parent, view) = parse_query_text(
        "@page and name like 'proj%'",
        QueryDialect::Tql,
        JournalDate::today(),
    );
    let result = crate::query::run_query_result_over(
        &crate::query::GraphQueryPages(&graph),
        &parent,
        &view,
        Bounds::unbounded(),
    );
    let QueryRows::Page { pages } = &result.rows else {
        panic!("page rows");
    };
    assert_eq!(pages.len(), 1, "{pages:?}");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn the_journal_page_attribute_is_a_page_row_leaf() {
    let (graph, dir) = graph("journal");
    let (query, view) = parse_query_text(
        "@page and journal = true",
        QueryDialect::Tql,
        JournalDate::today(),
    );
    let result = crate::query::run_query_result_over(
        &crate::query::GraphQueryPages(&graph),
        &query,
        &view,
        Bounds::unbounded(),
    );
    let QueryRows::Page { pages } = &result.rows else {
        panic!("page rows");
    };
    let names: Vec<&str> = pages.iter().map(|page| page.name.as_str()).collect();
    assert_eq!(names, vec!["Jul 29th, 2026"], "{pages:?}");
    let _ = fs::remove_dir_all(dir);
}

/// The OG and TQL spellings of one filter must return the same rows. This is
/// I-12 stated as a test: two surfaces, one answer.
#[test]
fn the_two_dialects_agree_row_for_row() {
    let (graph, dir) = graph("dialects");
    for (og, tql) in [
        ("(task TODO)", "task = 'TODO'"),
        ("(priority A)", "priority = 'A'"),
        ("(property size 5)", "prop('size') = '5'"),
        ("(page Names)", "page.name = 'Names'"),
        ("(namespace Proj)", "page.name like 'proj/%'"),
        ("(scheduled)", "scheduled is not null"),
        ("(deadline)", "deadline is not null"),
    ] {
        let (query, view) = parse_query_text(tql, QueryDialect::Tql, JournalDate::today());
        assert!(!query.is_invalid(), "{tql}: {:?}", query.diagnostics);
        let tql_rows = {
            let result = crate::query::run_query_result_over(
                &crate::query::GraphQueryPages(&graph),
                &query,
                &view,
                Bounds::unbounded(),
            );
            let QueryRows::Block { groups } = result.rows else {
                panic!("{tql} is block-anchored");
            };
            let mut out: Vec<String> = groups
                .iter()
                .flat_map(|group| group.blocks.iter())
                .map(|block| {
                    block
                        .raw
                        .lines()
                        .next()
                        .unwrap_or_default()
                        .trim()
                        .to_string()
                })
                .collect();
            out.sort();
            out
        };
        assert_eq!(rows(&graph, og), tql_rows, "{og} vs {tql}");
    }
    let _ = fs::remove_dir_all(dir);
}

/// OG has `(all-page-tags)`; Tine did not, so a graph using it silently
/// returned nothing. Catalogued as REG-P0-QUERY-ALL-PAGE-TAGS-001.
#[test]
fn all_page_tags_selects_every_page_carrying_a_tag() {
    let (graph, dir) = graph("all-page-tags");
    let (query, view) = parse_query_text("(all-page-tags)", QueryDialect::Og, JournalDate::today());
    assert!(!query.is_invalid(), "{:?}", query.diagnostics);
    let result = crate::query::run_query_result_over(
        &crate::query::GraphQueryPages(&graph),
        &query,
        &view,
        Bounds::unbounded(),
    );
    let QueryRows::Page { pages } = &result.rows else {
        panic!("(all-page-tags) is page-anchored: {:?}", result.rows);
    };
    let names: Vec<&str> = pages.iter().map(|page| page.name.as_str()).collect();
    assert_eq!(names, vec!["Tagged"], "{pages:?}");
    // The block-group adapter answers the same query with that page's blocks,
    // which is the shape a `{{query}}` block renders today.
    case(&graph, "(all-page-tags)", &["a block on a tagged page"]);
    let _ = fs::remove_dir_all(dir);
}

/// The unknown head no longer truncates: the query is refused, not narrowed.
/// Catalogued as REG-P0-QUERY-UNKNOWN-HEAD-001.
#[test]
fn an_unknown_head_returns_nothing_rather_than_a_shorter_query() {
    let (graph, dir) = graph("unknown-head");
    // `(task TODO)` alone matches four rows in this graph. The old parser ran
    // exactly that for the query below, silently.
    assert_eq!(rows(&graph, "(task TODO)").len(), 4);
    case(&graph, "(and (task TODO) (frobnicate x))", &[]);
    let _ = fs::remove_dir_all(dir);
}

/// A graph of exactly the given `(relative path, text)` pages.
fn ad_hoc_graph(label: &str, pages: &[(&str, &str)]) -> (Graph, PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "tine-query-conformance-{label}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("pages")).expect("pages");
    fs::create_dir_all(dir.join("journals")).expect("journals");
    for (path, text) in pages {
        fs::write(dir.join(path), text).expect("page");
    }
    let graph = Graph::open(&dir);
    graph.warm_cache();
    (graph, dir)
}

/// The §3.4 rows that only exist once a key HAS a type: a property atom is
/// coerced by its key's effective type (§6.3), not by how the query spells its
/// literal. The three rows are numeric, date and uncoercible.
///
/// The point of each `!` case is that the SAME comparison under text semantics
/// would answer differently — `'10' > '9'` is false as text and true as a
/// number — so a regression that dropped the registry would fail here rather
/// than pass silently.
#[test]
fn property_atoms_compare_by_their_keys_effective_type() {
    let (graph, dir) = ad_hoc_graph(
        "effective-type",
        &[(
            "pages/Typed.md",
            "\
- score ten
  score:: 10
- score nine
  score:: 9
- score is a word
  score:: high
- due in august
  due:: 2026-08-05
- due in september
  due:: 2026-09-01
- due someday
  due:: someday
",
        )],
    );
    let typed = |query: &str| -> Vec<String> {
        let result = run_query_result(&graph, query, QueryDialect::Tql, Bounds::unbounded());
        let QueryRows::Block { groups } = &result.rows else {
            panic!("a block-anchored query returns block rows");
        };
        let mut matched: Vec<String> = groups
            .iter()
            .flat_map(|group| group.blocks.iter())
            .map(|block| {
                block
                    .raw
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_string()
            })
            .collect();
        matched.sort();
        matched
    };

    // NUMERIC. Two of the three `score` atoms are numbers, so the key is a
    // number key and `10 > 9` holds -- as text, `'10' > '9'` would be false and
    // this would answer with `score nine` instead.
    assert_eq!(typed("prop('score') > 9"), vec!["score ten".to_string()]);
    // UNCOERCIBLE. `high` is not a number, so EVERY comparison on it is false,
    // `!=` included (K3): the word row is absent from both answers.
    assert_eq!(
        typed("prop('score') != 9"),
        vec!["score ten".to_string()],
        "an uncoercible atom fails `!=` too"
    );
    assert_eq!(typed("prop('score') = 'high'"), Vec::<String>::new());
    // ... but it is still PRESENT, which is a property of the list and not of
    // any atom's type.
    assert_eq!(
        typed("prop('score') is not null"),
        vec![
            "score is a word".to_string(),
            "score nine".to_string(),
            "score ten".to_string(),
        ]
    );
    // `every` over a key holding one uncoercible atom is false for that row and
    // true for the rows whose every atom satisfies the predicate.
    assert_eq!(
        typed("every(prop('score'), value > 3)"),
        vec!["score nine".to_string(), "score ten".to_string()]
    );

    // DATE. Two of the three `due` atoms are dates, so the key is a date key
    // and the comparison is on the day, not on the string.
    assert_eq!(
        typed("prop('due') < '2026-08-15'"),
        vec!["due in august".to_string()]
    );
    assert_eq!(
        typed("prop('due') between '2026-08-01' and '2026-09-30'"),
        vec!["due in august".to_string(), "due in september".to_string()]
    );
    // The uncoercible date atom fails every comparison and is still present.
    assert_eq!(
        typed("prop('due') is not null"),
        vec![
            "due in august".to_string(),
            "due in september".to_string(),
            "due someday".to_string(),
        ]
    );

    let _ = fs::remove_dir_all(dir);
}

/// `finish_query_groups` orders result groups by page name and breaks a tie on
/// page KIND, journal first.
///
/// The tie needs two groups carrying the IDENTICAL name string with different
/// kinds. Direct Files cannot produce that pair — it classifies a page by its
/// title, so a `pages/Jul 29th, 2026.md` beside `journals/2026_07_29.md` comes
/// back as two JOURNAL groups (asserted below, so the classification stays
/// true). The tie-break therefore has to be exercised at the function, which is
/// where the rule lives and where a page source that reports kinds
/// independently would hit it.
#[test]
fn two_groups_of_the_same_name_are_ordered_journal_first() {
    use crate::model::PageKind;

    let group = |kind| crate::model::RefGroup {
        page: "Jul 29th, 2026".to_string(),
        kind,
        blocks: Vec::new(),
        evidence: Vec::new(),
    };
    let finished = crate::query::finish_query_groups(
        vec![group(PageKind::Page), group(PageKind::Journal)],
        std::collections::HashMap::new(),
        &crate::query::QueryOpts::from_view(&crate::query::ir::ViewSettings::default()),
        crate::query::ConstructionBudget::new(usize::MAX, usize::MAX),
    );
    assert_eq!(
        finished
            .groups
            .iter()
            .map(|group| group.kind)
            .collect::<Vec<_>>(),
        vec![PageKind::Journal, PageKind::Page],
        "equal names break journal-first"
    );

    // And the classification that makes the pair unreachable from disk.
    let (graph, dir) = ad_hoc_graph(
        "base-order-tie",
        &[
            ("journals/2026_07_29.md", "- TODO from the journal\n"),
            ("pages/Jul 29th, 2026.md", "- TODO from the named page\n"),
        ],
    );
    let bounded = run_query_bounded(&graph, "(task TODO)", usize::MAX, usize::MAX);
    assert_eq!(
        bounded
            .groups
            .iter()
            .map(|group| (group.page.as_str(), group.kind))
            .collect::<Vec<_>>(),
        vec![
            ("Jul 29th, 2026", PageKind::Journal),
            ("Jul 29th, 2026", PageKind::Journal),
        ],
        "Direct Files classifies by title, so both groups are journals"
    );
    let _ = fs::remove_dir_all(dir);
}

/// SPEC §3.2's **pending-oracle** planning fixture.
///
/// A `SCHEDULED:` on the block's own first line, after the marker, is the one
/// planning shape whose expectation the spec defers to a measurement on the OG
/// oracle ("expectation = OG's measured answer, whatever it is; walk and SQL
/// must agree with it").
///
/// **Measured (P0-rust Wave B).** `tools/og-query-oracle/fixture-planning-graph`
/// holds exactly these two blocks; `./q.sh dump.cljs fixture-planning-graph`
/// reports:
///
/// ```text
/// "TODO SCHEDULED: <2026-09-01 Tue>" |marker "TODO" |scheduled 0        |deadline 0
/// "TODO on its own"                  |marker "TODO" |scheduled 20260902 |deadline 0
/// ```
///
/// OG reads no planning timestamp from the marker's own line either: it stores
/// `:block/scheduled 0`. Tine agrees, so the expectation below is now OG's
/// measured answer and not a placeholder.
#[test]
fn planning_on_the_markers_own_line_pins_tines_current_answer() {
    let (graph, dir) = ad_hoc_graph(
        "planning-first-line",
        &[
            (
                "pages/Planning.md",
                "- TODO SCHEDULED: <2026-08-05 Wed>\n- TODO on its own\n  SCHEDULED: <2026-08-06 Thu>\n",
            ),
        ],
    );
    let matched = rows(&graph, "(scheduled)");
    assert_eq!(
        matched,
        vec!["TODO on its own".to_string()],
        "Tine reads planning only from a timestamp that STARTS a source line, so \
         the marker's own line is not a planning row -- and OG, measured on the \
         oracle, stores :block/scheduled 0 for exactly that block."
    );
    let _ = fs::remove_dir_all(dir);
}

// ---------------------------------------------------------------------------
// SPEC §8.1 / §8.3: the five counterfactual modes, over the SAME fixture graph
// the OG oracle measures (`tools/og-query-oracle/fixture-case-graph`).
//
// Gate 1 attributes a walk/OG difference by running the walk with one decision
// switched off at a time. That attribution is only trustworthy if each mode
// actually differs where it should, which is what this matrix pins: the answer
// under every mode, for every case `case.cljs` measures on OG, plus the typed
// case v12 §8.3 adds. Mode `OG` must equal OG's measured answer.

/// The Rust twin of `tools/og-query-oracle/fixture-case-graph`, byte for byte.
/// Two copies of one fixture would drift; this comment is the contract, and
/// `the_case_matrix_mode_og_reproduces_ogs_measured_answers` below carries OG's
/// measured column so a drift shows up as a failing row rather than a silence.
const CASE_PAGES: &[Page] = &[
    Page {
        path: "pages/book.md",
        text: "- the book page\n",
    },
    Page {
        path: "pages/cases.md",
        text: "\
- block A
  status:: Done
- block B
  status:: done
- block C
  type:: [[Book]]
- block D
  type:: [[book]]
- block E
  kind:: Foo Bar
- block F
  type:: #Novel
- block G
  list:: Foo, Bar
- block H
  list:: Foo
",
    },
    Page {
        path: "pages/novel.md",
        text: "type:: Fiction\ntags:: Genre\n\n- page body\n",
    },
    // §8.3's typed case: a key whose declared type is `number`, holding one
    // number and one text value, so `type-attributed` is exercised -- the
    // difference between `both-untyped` and `both` on the SAME atoms.
    Page {
        path: "pages/typed.md",
        text: "- block I\n  score:: 01\n- block J\n  score:: high\n",
    },
    Page {
        path: "pages/score.md",
        text: "tine.type:: number\n\n- the score key page\n",
    },
];

fn case_graph(label: &str) -> (Graph, PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "tine-query-case-matrix-{label}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("pages")).expect("pages");
    fs::create_dir_all(dir.join("journals")).expect("journals");
    fs::create_dir_all(dir.join("logseq")).expect("logseq");
    fs::write(dir.join("logseq/config.edn"), "{}\n").expect("config");
    for page in CASE_PAGES {
        fs::write(dir.join(page.path), page.text).expect("page");
    }
    let graph = Graph::open(&dir);
    graph.warm_cache();
    (graph, dir)
}

fn matched_in(graph: &Graph, query: &str, mode: CompareMode) -> Vec<String> {
    let bounded =
        crate::query::run_query_bounded_in_mode(graph, query, mode, usize::MAX, usize::MAX);
    let mut out: Vec<String> = bounded
        .groups
        .iter()
        .flat_map(|group| group.blocks.iter())
        .map(|block| {
            block
                .raw
                .lines()
                .next()
                .unwrap_or_default()
                .trim()
                .to_string()
        })
        .collect();
    out.sort();
    out
}

/// Mode `OG` must equal OG's own answer. The right-hand column is copied from
/// the oracle run `./q.sh case.cljs fixture-case-graph`, which prints it.
///
/// All twelve rows, including the two page-identity ones: gate 1 runs the
/// same list against the same fixture through the walk
/// (`tools/og-query-oracle/case-queries.txt`) and every row agrees.
#[test]
fn the_case_matrix_mode_og_reproduces_ogs_measured_answers() {
    let (graph, dir) = case_graph("og");
    let rows: &[(&str, &[&str])] = &[
        // Property values are case-SENSITIVE in OG.
        ("(property status Done)", &["block A"]),
        ("(property status done)", &["block B"]),
        // ...and so are ref values, although Book/book is one page.
        ("(property type Book)", &["block C"]),
        ("(property type book)", &["block D"]),
        ("(property type Novel)", &["block F"]),
        ("(property type novel)", &[]),
        // Commas on a non-configured key: OG keeps one string.
        ("(property list Foo)", &["block H"]),
        ("(property list \"Foo, Bar\")", &["block G"]),
        // Page-property is case-sensitive; page-tags is not.
        ("(page-property type Fiction)", &["page body"]),
        ("(page-property type fiction)", &[]),
        // Page identity. OG resolves `[[book]]` and `tags:: Genre` to a PAGE
        // and compares lower-cased names, so both are case-insensitive even in
        // mode `OG` -- and `[[book]]`'s answer includes `book.md`'s own block,
        // which OG reaches through `:block/path-refs` and the walk reaches as
        // the page's own row.
        ("[[book]]", &["block C", "block D", "the book page"]),
        ("(page-tags genre)", &["page body"]),
    ];
    for (query, expected) in rows {
        assert_eq!(
            matched_in(&graph, query, CompareMode::Og),
            expected.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            "mode OG: {query}"
        );
    }
    let _ = fs::remove_dir_all(dir);
}

/// Each of Q20 and Q21 changes exactly the answers it is supposed to change,
/// and nothing else. This is what makes a gate-1 label attributable: if
/// `Q20-only` closes the gap, case folding caused it.
#[test]
fn each_mode_changes_only_the_answers_its_decision_owns() {
    let (graph, dir) = case_graph("modes");

    // Q20 (case folding) turns a case-sensitive miss into a hit, in every mode
    // that carries it, and in no mode that does not.
    for (mode, expected) in [
        (CompareMode::Og, vec!["block A"]),
        (CompareMode::Q20Only, vec!["block A", "block B"]),
        (CompareMode::Q21Only, vec!["block A"]),
        (CompareMode::BothUntyped, vec!["block A", "block B"]),
        (CompareMode::Both, vec!["block A", "block B"]),
    ] {
        assert_eq!(
            matched_in(&graph, "(property status Done)", mode),
            expected,
            "case folding under {}",
            mode.label()
        );
    }

    // Q21 (comma split on every key) turns the whole-string value into two
    // atoms, so a single segment matches.
    for (mode, expected) in [
        (CompareMode::Og, vec!["block H"]),
        (CompareMode::Q20Only, vec!["block H"]),
        (CompareMode::Q21Only, vec!["block G", "block H"]),
        (CompareMode::BothUntyped, vec!["block G", "block H"]),
        (CompareMode::Both, vec!["block G", "block H"]),
    ] {
        assert_eq!(
            matched_in(&graph, "(property list Foo)", mode),
            expected,
            "comma split under {}",
            mode.label()
        );
    }
    // ...and the whole string stops matching once it is split.
    for (mode, expected) in [
        (CompareMode::Og, vec!["block G"]),
        (CompareMode::Q21Only, Vec::<&str>::new()),
        (CompareMode::Both, Vec::<&str>::new()),
    ] {
        assert_eq!(
            matched_in(&graph, "(property list \"Foo, Bar\")", mode),
            expected,
            "the whole string under {}",
            mode.label()
        );
    }
    let _ = fs::remove_dir_all(dir);
}

/// §8.3's typed case (v12): `score:: 01` under a key DECLARED `number`.
///
/// Every OG-ward mode compares `01` by OG's `^\d+$` integer rule, so it matches
/// `(property score 1)`. Only `both` also coerces, which is what distinguishes
/// `type-attributed` from the other labels — and the distinguishing row is the
/// TEXT value `high`, which `both` refuses on a number key while every untyped
/// mode compares as text.
#[test]
fn the_typed_case_separates_both_untyped_from_both() {
    let (graph, dir) = case_graph("typed");

    // OG's own integer rule holds in all four non-`both` modes (v13 §8.1, Y3),
    // and `both` agrees because the declared type is `number`.
    for mode in CompareMode::all() {
        assert_eq!(
            matched_in(&graph, "(property score 1)", mode),
            vec!["block I".to_string()],
            "`01` is the integer 1 under {}",
            mode.label()
        );
    }

    // The text value on a number key: text under every untyped mode, refused
    // under `both`. This is the ONLY row in the matrix where `both-untyped`
    // and `both` differ, so `type-attributed` means exactly this.
    for (mode, expected) in [
        (CompareMode::Og, vec!["block J"]),
        (CompareMode::Q20Only, vec!["block J"]),
        (CompareMode::Q21Only, vec!["block J"]),
        (CompareMode::BothUntyped, vec!["block J"]),
        (CompareMode::Both, Vec::<&str>::new()),
    ] {
        assert_eq!(
            matched_in(&graph, "(property score high)", mode),
            expected,
            "a text value on a declared-number key under {}",
            mode.label()
        );
    }
    let _ = fs::remove_dir_all(dir);
}

/// SPEC §6.2's frozen atom fixture vectors, in the module gate 3 runs.
///
/// `query::atom`'s own tests pin the MECHANISM (origin, ordinal, de-duplication,
/// classification). This is the frozen TABLE, here because gate 3 is
/// `cargo test -p tine-core conformance::` and the table is part of what that
/// gate asserts. Each row is also an oracle case in
/// `tools/og-query-oracle/atoms.cljs`, which records what OG retains for the
/// same value.
#[test]
fn the_frozen_atom_fixture_vectors_hold() {
    use crate::query::atom::{property_atoms, AtomFormat, AtomOrigin};

    let config = crate::config::ParseConfig::default();
    let texts = |key: &str, value: &str| -> Vec<String> {
        property_atoms(key, value, AtomFormat::Markdown, &config)
            .into_iter()
            .map(|atom| atom.text)
            .collect()
    };

    assert_eq!(texts("k", "foo"), vec!["foo"]);
    assert_eq!(texts("k", "[[a]]"), vec!["a"]);
    assert_eq!(texts("k", "foo [[a]]"), vec!["a"]);
    assert_eq!(texts("k", "[[a]] #b"), vec!["a", "b"]);
    assert_eq!(texts("tags", "a, b"), vec!["a", "b"]);
    assert_eq!(texts("k", "a, b"), vec!["a", "b"]);
    assert_eq!(texts("k", "1,5"), vec!["1", "5"]);
    assert!(texts("k", "").is_empty());
    assert!(texts("k", "   ").is_empty());
    assert_eq!(texts("k", "[[a]] [[a]]"), vec!["a"]);
    assert_eq!(texts("k", "\"x, [[y]]\""), vec!["\"x, [[y]]\""]);
    assert_eq!(texts("k", "12"), vec!["12"]);
    assert_eq!(texts("k", "1.5"), vec!["1.5"]);
    assert_eq!(texts("tags", "[[a]], a"), vec!["a"]);
    assert_eq!(texts("template", "weekly review"), vec!["weekly review"]);
    assert_eq!(texts("title", "A, B"), vec!["A", "B"]);

    // Step 1 (v12, VERIFY-11 A1): reference parsing suppressed, comma split
    // still applied.
    let mut ignored = crate::config::ParseConfig::default();
    ignored.ignored_page_references_keywords = vec!["url".into()];
    let atoms = property_atoms("url", "http://a.b/x, [[y]]", AtomFormat::Markdown, &ignored);
    assert_eq!(
        atoms.iter().map(|a| a.text.clone()).collect::<Vec<_>>(),
        vec!["http://a.b/x", "[[y]]"]
    );
    assert!(atoms.iter().all(|a| a.origin == AtomOrigin::Plain));

    // The G8 repeated-row flattening fixture: `k:: a` twice, `K:: b`, then
    // `k:: a, c` -- one owner, four source rows, first occurrence wins.
    let flattened: Vec<String> = ["a", "a", "b", "a, c"]
        .iter()
        .flat_map(|value| property_atoms("k", value, AtomFormat::Markdown, &config))
        .fold(
            Vec::new(),
            |mut out: Vec<crate::query::atom::Atom>, atom| {
                if !out.iter().any(|existing| existing.key == atom.key) {
                    out.push(atom);
                }
                out
            },
        )
        .into_iter()
        .map(|atom| atom.text)
        .collect();
    assert_eq!(flattened, vec!["a", "b", "c"]);
}

// ---------------------------------------------------------------------------
// §4.3.1 — the document form of a query (R1)
//
// These are the gate-3 half of "macro recognition/refusal on both formats"
// (§8.3). They assert the LITERAL bytes §4.3.1 pins, that the raw reader
// recovers them, and that a refusal writes nothing.
// ---------------------------------------------------------------------------

use crate::query::ir::{
    Attr, CmpOp, DiagnosticKind, Filter, Quant, Query, Rel, Source, ViewSettings,
};
use crate::query::macro_text::{macro_safe, query_macro_extent, recognizable_macro, FormFamily};
use crate::query::print::{query_print, PrintDialect};
use crate::query::registry::Registry;
use crate::query::tql::parse_tql;

/// A block-anchored, empty-view, source-free query over `filter` — the shape
/// §4.3.1's literal output table is stated for.
fn builder(filter: Filter) -> Query {
    Query::new(Anchor::Block, filter, Source::Builder)
}

fn saved_macro(query: &Query) -> String {
    let argument = query_print(
        query,
        &ViewSettings::default(),
        PrintDialect::TqlMacro,
        false,
    )
    .unwrap_or_else(|d| panic!("printable: {d:?}"));
    format!("{{{{tine-query {argument}}}}}")
}

/// The walk's answer for TQL text. `rows` above reads the OG dialect; the
/// engine underneath is the same one (I-12), only the surface syntax differs.
fn tql_rows(graph: &Graph, source: &str) -> Vec<String> {
    let query = parse_tql(source, Registry::none()).0;
    let view = ViewSettings::default();
    let bounded = crate::query::run_pred_bounded_over(
        &crate::query::GraphQueryPages(graph),
        &query,
        &view,
        usize::MAX,
        usize::MAX,
    );
    let mut out: Vec<String> = bounded
        .groups
        .iter()
        .flat_map(|group| group.blocks.iter())
        .map(|block| {
            block
                .raw
                .lines()
                .next()
                .unwrap_or_default()
                .trim()
                .to_string()
        })
        .collect();
    out.sort();
    out
}

fn ref_a() -> Filter {
    Filter::page_ref("a")
}
fn ref_b() -> Filter {
    Filter::page_ref("b")
}

/// §4.3.1's three literal output fixtures, byte for byte.
#[test]
fn the_three_pinned_macro_outputs_are_exact() {
    assert_eq!(
        saved_macro(&builder(Filter::and(vec![
            Filter::off(ref_a()),
            Filter::off(ref_b())
        ]))),
        "{{tine-query @block and off([[a]]) and off([[b]])}}"
    );
    assert_eq!(
        saved_macro(&builder(Filter::off(Filter::off(ref_a())))),
        "{{tine-query @block and off([[a]])}}",
        "the printer normalizes nested Off before either layout"
    );
    assert_eq!(
        saved_macro(&builder(Filter::not(Filter::off(ref_a())))),
        "{{tine-query @block and not off([[a]])}}"
    );
}

/// The title/options variant of each: the map is appended once, verbatim, and
/// the raw reader recovers it exactly — which the AST argument cannot, because
/// the document parser's macro ends before the final `}`.
#[test]
fn each_pinned_output_carries_its_options_map_and_the_raw_reader_recovers_it() {
    let options = "{:title \"T\" :collapsed? true}";
    for filter in [
        Filter::and(vec![Filter::off(ref_a()), Filter::off(ref_b())]),
        Filter::off(Filter::off(ref_a())),
        Filter::not(Filter::off(ref_a())),
    ] {
        let query = Query::new(
            Anchor::Block,
            filter,
            Source::Tql {
                original: String::new(),
                og_options: options.to_string(),
            },
        );
        let argument = query_print(
            &query,
            &ViewSettings::default(),
            PrintDialect::TqlMacro,
            false,
        )
        .expect("printable");
        assert!(
            argument.ends_with(options),
            "the options map is appended once, verbatim: {argument}"
        );
        let raw = format!("{{{{tine-query {argument}}}}}");
        let extent = query_macro_extent(&raw).expect("a macro");
        assert_eq!(
            extent.argument, argument,
            "the raw reader recovers it whole"
        );
        assert_eq!(extent.end, raw.len(), "including the options closing brace");
    }
}

/// The persisted form ALWAYS carries an explicit anchor. Without it the document
/// parser takes its leading-page-reference argument alternative and the macro
/// stops being a macro — measured in both Markdown and Org.
#[test]
fn the_persisted_form_is_anchored_and_the_unanchored_one_is_not_a_macro() {
    assert_eq!(
        saved_macro(&builder(ref_a())),
        "{{tine-query @block and [[a]]}}"
    );
    // Measured in both readers: a page reference ALONE is an accepted macro
    // argument, but one followed by anything else takes the leading-reference
    // alternative and the macro stops being a macro. The explicit anchor is
    // what makes every ordinary conjunction safe.
    assert!(recognizable_macro("tine-query", "[[a]]").is_ok());
    assert!(recognizable_macro("tine-query", "[[a]] and task = 'TODO'").is_err());
    assert!(recognizable_macro("tine-query", "@block and [[a]] and task = 'TODO'").is_ok());
}

/// A bare anchor when the filter is exactly `True`; no trailing ` and true`.
#[test]
fn a_true_filter_prints_the_anchor_alone() {
    assert_eq!(saved_macro(&builder(Filter::True)), "{{tine-query @block}}");
    let page = Query::new(Anchor::Page, Filter::True, Source::Builder);
    assert_eq!(saved_macro(&page), "{{tine-query @page}}");
}

/// §4.3.1's focused acceptance: the comma survives with no inserted space, in
/// both readers, even though the AST argument arrives split in two.
#[test]
fn a_comma_in_a_literal_survives_the_document_round_trip_in_both_formats() {
    let query = parse_tql("@block and content = 'a,b'", Registry::none()).0;
    let argument = query_print(
        &query,
        &ViewSettings::default(),
        PrintDialect::TqlMacro,
        false,
    )
    .expect("printable");
    assert_eq!(argument, "@block and content = 'a,b'");
    for format in ["md", "org"] {
        let raw = format!("{{{{tine-query {argument}}}}}");
        let nodes = lsdoc::inline(&raw, format);
        assert!(
            matches!(nodes.first(), Some(lsdoc::ast::Inline::Macro { name, .. }) if name == "tine-query"),
            "{format} must read it back as a macro"
        );
        assert_eq!(
            query_macro_extent(&raw).expect("a macro").argument,
            argument,
            "{format}: the raw reader keeps the comma with no inserted space"
        );
    }
}

/// The lone-brace / semicolon / escaped-quote fixtures produce identical split
/// AND extent results — the two readers are one scan (I-12).
#[test]
fn split_and_extent_agree_on_the_lone_brace_semicolon_and_escaped_quote_cases() {
    for form in [
        "content = '{'",
        "content = 'a;b'",
        "content = 'a''b'",
        "content = 'a,b'",
    ] {
        let options = "{:title \"T\"}";
        let argument = format!("{form} {options}");
        let (split_form, split_options) =
            crate::query::macro_text::split_trailing_map(&argument, FormFamily::Tql);
        assert_eq!(
            (split_form.as_str(), split_options.as_str()),
            (form, options)
        );
        let raw = format!("{{{{tine-query {argument}}}}}");
        let extent = query_macro_extent(&raw).expect("a macro");
        assert_eq!(extent.argument, argument);
        let (again_form, again_options) =
            crate::query::macro_text::split_trailing_map(&extent.argument, FormFamily::Tql);
        assert_eq!(
            (again_form, again_options),
            (split_form, split_options),
            "split and extent must agree on {form}"
        );
    }
}

/// An unsafe save is a refusal that writes nothing: the printer returns the
/// located diagnostic and the caller still holds its original bytes (I-4).
#[test]
fn an_unsafe_argument_is_refused_and_the_original_bytes_are_untouched() {
    let original = "{{tine-query @block and [[a]]}}";
    let query = parse_tql("@block and content = 'x}'", Registry::none()).0;
    let refusal = query_print(
        &query,
        &ViewSettings::default(),
        PrintDialect::TqlMacro,
        false,
    )
    .expect_err("a `}` in the form cannot be saved as a macro");
    assert_eq!(refusal.kind, DiagnosticKind::Syntax);
    assert!(refusal.span.is_some(), "the refusal is located");
    assert_eq!(original, "{{tine-query @block and [[a]]}}");
    // The lexical rule and the parser agree here; the parser is the authority.
    assert!(macro_safe("@block and content = 'x}'", FormFamily::Tql).is_err());
}

/// A title-only edit preserves the authored form, including a form the filter
/// printer could never regenerate.
#[test]
fn a_title_only_edit_preserves_an_unsupported_authored_form() {
    let advanced = "[:find (pull ?b [*]) :where [?b :block/marker \"TODO\"]]";
    let query = Query::new(
        Anchor::Block,
        Filter::True,
        Source::Advanced {
            original: advanced.to_string(),
            og_options: "{:title \"New\"}".to_string(),
        },
    );
    let printed = query_print(
        &query,
        &ViewSettings::default(),
        PrintDialect::AdvancedMacro,
        false,
    )
    .expect("source-preserving");
    assert_eq!(printed, format!("{advanced} {{:title \"New\"}}"));
    // A non-Advanced source gets `NotApplicable`, never a regenerated datalog.
    let tql = parse_tql("@block and [[a]]", Registry::none()).0;
    let refused = query_print(
        &tql,
        &ViewSettings::default(),
        PrintDialect::AdvancedMacro,
        false,
    )
    .expect_err("not an advanced source");
    assert_eq!(refused.kind, DiagnosticKind::NotApplicable);
    // `Builder` is refused on every source-preserving path.
    assert_eq!(
        query_print(
            &builder(Filter::True),
            &ViewSettings::default(),
            PrintDialect::TqlMacro,
            true,
        )
        .expect_err("a builder query has no authored form")
        .kind,
        DiagnosticKind::NotApplicable
    );
}

// ---------------------------------------------------------------------------
// §3.5 — omission-safe normalization (R2)
// ---------------------------------------------------------------------------

/// Round-trip is now **structural AND semantic**: normalizing may not change
/// what the query answers, and neither may printing and re-parsing it.
///
/// `eval` here is the executable tree — the filter after `Off` omission — which
/// is exactly what the walk and the lowering consume (§3.5).
fn evaluable(filter: &Filter) -> Filter {
    Query::new(Anchor::Block, filter.clone(), Source::Builder).evaluable_filter()
}

#[test]
fn normalization_preserves_truth_over_constants_off_not_and_relations() {
    let cases: Vec<Filter> = vec![
        Filter::True,
        Filter::False,
        Filter::and(vec![]),
        Filter::or(vec![]),
        // R2's two named regressions: identity removal turned each of these
        // into a fully disabled root, which evaluates as `True`.
        Filter::or(vec![Filter::False, Filter::off(Filter::True)]),
        Filter::not(Filter::and(vec![Filter::True, Filter::off(Filter::True)])),
        Filter::and(vec![Filter::True, Filter::off(ref_a())]),
        Filter::or(vec![Filter::off(ref_a()), Filter::off(ref_b())]),
        Filter::not(Filter::off(ref_a())),
        // A relation predicate with an empty and a nonempty related set.
        Filter::rel(Rel::Children, Quant::Any, Filter::or(vec![])),
        Filter::rel(Rel::Children, Quant::Any, Filter::and(vec![])),
        Filter::rel(
            Rel::Children,
            Quant::Any,
            Filter::and(vec![ref_a(), Filter::off(ref_b())]),
        ),
        Filter::rel(Rel::Children, Quant::Any, Filter::True),
    ];
    for filter in cases {
        let query = builder(filter.clone());
        assert_eq!(
            evaluable(&query.normalized().filter),
            evaluable(&filter),
            "normalizing changed what {filter:?} evaluates to"
        );
        // Idempotent: normalizing twice is normalizing once.
        assert_eq!(
            query.normalized().normalized().filter,
            query.normalized().filter
        );
    }
}

/// The same property through both print forms: parse(print(q)) must answer what
/// `q` answers, in the pane layout and in the persisted macro form.
#[test]
fn both_print_forms_round_trip_semantically() {
    let sources = [
        "@block and [[a]] and off([[b]])",
        "@block and (false or off(true))",
        "@block and not (true and off(true))",
        "@block and off([[a]]) and off([[b]])",
        "@block and not off([[a]])",
    ];
    for source in sources {
        let query = parse_tql(source, Registry::none()).0;
        assert!(!query.is_invalid(), "{source}: {:?}", query.diagnostics);
        let normalized = query.normalized();
        for dialect in [PrintDialect::Tql, PrintDialect::TqlMacro] {
            let printed = query_print(&query, &ViewSettings::default(), dialect, false)
                .unwrap_or_else(|d| panic!("{source} as {dialect:?}: {d:?}"));
            let again = parse_tql(&printed, Registry::none()).0;
            assert!(
                !again.is_invalid(),
                "{source} printed {printed:?} as {dialect:?}, which does not parse: {:?}",
                again.diagnostics
            );
            assert_eq!(
                again.normalized().filter,
                normalized.filter,
                "{source} as {dialect:?} printed {printed:?}: structural round trip"
            );
            assert_eq!(
                evaluable(&again.filter),
                evaluable(&query.filter),
                "{source} as {dialect:?} printed {printed:?}: semantic round trip"
            );
        }
    }
}

/// A quantifier's comma exposes a later-argument page reference, so the pane can
/// hold it and the document cannot. The pane round-trips it; the macro printer
/// refuses it and writes nothing (§4.3.1: no promise that every accepted text
/// literal fits a document macro).
#[test]
fn a_quantifier_over_a_page_ref_round_trips_in_the_pane_and_is_refused_as_a_macro() {
    let source = "@block and any(children, [[a]] and off([[b]]))";
    let query = parse_tql(source, Registry::none()).0;
    assert!(!query.is_invalid(), "{:?}", query.diagnostics);
    let pane = query_print(&query, &ViewSettings::default(), PrintDialect::Tql, false)
        .expect("the pane never checks macro safety");
    let again = parse_tql(&pane, Registry::none()).0;
    assert_eq!(again.normalized().filter, query.normalized().filter);
    assert_eq!(evaluable(&again.filter), evaluable(&query.filter));
    let refusal = query_print(
        &query,
        &ViewSettings::default(),
        PrintDialect::TqlMacro,
        false,
    )
    .expect_err("a comma that exposes a page reference is not a macro");
    assert_eq!(refusal.kind, DiagnosticKind::Syntax);
}

/// Cache keys use this normalization, so two trees that differ in truth must not
/// normalize to the same key (§3.5, §5.9).
#[test]
fn normalization_never_conflates_two_trees_that_differ_in_truth() {
    let false_tree = Filter::or(vec![Filter::False, Filter::off(Filter::True)]);
    let true_tree = Filter::off(Filter::True);
    assert_ne!(
        builder(false_tree.clone()).normalized().filter,
        builder(true_tree.clone()).normalized().filter
    );
    assert_eq!(evaluable(&false_tree), Filter::or(vec![Filter::False]));
    assert_eq!(evaluable(&true_tree), Filter::True);
}

// ---------------------------------------------------------------------------
// §4.3.2 — lossless invalid and legacy leaves (R4)
// ---------------------------------------------------------------------------

/// A malformed disabled operand is CAPTURED, not replaced by `off(false)`: the
/// payload survives a save and reopen next to an untouched active row.
#[test]
fn a_broken_disabled_row_survives_a_save_and_reopen_beside_an_active_row() {
    let authored = "-- task = '\nand [[a]]";
    let query = parse_tql(authored, Registry::none()).0;
    assert!(
        !query.is_invalid(),
        "a disabled broken row does not invalidate the query: {:?}",
        query.diagnostics
    );
    let Filter::And { items } = &query.normalized().filter else {
        panic!("{:?}", query.filter);
    };
    assert_eq!(
        items[0],
        Filter::off(Filter::raw("task = '", DiagnosticKind::Syntax)),
        "the exact payload is retained, never `off(false)`"
    );
    assert_eq!(
        items[1],
        ref_a(),
        "the neighbouring active row is untouched"
    );

    // Save, reopen: payload and kind both survive, and so does the neighbour.
    let saved = query_print(
        &query,
        &ViewSettings::default(),
        PrintDialect::TqlMacro,
        false,
    )
    .expect("printable");
    let reopened = parse_tql(&saved, Registry::none()).0;
    assert_eq!(reopened.normalized().filter, query.normalized().filter);
    assert!(!reopened.is_invalid());
}

/// Payloads with line breaks, quotes, braces and Unicode survive the capsule.
#[test]
fn a_capsule_preserves_line_breaks_quotes_braces_and_unicode() {
    for payload in [
        "a\nb",
        "quote ' and \"double\"",
        "braces { } and #{",
        "unicode: héllo 😀 ключ",
        "",
    ] {
        let query = builder(Filter::raw(payload, DiagnosticKind::Syntax));
        let printed = crate::query::print::print_tql(&query);
        let again = parse_tql(&printed, Registry::none()).0;
        assert_eq!(
            again.normalized().filter,
            Filter::raw(payload, DiagnosticKind::Syntax),
            "{payload:?} did not survive {printed}"
        );
    }
}

/// Every one of the six kinds round-trips exactly; prose may be regenerated,
/// the kind may not be lost.
#[test]
fn every_diagnostic_kind_round_trips_through_its_capsule() {
    for kind in [
        DiagnosticKind::UnknownHead,
        DiagnosticKind::Syntax,
        DiagnosticKind::UnknownIdent,
        DiagnosticKind::NotApplicable,
        DiagnosticKind::Depth,
        DiagnosticKind::Size,
    ] {
        let query = builder(Filter::raw("(frobnicate x)", kind));
        let printed = crate::query::print::print_tql(&query);
        let again = parse_tql(&printed, Registry::none()).0;
        assert_eq!(
            again.normalized().filter,
            Filter::raw("(frobnicate x)", kind)
        );
        assert_eq!(
            again.diagnostics.first().map(|d| d.kind),
            Some(kind),
            "an enabled capsule yields its retained kind and invalidates the query"
        );
        assert!(
            again.is_invalid(),
            "no auto-execution of previously invalid text"
        );
    }
}

/// A capsule that will not decode degrades to `Syntax` with its bytes retained —
/// never to an executable predicate, and never to silence.
#[test]
fn a_bad_capsule_is_syntax_and_never_an_executable_predicate() {
    for bad in [
        "raw_hex('nonsense', '61')",
        "raw_hex('syntax', '6')",
        "raw_hex('syntax', 'zz')",
        "raw_hex('syntax', 'ff')",
    ] {
        let query = parse_tql(&format!("@block and {bad}"), Registry::none()).0;
        assert!(
            matches!(
                query.normalized().filter,
                Filter::Raw {
                    kind: DiagnosticKind::Syntax,
                    ..
                }
            ),
            "{bad} became {:?}",
            query.filter
        );
        assert!(query.is_invalid(), "{bad} must report its problem");
    }
}

/// Re-enabling a capsule shows the retained error and returns no results.
#[test]
fn re_enabling_a_capsule_restores_an_enabled_error_rather_than_running_it() {
    let disabled = parse_tql(
        "@block and off(raw_hex('syntax', '2d2d20'))",
        Registry::none(),
    )
    .0;
    assert!(!disabled.is_invalid(), "disabled: greyed, not invalid");
    assert!(disabled.diagnostics.iter().all(|d| d.disabled));
    let enabled = parse_tql("@block and raw_hex('syntax', '2d2d20')", Registry::none()).0;
    assert!(
        enabled.is_invalid(),
        "re-enabled: the retained error is back"
    );
    assert_eq!(
        enabled.diagnostics[0].message, "`-- ` does not parse",
        "the renderer's input is the decoded original text, not hexadecimal"
    );
}

/// The legacy `(content-regex …)` head and its TQL spelling are one leaf.
#[test]
fn the_legacy_regex_head_and_the_tql_regexp_spelling_are_the_same_leaf() {
    let (og, _) = parse_query_text(
        "(content-regex \"^a.*b$\")",
        QueryDialect::Og,
        JournalDate::from_ordinal(20260904),
    );
    let tql = parse_tql("@block and content regexp '^a.*b$'", Registry::none()).0;
    assert_eq!(tql.normalized().filter, og.normalized().filter);
    // The printer emits the TQL spelling, which parses back to the same leaf.
    // The pane layout omits the default block anchor (§4.3); the persisted
    // macro form never does.
    let printed = crate::query::print::print_tql(&og);
    assert_eq!(printed, "content regexp '^a.*b$'");
    assert_eq!(
        query_print(&og, &ViewSettings::default(), PrintDialect::TqlMacro, false)
            .expect("printable"),
        "@block and content regexp '^a.*b$'"
    );
    assert_eq!(
        parse_tql(&printed, Registry::none()).0.normalized().filter,
        og.normalized().filter
    );
}

/// `regexp` is content-only, and the `RLIKE` alias is deliberately not admitted.
#[test]
fn regexp_is_content_only_and_rlike_is_not_a_second_spelling() {
    for refused in [
        "@block and task regexp 'a'",
        "@page and name regexp 'a'",
        "@block and prop('k') regexp 'a'",
        "@block and content rlike 'a'",
    ] {
        let query = parse_tql(refused, Registry::none()).0;
        assert!(query.is_invalid(), "{refused} must be refused");
    }
    // Negation lowers through ordinary `Not`, not a second operator.
    let negated = parse_tql("@block and not content regexp 'a'", Registry::none()).0;
    assert!(!negated.is_invalid(), "{:?}", negated.diagnostics);
    assert_eq!(
        negated.normalized().filter,
        Filter::not(Filter::attr(
            Attr::Content,
            CmpOp::Regex,
            crate::query::ir::Value::text("a")
        ))
    );
}

/// An invalid pattern is retained and matches FALSE without an enabled
/// whole-query diagnostic, so negating it matches true. Never a panic, never a
/// silently discarded leaf (§4.3.2, the existing behaviour).
#[test]
fn an_invalid_regex_pattern_is_retained_and_matches_false() {
    let (graph, _dir) = ad_hoc_graph("regex-invalid", &[("pages/R.md", "- alpha\n- beta\n")]);
    let query = parse_tql("@block and content regexp '('", Registry::none()).0;
    assert!(
        !query.is_invalid(),
        "an invalid pattern is not a whole-query diagnostic: {:?}",
        query.diagnostics
    );
    assert!(tql_rows(&graph, "@block and content regexp '('").is_empty());
    assert_eq!(
        tql_rows(&graph, "@block and not content regexp '('").len(),
        2,
        "negating a leaf that matches false matches true — never a panic, never a dropped leaf"
    );
}

// ---------------------------------------------------------------------------
// §7.1 — the macro-input dispatch (C3)
// ---------------------------------------------------------------------------

use crate::query::{parse_query_input, QueryInput};

fn parsed(text: &str, input: QueryInput) -> Query {
    parse_query_input(
        text,
        input,
        JournalDate::from_ordinal(20260904),
        Registry::none(),
    )
    .0
}

/// The macro inputs split their argument ONCE and record the grammar in the
/// source variant. `macro_query` picks advanced or OG by token, never by a
/// speculative parse.
#[test]
fn the_macro_inputs_split_once_and_record_the_grammar() {
    let og = parsed("(task TODO) {:title \"T\"}", QueryInput::MacroQuery);
    assert!(matches!(
        &og.source,
        Source::Og { original, og_options }
            if original == "(task TODO)" && og_options == "{:title \"T\"}"
    ));

    let tql = parsed("@block and [[a]] {:title \"T\"}", QueryInput::MacroTql);
    assert!(matches!(
        &tql.source,
        Source::Tql { original, og_options }
            if original == "@block and [[a]]" && og_options == "{:title \"T\"}"
    ));
    assert_eq!(
        tql.normalized().filter,
        ref_a(),
        "the map never reaches the grammar"
    );

    let advanced = parsed(
        "[:find (pull ?b [*]) :where [?b :block/marker \"TODO\"]] {:title \"T\"}",
        QueryInput::MacroQuery,
    );
    assert!(matches!(
        &advanced.source,
        Source::Advanced { original, og_options }
            if original == "[:find (pull ?b [*]) :where [?b :block/marker \"TODO\"]]"
                && og_options == "{:title \"T\"}"
    ));
}

/// A whole `{:query … :inputs …}` map is the FORM, not options (X4, §4.4).
#[test]
fn a_whole_advanced_map_is_the_form_not_the_options() {
    let form = "{:query [:find ?b :where [?b :block/marker \"TODO\"]] :inputs [:current-page]}";
    let query = parsed(form, QueryInput::MacroQuery);
    assert!(matches!(
        &query.source,
        Source::Advanced { original, og_options } if original == form && og_options.is_empty()
    ));
    // A map that FOLLOWS it still splits.
    let with_options = parsed(&format!("{form} {{:title \"T\"}}"), QueryInput::MacroQuery);
    assert!(matches!(
        &with_options.source,
        Source::Advanced { original, og_options }
            if original == form && og_options == "{:title \"T\"}"
    ));
}

/// The one discriminator protects strings and page refs: `:find` inside a
/// literal is text, and Macro and Export therefore pick the same variant.
#[test]
fn a_find_token_inside_a_literal_picks_the_same_source_variant_in_every_caller() {
    for form in [
        "(property note \"see the :find clause\")",
        "(property note \"a :where clause\")",
        "[[a :find b]]",
    ] {
        let query = parsed(form, QueryInput::MacroQuery);
        assert!(
            matches!(&query.source, Source::Og { .. }),
            "{form} is OG text, not datalog: {:?}",
            query.source
        );
    }
    assert!(matches!(
        parsed(
            "[:find ?b :where [?b :block/marker \"TODO\"]]",
            QueryInput::MacroQuery
        )
        .source,
        Source::Advanced { .. }
    ));
}

/// An options map appended to a TQL macro survives a title edit unchanged,
/// through the source-preserving path.
#[test]
fn a_tql_macro_title_edit_preserves_the_form_and_writes_the_new_map_once() {
    let mut query = parsed("@block and [[a]] {:title \"Old\"}", QueryInput::MacroTql);
    let Source::Tql { og_options, .. } = &mut query.source else {
        panic!("{:?}", query.source);
    };
    *og_options = "{:title \"New\"}".to_string();
    let printed = query_print(
        &query,
        &ViewSettings::default(),
        PrintDialect::TqlMacro,
        true,
    )
    .expect("source-preserving");
    assert_eq!(printed, "@block and [[a]] {:title \"New\"}");
    let raw = format!("{{{{tine-query {printed}}}}}");
    assert_eq!(query_macro_extent(&raw).expect("a macro").argument, printed);
}
