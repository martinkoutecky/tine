//! Property-presence narrowing (`exec_candidates.rs` `Need::Key`): a
//! `(property k)` query visits only the pages the query index lists for `k`,
//! and answers exactly as the full scan does.
use super::*;

/// Unrelated pages, export and share definitions in every place a property
/// row can sit (outline block, nested child, page header, prose before the
/// first block, Org drawer, Org `#+KEY:`, a journal), keys spelled with other
/// case or `-`/`_`, and the unindexed keys (`id::`, `tine.type::`).
fn open(unrelated: usize) -> (tempfile::TempDir, crate::Store) {
    open_with(unrelated, 0)
}

/// [`open`] plus one page referencing `ghosts` pages that have no file
/// (fileless rows of a page-anchored query).
fn open_with(unrelated: usize, ghosts: usize) -> (tempfile::TempDir, crate::Store) {
    let dir = tempfile::tempdir().unwrap();
    for sub in ["pages", "journals"] {
        std::fs::create_dir(dir.path().join(sub)).unwrap();
    }
    if ghosts > 0 {
        let refs: String = (0..ghosts)
            .map(|n| format!("- see [[Ghost {n}]]\n"))
            .collect();
        std::fs::write(dir.path().join("pages/Refs.md"), refs).unwrap();
    }
    for n in 0..unrelated {
        std::fs::write(
            dir.path().join(format!("pages/U{n}.md")),
            format!(
                "- unrelated\n  status:: open\n  id:: 6553f1a0-0000-4000-8000-{n:012}\n- more\n"
            ),
        )
        .unwrap();
    }
    for (file, text) in [
        (
            "pages/Exports.md",
            "- {{query (page \"x\")}}\n  tine.export:: out\n- {{tine-query @page name = 'A'}}\n  tine.share:: me/0123456789abcdef0123\n",
        ),
        (
            "pages/Spelled.md",
            "- upper\n  Tine.Export:: out\n  - nested\n    TINE.SHARE:: me/x\n- dashed\n  tine_share:: a\n  Due_Date:: 3\n- other\n  tine-share:: b\n  due-date:: 4\n",
        ),
        ("pages/Header.md", "tine.export:: folder\n\n- body\n"),
        ("pages/Prose.md", "Some prose\ntine.share:: me/p\n\n- body\n"),
        (
            "pages/Drawer.org",
            "* query\n  :PROPERTIES:\n  :tine.export: out\n  :END:\n* plain\n",
        ),
        ("pages/Directive.org", "#+tine.share: me/o\n\n* heading\n"),
        ("journals/2026_10_02.md", "- day\n  tine.share:: me/j\n"),
        (
            "pages/Typed.md",
            "tine.type:: number\n\n- typed\n  tine.type:: text\n",
        ),
    ] {
        std::fs::write(dir.path().join(file), text).unwrap();
    }
    let store = crate::Store::open(dir.path(), Default::default())
        .unwrap()
        .0;
    store.whole_graph().unwrap();
    (dir, store)
}

/// The answer and the candidate pages handed to the row loop.
fn run(graph: &ReadSnapshot, source: &str, dialect: QueryDialect, narrow: bool) -> (String, usize) {
    let today = JournalDate::today();
    let (query, view) = parse_query_text(source, dialect, today);
    assert!(
        query.diagnostics.is_empty(),
        "{source}: {:?}",
        query.diagnostics
    );
    let resolved = resolve_for_execution(&query, &ExecutionContext::none(), today);
    candidates::NARROW_BY_KEYS.with(|on| on.set(narrow));
    candidates::CANDIDATE_PAGES.with(|count| count.set(0));
    let bounds = Bounds {
        max_rows: crate::store::RESULT_BRIDGE_MAX_ROWS,
        max_bytes: crate::store::RESULT_BRIDGE_MAX_BYTES,
    };
    let (result, _) = run_resolved_cancellable(graph, &resolved, &view, bounds, &|| false);
    candidates::NARROW_BY_KEYS.with(|on| on.set(true));
    let visited = candidates::CANDIDATE_PAGES.with(|count| count.get());
    let result = result.unwrap().expect("never cancelled");
    (serde_json::to_string(&result).unwrap(), visited)
}

