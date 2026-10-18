//! TQL `links`, `linked_by` and `near` (docs/contracts/tql-links-near.md):
//! direction cases on a hand graph, pages without a file, rename survival of
//! seeds, and parity of `links` with the Linked References panel.

use std::collections::BTreeSet;
use std::path::Path;

use tine_core::query::ir::{ExecutionContext, QueryRows};
use tine_core::query::registry::Registry;
use tine_core::query::{parse_query_input, QueryInput};
use tine_core::refs::page_key;
use tine_store::{Cancel, IrAnswer, IrRequest, Resolved, Store, WholeGraph};

fn write(root: &Path, rel: &str, text: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// The page keys a `@page` TQL query selects.
fn pages(graph: &WholeGraph, tql: &str) -> BTreeSet<String> {
    let (query, view) = parse_query_input(
        tql,
        QueryInput::Tql,
        tine_core::date::JournalDate::today(),
        Registry::none(),
    );
    assert!(!query.is_invalid(), "{tql}: {:?}", query.diagnostics);
    let IrAnswer::Result(result) = graph
        .query_ir(IrRequest::Run {
            query: &query,
            view: &view,
            context: &ExecutionContext::default(),
        })
        .unwrap()
    else {
        panic!("query_ir(Run) returns a result");
    };
    assert!(!result.exceeded, "{tql}");
    let QueryRows::Page { pages } = result.rows else {
        panic!("{tql} is page-anchored");
    };
    pages.into_iter().map(|row| page_key(&row.name)).collect()
}

fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| page_key(name)).collect()
}

/// The page keys of `target`'s Linked References panel (unfiltered).
fn panel(graph: &WholeGraph, target: &str) -> BTreeSet<String> {
    graph
        .backlinks(target)
        .unwrap()
        .iter()
        .map(|group| page_key(&group.page))
        .collect()
}

fn quoted(name: &str) -> String {
    format!("'{}'", name.replace('\'', "''"))
}

/// Cycle a→b→c→a, 2-cycle d↔e, the "not undirected" pair x→y←z, a hub, a
/// journal hub, a page without a file, an alias, a namespace and a self link.
fn hand_graph(root: &Path) {
    write(
        root,
        "pages/a.md",
        "- to [[b]] and to myself [[a]] and to [[idea]]\n",
    );
    write(root, "pages/b.md", "- to [[c]]\n");
    write(root, "pages/c.md", "- to #a\n");
    write(root, "pages/d.md", "- to [[e]]\n");
    write(root, "pages/e.md", "- to [[d]]\n");
    write(root, "pages/x.md", "- to [[y]]\n");
    write(root, "pages/y.md", "- leaf\n");
    write(root, "pages/z.md", "- to [[y]]\n");
    write(root, "pages/hub.md", "- [[h1]] [[h2]]\n  - nested [[h3]]\n");
    write(root, "pages/h1.md", "- h1\n");
    write(root, "pages/h2.md", "- h2\n");
    write(root, "pages/h3.md", "- h3\n");
    write(root, "pages/p1.md", "tags:: hub\n\n- p1\n");
    write(root, "pages/p2.md", "- [[hub]]\n");
    write(root, "pages/p3.md", "- #[[hub]]\n");
    // The seed mentions a journal day, the day mentions everything else.
    write(root, "pages/seed.md", "- see [[Oct 1st, 2026]]\n");
    write(
        root,
        "journals/2026_10_01.md",
        "- [[seed]] then [[q1]] and [[q2]]\n",
    );
    write(root, "pages/q1.md", "- q1\n");
    write(root, "pages/q2.md", "- q2\n");
    write(root, "pages/r.md", "- the day [[Oct 1st, 2026]]\n");
    write(root, "pages/w.md", "- [[idea]] and [[Oct 2nd, 2026]]\n");
    write(root, "pages/target.md", "alias:: tt\n\n- target\n");
    write(root, "pages/u.md", "- [[tt]]\n");
    write(root, "pages/n1.md", "- [[ns/child]]\n");
}

fn open(root: &Path) -> (Store, WholeGraph) {
    let store = Store::open(root, Default::default()).unwrap().0;
    let graph = store.whole_graph().unwrap();
    (store, graph)
}

