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
/// must agree with it"). That measurement has NOT been taken in this lane — the
/// headless OG oracle is not part of the P0-rust packet — so this test pins what
/// TINE answers today rather than what OG answers. It exists so the P1 lane that
/// takes the measurement finds a red test if the two disagree, instead of
/// discovering the divergence in the field.
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
         the marker's own line is not a planning row. OG's answer is unmeasured."
    );
    let _ = fs::remove_dir_all(dir);
}