const QUERIES: [(&str, QueryDialect); 19] = [
    (
        "(or (property tine.export) (property tine.share))",
        QueryDialect::Og,
    ),
    ("(property tine.export)", QueryDialect::Og),
    ("(property Tine.Share)", QueryDialect::Og),
    ("(property tine_share)", QueryDialect::Og),
    ("(property due-date)", QueryDialect::Og),
    ("(property tine.export \"out\")", QueryDialect::Og),
    ("(page-property tine.export)", QueryDialect::Og),
    ("(page-property tine.share)", QueryDialect::Og),
    ("(and (property tine.share) (task TODO))", QueryDialect::Og),
    (
        "(or (property tine.export) (page \"U1\"))",
        QueryDialect::Og,
    ),
    ("(not (property tine.export))", QueryDialect::Og),
    ("(property id)", QueryDialect::Og),
    ("(page-property tine.type)", QueryDialect::Og),
    ("(property absent-key)", QueryDialect::Og),
    (
        "@block and prop('TINE.share') is not null",
        QueryDialect::Tql,
    ),
    (
        "@page and prop('tine.export') is not null",
        QueryDialect::Tql,
    ),
    (
        "@block and every(prop('due_date'), value > 3)",
        QueryDialect::Tql,
    ),
    ("@block and prop('tine.export') = ''", QueryDialect::Tql),
    ("@block and prop('tine.share') is null", QueryDialect::Tql),
];