#[test]
fn links_near_directions_on_a_hand_graph() {
    let _integration_guard = crate::integration_lock();
    let dir = tempfile::tempdir().unwrap();
    hand_graph(dir.path());
    let (_store, graph) = open(dir.path());
    let q = |tql: &str| pages(&graph, tql);

    // Cycle a→b→c→a.
    assert_eq!(q("@page and near(name = 'a', 0)"), set(&["a"]));
    assert_eq!(q("@page and near(name = 'a', 1, 'out')"), set(&["a", "b"]));
    assert_eq!(q("@page and near(name = 'a', 1, 'in')"), set(&["a", "c"]));
    assert_eq!(q("@page and near(name = 'a', 1)"), set(&["a", "b", "c"]));
    assert_eq!(
        q("@page and near(name = 'a', 2, 'out')"),
        set(&["a", "b", "c"])
    );
    assert_eq!(
        q("@page and near(name = 'a', 99, 'in')"),
        set(&["a", "b", "c"])
    );
    // 2-cycle.
    assert_eq!(q("@page and near(name = 'd', 1, 'out')"), set(&["d", "e"]));
    assert_eq!(q("@page and near(name = 'd', 5)"), set(&["d", "e"]));
    // 'both' keeps one direction per path: z links to what x links to, and is
    // NOT within 2 steps of x (it would be in the undirected ball).
    assert_eq!(q("@page and near(name = 'x', 2)"), set(&["x", "y"]));
    assert_eq!(
        q("@page and near(name = 'y', 1, 'in')"),
        set(&["x", "y", "z"])
    );
    // Hub: nested blocks and page-properties `tags::` are links too.
    assert_eq!(
        q("@page and near(name = 'hub', 1, 'out')"),
        set(&["hub", "h1", "h2", "h3"])
    );
    assert_eq!(
        q("@page and near(name = 'hub', 1, 'in')"),
        set(&["hub", "p1", "p2", "p3"])
    );
    // Journal hub: the day is selected but, through `journal = false`, does not
    // drag in what it mentions (include-but-stop).
    let day = "oct 1st, 2026";
    assert_eq!(
        q("@page and near(name = 'seed', 2, 'out')"),
        set(&["seed", day, "q1", "q2"])
    );
    assert_eq!(
        q("@page and near(name = 'seed', 2, 'out', through => journal = false)"),
        set(&["seed", day])
    );
    assert_eq!(
        q("@page and near(name = 'seed', 2, 'in')"),
        set(&["seed", day, "r"])
    );
    assert_eq!(
        q("@page and near(name = 'seed', 2, 'in', through => journal = false)"),
        set(&["seed", day])
    );
    // Seeds always expand, whatever `through` says; exclusion is a conjunct.
    assert_eq!(
        q("@page and near(name = 'seed', 1, through => false)"),
        set(&["seed", day])
    );
    assert_eq!(
        q("@page and near(name = 'seed', 2, through => journal = false) and journal = false"),
        set(&["seed"])
    );
    // Several seeds.
    assert_eq!(
        q("@page and near(name in ('d', 'x'), 1, 'out')"),
        set(&["d", "e", "x", "y"])
    );

    // links / linked_by and their quantifiers.
    assert_eq!(q("@page and any(links, name = 'b')"), set(&["a"]));
    assert_eq!(q("@page and any(linked_by, name = 'a')"), set(&["b"]));
    // A self link is not a link (the panel never lists a page under itself).
    assert!(!q("@page and any(links, name = 'a')").contains("a"));
    // An alias: a link to `tt` is a link to `target`.
    assert_eq!(q("@page and any(links, name = 'target')"), set(&["u"]));
    // A namespace link also reaches its parent.
    assert_eq!(q("@page and any(links, name = 'ns')"), set(&["n1"]));
    assert_eq!(
        q("@page and every(links, journal = true) and any(links, true)"),
        set(&["seed", "r"])
    );
    let orphans = q("@page and none(linked_by, true)");
    for linked in ["a", "b", "c", "y", "hub", "h1", "seed", "q1", "target"] {
        assert!(!orphans.contains(linked), "{linked}");
    }
    for orphan in ["x", "z", "p2", "u", "w"] {
        assert!(orphans.contains(orphan), "{orphan}");
    }
    // Nested, innermost first: pages linking to a page some journal day links
    // to. The day links to seed, q1 and q2, and only the day links to those.
    assert_eq!(
        q("@page and any(links, any(linked_by, journal = true))"),
        set(&[day])
    );
    // At @block a link condition does not apply: kept and reported.
    let (query, _) = parse_query_input(
        "near(name = 'hub', 1) and content like '%nested%'",
        QueryInput::Tql,
        tine_core::date::JournalDate::today(),
        Registry::none(),
    );
    assert!(query.is_invalid());
}

#[test]
fn links_near_pages_without_a_file_are_seeds_and_elements() {
    let _integration_guard = crate::integration_lock();
    let dir = tempfile::tempdir().unwrap();
    hand_graph(dir.path());
    let (_store, graph) = open(dir.path());
    let q = |tql: &str| pages(&graph, tql);
    // `idea` has no file; its panel lists a and w.
    assert_eq!(panel(&graph, "idea"), set(&["a", "w"]));
    assert_eq!(
        q("@page and near(name = 'idea', 1, 'in')"),
        set(&["a", "w"])
    );
    assert_eq!(q("@page and any(links, name = 'idea')"), set(&["a", "w"]));
    // A fileless journal day is a journal with its day.
    assert_eq!(
        q("@page and any(links, journal = true and day = '2026-10-02')"),
        set(&["w"])
    );
    assert_eq!(
        q("@page and any(links, name = 'Oct 2nd, 2026')"),
        set(&["w"])
    );
    // It never links anywhere: two steps in from `idea` stop at its sources'
    // own sources, never through `idea` outwards.
    assert_eq!(
        q("@page and near(name = 'idea', 2, 'out')"),
        BTreeSet::new()
    );
}

/// Review r1 blocker: a nested link condition evaluated ON a page without a
/// file (as an element of an outer `links`) must see that page's own links.
#[test]
fn links_near_nested_conditions_see_a_page_without_a_file() {
    let _integration_guard = crate::integration_lock();
    let dir = tempfile::tempdir().unwrap();
    hand_graph(dir.path());
    let (_store, graph) = open(dir.path());
    let q = |tql: &str| pages(&graph, tql);
    assert_eq!(
        q("@page and any(links, near(name = 'idea', 0))"),
        set(&["a", "w"])
    );
    assert_eq!(
        q("@page and any(links, any(linked_by, name = 'a')) and name in ('a', 'w')"),
        set(&["a", "w"])
    );
    assert_eq!(
        q("@page and any(links, name = 'idea' and any(linked_by, name = 'w'))"),
        set(&["a", "w"])
    );
    // The fileless journal day too.
    assert_eq!(
        q("@page and any(links, journal = true and none(links, true) and any(linked_by, name = 'w'))"),
        set(&["w"])
    );
}

#[test]
fn links_near_an_edit_anywhere_moves_a_link_answer() {
    let _integration_guard = crate::integration_lock();
    let dir = tempfile::tempdir().unwrap();
    hand_graph(dir.path());
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    let tql = "@page and near(name = 'y', 1, 'in')";
    assert_eq!(
        pages(&store.whole_graph().unwrap(), tql),
        set(&["x", "y", "z"])
    );
    // An external editor adds a link on a page the answer never contained.
    write(dir.path(), "pages/h1.md", "- now [[y]]\n");
    store.refresh(tine_store::Depth::Stamps).unwrap();
    assert_eq!(
        pages(&store.whole_graph().unwrap(), tql),
        set(&["x", "y", "z", "h1"])
    );
}

#[test]
fn links_near_seeds_survive_a_rename_inside_a_definition() {
    let _integration_guard = crate::integration_lock();
    let dir = tempfile::tempdir().unwrap();
    hand_graph(dir.path());
    let definition =
        "- {{tine-query @page and near(name = 'hub', 1, 'in')}}\n  export:: hub-site\n";
    write(dir.path(), "pages/sites.md", definition);
    let store = std::sync::Arc::new(Store::open(dir.path(), Default::default()).unwrap().0);
    let app_data = tempfile::tempdir().unwrap();
    let host = tine_store::PageHost::start_for_tests(&store, app_data.path()).unwrap();
    tine_graph_features::pages::rename_page_expected(&store, &host, "hub", "Central", None)
        .unwrap();
    let text = std::fs::read_to_string(dir.path().join("pages/sites.md")).unwrap();
    assert!(
        text.contains("near(name = 'Central', 1, 'in')"),
        "the seed follows the rename: {text}"
    );
    // The renamed query still selects the same neighbourhood.
    let graph = store.whole_graph().unwrap();
    assert_eq!(
        pages(&graph, "@page and near(name = 'Central', 1, 'in')"),
        set(&["central", "p1", "p2", "p3"])
    );
}