#[test]
fn property_presence_narrowing_answers_as_the_full_scan() {
    let (dir, store) = open(30);
    let whole = store.whole_graph().unwrap();
    let graph: &ReadSnapshot = &whole.graph;
    for (source, dialect) in QUERIES {
        let (narrowed, _) = run(graph, source, dialect, true);
        let (scanned, _) = run(graph, source, dialect, false);
        assert_eq!(narrowed, scanned, "{source}");
    }
    // The fixture exercises every placement: each definition is found.
    let (answer, _) = run(
        graph,
        "(or (property tine.export) (property tine.share))",
        QueryDialect::Og,
        true,
    );
    for page in [
        "Exports",
        "Spelled",
        "Header",
        "Prose",
        "Drawer",
        "Directive",
    ] {
        assert!(
            answer.contains(&format!("\"page\":\"{page}\"")),
            "{page}: {answer}"
        );
    }
    assert!(answer.contains("me/j"), "journal definition: {answer}");
    // An edit patches the key -> pages table (the seeded index): a key added
    // to an unrelated page, and one removed from its last holder, answer as
    // the scan of the new generation does.
    drop(whole);
    std::fs::write(
        dir.path().join("pages/U1.md"),
        "- now a definition\n  tine.export:: late\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("pages/Header.md"), "- body\n").unwrap();
    store.refresh(crate::Depth::Stamps).unwrap();
    let whole = store.whole_graph().unwrap();
    let graph: &ReadSnapshot = &whole.graph;
    for (source, dialect) in QUERIES {
        let (narrowed, _) = run(graph, source, dialect, true);
        let (scanned, _) = run(graph, source, dialect, false);
        assert_eq!(narrowed, scanned, "after edit: {source}");
    }
    let (answer, _) = run(graph, "(property tine.export)", QueryDialect::Og, true);
    assert!(answer.contains("\"page\":\"U1\""), "{answer}");
    assert!(!answer.contains("\"page\":\"Header\""), "{answer}");
    store.close();
}

/// PC-1: the export-definitions query visits the pages holding a destination
/// key, whatever the number of unrelated pages; an unindexed key still scans.
#[test]
fn export_definitions_visit_only_the_pages_holding_the_key() {
    let mut visits = Vec::new();
    for unrelated in [5, 50] {
        let (_dir, store) = open(unrelated);
        let whole = store.whole_graph().unwrap();
        let graph: &ReadSnapshot = &whole.graph;
        let (_, definitions) = run(
            graph,
            &tine_core::export_definition::definitions_query(),
            QueryDialect::Og,
            true,
        );
        let (_, scanned) = run(graph, "(property id)", QueryDialect::Og, true);
        visits.push((definitions, scanned));
        store.close();
    }
    // Exports, Spelled, Header, Prose, Drawer, Directive and the journal.
    assert_eq!(
        visits
            .iter()
            .map(|(definitions, _)| *definitions)
            .collect::<Vec<_>>(),
        [7, 7],
        "PC-1: definitions query work is constant in unrelated pages"
    );
    assert_eq!(
        visits
            .iter()
            .map(|(_, scanned)| *scanned)
            .collect::<Vec<_>>(),
        [5 + 8, 50 + 8],
        "an unindexed key (`id::`) scans every page"
    );
}

/// PC-1/PC-3: a page-anchored property-presence query enumerates no
/// fileless rows (a page without a file has no properties), however many
/// unrelated fileless references the graph holds, and answers as the scan.
#[test]
fn page_property_presence_enumerates_no_fileless_rows() {
    let fileless = |graph: &ReadSnapshot, source: &str, dialect, narrow| {
        candidates::FILELESS_ROWS.with(|count| count.set(0));
        let (answer, _) = run(graph, source, dialect, narrow);
        (answer, candidates::FILELESS_ROWS.with(|count| count.get()))
    };
    let mut enumerated = Vec::new();
    for ghosts in [3, 30] {
        let (_dir, store) = open_with(5, ghosts);
        let whole = store.whole_graph().unwrap();
        let graph: &ReadSnapshot = &whole.graph;
        let mut counts = Vec::new();
        for (source, dialect) in [
            ("(page-property tine.export)", QueryDialect::Og),
            ("(page-property tine.share)", QueryDialect::Og),
            (
                "@page and prop('tine.export') is not null",
                QueryDialect::Tql,
            ),
        ] {
            let (narrowed, rows) = fileless(graph, source, dialect, true);
            let (scanned, _) = fileless(graph, source, dialect, false);
            assert_eq!(narrowed, scanned, "{source}");
            assert!(!narrowed.contains("Ghost"), "{source}: {narrowed}");
            let today = JournalDate::today();
            let (query, _) = parse_query_text(source, dialect, today);
            let resolved = resolve_for_execution(&query, &ExecutionContext::none(), today);
            assert!(
                !plan(graph, resolved.query(), today, false)
                    .may_list_fileless(None, &graph.config().journal_format()),
                "{source}: even unknown fileless changes cannot touch a property-required plan"
            );
            counts.push(rows);
        }
        // The counter is live: a page query without a necessary condition
        // walks every fileless row.
        let (_, all) = fileless(
            graph,
            "(not (page-property tine.export))",
            QueryDialect::Og,
            true,
        );
        assert!(all >= ghosts, "{all} fileless rows for {ghosts} ghosts");
        enumerated.push(counts);
        drop(whole);
        store.close();
    }
    assert_eq!(
        enumerated,
        [[0, 0, 0], [0, 0, 0]],
        "PC-1: property presence builds no fileless rows"
    );
}

/// PC-1/PC-4: a conjunction narrows by its tightest constraint, never by the
/// evaluation order `cheapest_first` gives it. A property conjunct (indexed
/// `status::` on every unrelated page, or unindexed `id::`) beside a page
/// constraint, plain or an OR of page-anchored branches, in either order,
/// visits only the named pages; a rare key beside a wide OR visits only its
/// holders. Every answer equals the full scan.
#[test]
fn conjunction_narrows_by_its_tightest_constraint() {
    const COMPOUND: &str = r#"(or (and (page "U1") "unrelated") (and (page "U2") "more"))"#;
    // (query, dialect, candidate pages, a page the answer holds)
    let cases: [(String, QueryDialect, usize, &str); 11] = [
        (
            format!("(and (property id) {COMPOUND})"),
            QueryDialect::Og,
            2,
            "U1",
        ),
        (
            format!("(and {COMPOUND} (property id))"),
            QueryDialect::Og,
            2,
            "U1",
        ),
        (
            format!("(and (property status) {COMPOUND})"),
            QueryDialect::Og,
            2,
            "U1",
        ),
        (
            format!("(and {COMPOUND} (property status))"),
            QueryDialect::Og,
            2,
            "U1",
        ),
        (
            r#"(and (property status) (page "U1"))"#.into(),
            QueryDialect::Og,
            1,
            "U1",
        ),
        (
            r#"(and (page "U1") (property status))"#.into(),
            QueryDialect::Og,
            1,
            "U1",
        ),
        (
            r#"(and (property id) (page "U1"))"#.into(),
            QueryDialect::Og,
            1,
            "U1",
        ),
        // Page-anchored: the unindexed `tine.type::` and the indexed
        // `tine.export::` (four holders) beside two names.
        (
            "@page and prop('tine.type') is not null and (name = 'Typed' or name = 'U1')".into(),
            QueryDialect::Tql,
            2,
            "Typed",
        ),
        (
            "@page and (name = 'Typed' or name = 'U1') and prop('tine.type') is not null".into(),
            QueryDialect::Tql,
            2,
            "Typed",
        ),
        (
            "@page and prop('tine.export') is not null and (name = 'Header' or name = 'U1')".into(),
            QueryDialect::Tql,
            2,
            "Header",
        ),
        // The key is the tighter constraint: its one holder, not the OR's 3.
        (
            r#"(and (or (page "U1") (page "U2") (page "Spelled")) (property due-date))"#.into(),
            QueryDialect::Og,
            1,
            "Spelled",
        ),
    ];
    let mut counts = Vec::new();
    for unrelated in [5, 50] {
        let (_dir, store) = open_with(unrelated, 3);
        let whole = store.whole_graph().unwrap();
        let graph: &ReadSnapshot = &whole.graph;
        let mut row = Vec::new();
        for (source, dialect, _, holds) in &cases {
            candidates::FILELESS_ROWS.with(|count| count.set(0));
            let (narrowed, visited) = run(graph, source, *dialect, true);
            let fileless = candidates::FILELESS_ROWS.with(|count| count.get());
            let (scanned, _) = run(graph, source, *dialect, false);
            assert_eq!(narrowed, scanned, "{source}");
            assert!(narrowed.contains(holds), "{source}: {narrowed}");
            row.push((visited, fileless));
        }
        counts.push(row);
        drop(whole);
        store.close();
    }
    // A fileless row holds no property: a property conjunct walks none.
    let expected: Vec<(usize, usize)> =
        cases.iter().map(|(_, _, visits, _)| (*visits, 0)).collect();
    assert_eq!(
        counts,
        [expected.clone(), expected],
        "PC-1: a conjunction visits its tightest constraint's pages, (pages, fileless rows) per case"
    );
}

/// Timing probe for the receipt. Ignored: `TINE_KEY_PROBE_ROOT=<copy of a
/// generated 10k graph>` (`TINE_KEY_PROBE_ROUNDS`, default 7;
/// `TINE_KEY_PROBE_QUERY`, an OG query, default the export definitions),
/// `--release`, run alone. Prints the median wall time of the query scanning
/// every page (the path before narrowing) and narrowed, rounds interleaved,
/// with the candidate pages of each.
#[test]
#[ignore]
fn definitions_query_at_scale() {
    let root = std::path::PathBuf::from(std::env::var("TINE_KEY_PROBE_ROOT").unwrap());
    let rounds: usize = std::env::var("TINE_KEY_PROBE_ROUNDS")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(7);
    let store = crate::Store::open(&root, Default::default()).unwrap().0;
    let whole = store.whole_graph().unwrap();
    let graph: &ReadSnapshot = &whole.graph;
    let source = std::env::var("TINE_KEY_PROBE_QUERY")
        .unwrap_or_else(|_| tine_core::export_definition::definitions_query());
    let (first, _) = run(graph, &source, QueryDialect::Og, true);
    let (mut scan, mut narrow) = (Vec::new(), Vec::new());
    let mut visited = (0, 0);
    for _ in 0..rounds {
        for (narrowed, times) in [(false, &mut scan), (true, &mut narrow)] {
            let started = std::time::Instant::now();
            let (answer, pages) = run(graph, &source, QueryDialect::Og, narrowed);
            times.push(started.elapsed().as_micros());
            assert_eq!(answer, first);
            if narrowed {
                visited.1 = pages
            } else {
                visited.0 = pages
            }
        }
    }
    let median = |mut v: Vec<u128>| {
        v.sort_unstable();
        v[v.len() / 2]
    };
    println!(
        "DEFINITIONS rounds={rounds} scan_us={} narrowed_us={} candidates(scan,narrowed)={visited:?}",
        median(scan),
        median(narrow)
    );
    drop(whole);
    store.close();
}

/// The open-threads review (DESIGN §12, `src/commentReviewQuery.ts`): the printed queries in
/// `tests/fixtures/comment-review-queries.json` list open margin-comment roots, leave out
/// resolved threads and replies, filter by page and author, and ride the `tine.quote` key
/// table: the pages handed to the row loop are the pages holding comments, not the graph.
#[test]
fn open_threads_review_lists_open_roots_and_visits_only_pages_with_comments() {
    #[derive(serde::Deserialize)]
    struct Cases {
        cases: Vec<Case>,
    }
    #[derive(serde::Deserialize)]
    struct Case {
        scope: String,
        page: String,
        author: Option<String>,
        query: String,
    }
    let cases: Cases = serde_json::from_str(include_str!(
        "../../tests/fixtures/comment-review-queries.json"
    ))
    .unwrap();
    let query_for = |scope: &str, author: Option<&str>| {
        cases
            .cases
            .iter()
            .find(|c| c.scope == scope && c.author.as_deref() == author && c.page != "Say \"hi\"")
            .unwrap_or_else(|| panic!("fixture case {scope} {author:?}"))
            .query
            .clone()
    };
    let notes = "- Alpha passage\n  - Open thread by Ada\n    tine.quote:: Alpha\n    tine.author:: Ada\n    tine.date:: 2026-11-07T13:37Z\n    - Reply from Ben\n      tine.author:: Ben\n      tine.date:: 2026-11-08T09:00Z\n  - Resolved thread by Ada\n    tine.quote:: passage\n    tine.author:: Ada\n    tine.resolved:: true\n- Another passage\n  - Open thread by Bob\n    tine.quote:: Another\n    tine.author:: Bob\n";
    let other =
        "- Beta\n  - Other page thread by Ada\n    tine.quote:: Beta\n    tine.author:: Ada\n";
    let mut visited_by_size = Vec::new();
    for unrelated in [30, 120] {
        let (dir, store) = open(unrelated);
        std::fs::write(dir.path().join("pages/Notes.md"), notes).unwrap();
        std::fs::write(dir.path().join("pages/Other.md"), other).unwrap();
        drop(store);
        let store = crate::Store::open(dir.path(), Default::default())
            .unwrap()
            .0;
        let whole = store.whole_graph().unwrap();
        let graph: &ReadSnapshot = &whole.graph;
        let listed = |answer: &str| -> Vec<&'static str> {
            [
                "Open thread by Ada",
                "Open thread by Bob",
                "Other page thread by Ada",
                "Resolved thread by Ada",
                "Reply from Ben",
                "unrelated",
            ]
            .into_iter()
            .filter(|text| answer.contains(text))
            .collect()
        };
        let (everything, graph_visits) =
            run(graph, &query_for("graph", None), QueryDialect::Og, true);
        assert_eq!(
            listed(&everything),
            [
                "Open thread by Ada",
                "Open thread by Bob",
                "Other page thread by Ada"
            ],
            "graph scope lists open roots only (no resolved thread, no reply)"
        );
        let (on_page, page_visits) = run(graph, &query_for("page", None), QueryDialect::Og, true);
        assert_eq!(
            listed(&on_page),
            ["Open thread by Ada", "Open thread by Bob"]
        );
        let (by_ada, _) = run(
            graph,
            &query_for("graph", Some("Ada")),
            QueryDialect::Og,
            true,
        );
        assert_eq!(
            listed(&by_ada),
            ["Open thread by Ada", "Other page thread by Ada"]
        );
        let (page_by_ada, _) = run(
            graph,
            &query_for("page", Some("Ada")),
            QueryDialect::Og,
            true,
        );
        assert_eq!(listed(&page_by_ada), ["Open thread by Ada"]);
        // The narrowing is an evaluation-order change only: the full scan answers the same.
        let (scanned, scan_visits) = run(graph, &query_for("graph", None), QueryDialect::Og, false);
        assert_eq!(scanned, everything);
        assert!(
            scan_visits > graph_visits,
            "the scan would have walked {scan_visits} pages"
        );
        visited_by_size.push((graph_visits, page_visits));
        drop(whole);
        store.close();
    }
    // Pages holding comments (Notes, Other), and Notes alone for the page scope, whatever
    // the graph holds: 30 or 120 comment-free pages add no candidate.
    assert_eq!(visited_by_size, [(2, 1), (2, 1)]);
}