/// Every name a parity check visits: each page's own name, and each explicit
/// reference spelling (with its namespace parents) that no file and no alias
/// claims — the pages without a file.
fn parity(root: &Path) -> (usize, usize) {
    let (_store, graph) = open(root);
    let corpus = graph.corpus();
    let mut physical: BTreeSet<String> = BTreeSet::new();
    let mut fileless: BTreeSet<String> = BTreeSet::new();
    let format = tine_core::date::JournalFormat::new(None, None);
    for page in &corpus.pages {
        physical.insert(page.name.clone());
        let mut stack: Vec<&tine_core::doc::DocBlock> = page.document.roots.iter().collect();
        while let Some(block) = stack.pop() {
            for reference in block.projection().reference_source().explicit {
                let key = page_key(&reference.name);
                for name in tine_core::refs::namespace_refs(&key) {
                    if format.parse(name).is_none()
                        && matches!(graph.resolve(name, false), Resolved::Absent { .. })
                    {
                        fileless.insert(name.to_owned());
                    }
                }
            }
            stack.extend(block.children.iter());
        }
    }
    let mut checked = 0;
    let mut mismatches = Vec::new();
    for name in physical.iter().chain(fileless.iter()) {
        let expected = panel(&graph, name);
        let actual = pages(
            &graph,
            &format!("@page and any(links, name = {})", quoted(name)),
        );
        checked += 1;
        if expected != actual {
            mismatches.push((name.clone(), expected, actual));
        }
    }
    assert!(
        mismatches.is_empty(),
        "{} of {checked} panels differ from `links`; first: {:?}",
        mismatches.len(),
        mismatches.first()
    );
    (physical.len(), fileless.len())
}

fn copy_graph(from: &Path, to: &Path) {
    for area in ["pages", "journals", "logseq/config.edn"] {
        let source = from.join(area);
        if source.is_file() {
            write(to, area, &std::fs::read_to_string(&source).unwrap());
            continue;
        }
        if !source.is_dir() {
            continue;
        }
        let mut stack = vec![source.clone()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let entry = entry.unwrap();
                let path = entry.path();
                if entry.file_type().unwrap().is_dir() {
                    stack.push(path);
                } else {
                    let rel = path.strip_prefix(from).unwrap();
                    let dest = to.join(rel);
                    std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
                    std::fs::copy(&path, dest).unwrap();
                }
            }
        }
    }
}

#[test]
fn links_match_the_linked_references_panel_on_the_hand_graph_and_kitchen_sink() {
    let _integration_guard = crate::integration_lock();
    let dir = tempfile::tempdir().unwrap();
    hand_graph(dir.path());
    write(
        dir.path(),
        "pages/kitchen sink.md",
        include_str!("../../../src/fixtures/kitchen-sink.md"),
    );
    let (physical, fileless) = parity(dir.path());
    assert!(physical > 20 && fileless > 3, "{physical} {fileless}");
}

/// The maintainer's corpora (AGENTS.md §4), read from a COPY. Run with
/// `cargo test -p tine-store --test integration links_match_the_panel_on_corpora -- --ignored`.
#[test]
#[ignore = "needs ~/research/tine-test and ~/research/logseq-anonymized"]
fn links_match_the_panel_on_corpora() {
    let _integration_guard = crate::integration_lock();
    let home = std::env::var("HOME").unwrap();
    for corpus in ["research/tine-test", "research/logseq-anonymized"] {
        let source = Path::new(&home).join(corpus);
        assert!(source.is_dir(), "{corpus} missing");
        let dir = tempfile::tempdir().unwrap();
        copy_graph(&source, dir.path());
        let started = std::time::Instant::now();
        let (physical, fileless) = parity(dir.path());
        eprintln!(
            "{corpus}: {physical} pages + {fileless} fileless names, panel == links, {:?}",
            started.elapsed()
        );
    }
}

fn rss_kib(field: &str) -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .unwrap()
        .lines()
        .find_map(|line| line.strip_prefix(field))
        .and_then(|rest| rest.trim().trim_end_matches("kB").trim().parse().ok())
        .unwrap_or(0)
}

/// k = 2 on the realistic 10k generator. Run with
/// `TINE_LINKS_BENCH_GRAPH=<dir from scripts/generate-realistic-graph.mjs>
/// cargo test --release -p tine-store --test integration links_near_k2_on_the_realistic_graph -- --ignored --nocapture`.
#[test]
#[ignore = "needs TINE_LINKS_BENCH_GRAPH (scripts/generate-realistic-graph.mjs)"]
fn links_near_k2_on_the_realistic_graph() {
    let _integration_guard = crate::integration_lock();
    let root = std::env::var("TINE_LINKS_BENCH_GRAPH").expect("TINE_LINKS_BENCH_GRAPH");
    let (_store, graph) = open(Path::new(&root));
    let names: Vec<String> = graph
        .corpus()
        .pages
        .iter()
        .filter(|page| page.kind == tine_core::model::PageKind::Page)
        .map(|page| page.name.clone())
        .collect();
    // Two seeds: an ordinary page, and the most linked-to of a sample (a hub,
    // the costly case for k = 2).
    let ordinary = names[names.len() / 2].clone();
    let hub = names
        .iter()
        .step_by((names.len() / 300).max(1))
        .max_by_key(|name| graph.backlinks(name).map_or(0, |groups| groups.len()))
        .unwrap()
        .clone();
    let rss_before = rss_kib("VmRSS:");
    let mut runs = vec![(
        "orphans (cold: builds the table)".to_owned(),
        "@page and none(linked_by, true)".to_owned(),
    )];
    for (label, seed) in [("ordinary", &ordinary), ("hub", &hub)] {
        let seed = quoted(seed);
        runs.extend([
            (
                format!("{label} k=2 both"),
                format!("@page and near(name = {seed}, 2)"),
            ),
            (
                format!("{label} k=2 both through journal = false"),
                format!("@page and near(name = {seed}, 2, through => journal = false)"),
            ),
            (
                format!("{label} k=2 out"),
                format!("@page and near(name = {seed}, 2, 'out')"),
            ),
            (
                format!("{label} k=2 in"),
                format!("@page and near(name = {seed}, 2, 'in')"),
            ),
        ]);
    }
    for (label, tql) in runs {
        let started = std::time::Instant::now();
        let selected = pages(&graph, &tql);
        eprintln!(
            "{label}: {} pages in {:?} (VmRSS +{} KiB)",
            selected.len(),
            started.elapsed(),
            rss_kib("VmRSS:").saturating_sub(rss_before)
        );
    }
    let (nodes, links, bytes) = graph.link_table_stats();
    eprintln!(
        "link table: {nodes} nodes, {links} links, ~{} KiB heap",
        bytes / 1024
    );
    // The table build alone, on a fresh view of an unchanged graph... the view
    // shares the snapshot, so time a rebuild through a new store instead.
    let (_store2, graph2) = open(Path::new(&root));
    let _ = graph2.corpus();
    let started = std::time::Instant::now();
    let _ = graph2.link_table_stats();
    eprintln!("link table build alone: {:?}", started.elapsed());
    eprintln!(
        "{} pages, VmHWM {} KiB",
        graph.corpus().pages.len(),
        rss_kib("VmHWM:")
    );
}

/// Review r1: a link query takes the flag interactive search takes; a set flag
/// answers nothing, and the same query then runs whole.
#[test]
fn links_near_a_set_cancel_flag_answers_nothing() {
    let _integration_guard = crate::integration_lock();
    let dir = tempfile::tempdir().unwrap();
    hand_graph(dir.path());
    let (_store, graph) = open(dir.path());
    let (query, view) = parse_query_input(
        "@page and near(name = 'idea', 1, 'in')",
        QueryInput::Tql,
        tine_core::date::JournalDate::today(),
        Registry::none(),
    );
    let context = ExecutionContext::default();
    let run = |cancelled: bool| {
        graph
            .query_ir_cancellable(
                IrRequest::Run {
                    query: &query,
                    view: &view,
                    context: &context,
                },
                &Cancel(std::sync::Arc::new(std::sync::atomic::AtomicBool::new(
                    cancelled,
                ))),
            )
            .unwrap()
    };
    assert!(run(true).is_none());
    let Some(IrAnswer::Result(result)) = run(false) else {
        panic!("an uncancelled run answers a result");
    };
    let QueryRows::Page { pages } = result.rows else {
        panic!("page rows");
    };
    let names: BTreeSet<String> = pages.iter().map(|row| page_key(&row.name)).collect();
    assert_eq!(names, set(&["a", "w"]));
}
