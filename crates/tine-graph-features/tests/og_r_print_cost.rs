//! Checkpoint-5 packet R, I-13: a single-page print is single-page work.
//! Own test binary: the cost counters are process-global, so no other test may
//! run beside it.
use std::fs;
use std::path::PathBuf;
use tine_graph_features::print;
use tine_store::cost_counters;
use tine_store::Store;

fn graph(pages: &[(String, String)]) -> PathBuf {
    let dir = crate::test_support::fresh_dir("tine-ogr-cost-");
    for sub in ["pages", "journals", "logseq"] {
        fs::create_dir_all(dir.join(sub)).unwrap();
    }
    fs::write(dir.join("logseq/config.edn"), "{}\n").unwrap();
    for (file, text) in pages {
        fs::write(dir.join("pages").join(file), text).unwrap();
    }
    dir
}

fn print_counts(unrelated: usize) -> cost_counters::Counts {
    let mut all: Vec<(String, String)> = (0..unrelated)
        .map(|i| (format!("Other{i:04}.md"), format!("- unrelated {i}\n")))
        .collect();
    all.push(("Target.md".into(), "- the printed page\n".into()));
    let dir = graph(&all);
    let store = Store::open(&dir, Default::default()).unwrap().0;
    // Finish the launch reconciliation first: its by-path stamps must not be
    // billed to the print.
    store.whole_graph().unwrap();
    store.refresh(tine_store::Depth::Stamps).unwrap();
    cost_counters::reset();
    let html = print::page_print_html(&store, "Target", Default::default())
        .unwrap()
        .unwrap();
    let counts = cost_counters::snapshot();
    assert!(html.contains("the printed page"));
    store.close();
    let _ = fs::remove_dir_all(&dir);
    counts
}

/// I-13: a single-page print does not walk the graph directory or re-stamp
/// every graph file to find out what it was asked to print.
#[test]
fn single_page_print_does_no_graph_wide_disk_work() {
    let _integration_guard = crate::exclusive_integration_lock();
    let few = print_counts(20);
    let many = print_counts(200);
    eprintln!("print cost: 20 pages {few:?}\n200 pages {many:?}");
    assert_eq!(
        many.stamps_by_path, few.stamps_by_path,
        "I-13: a single-page print must not stat every graph file; exemplar print.rs page_print_html"
    );
    assert_eq!(
        many.readdir, few.readdir,
        "I-13: no directory walk per print"
    );
}

/// The app's whole print flow for one page that embeds a block and a page of
/// another file and references a third block: sheet inputs, then the print
/// document. Returns the counts and the HTML.
fn full_print_counts(unrelated: usize) -> (cost_counters::Counts, String) {
    let mut all: Vec<(String, String)> = (0..unrelated)
        .map(|i| (format!("Other{i:04}.md"), format!("- unrelated {i}\n")))
        .collect();
    all.push((
        "Source.md".into(),
        "- embedded block text\n  id:: 6553f1a0-0000-4000-8000-000000000001\n- second\n".into(),
    ));
    all.push(("Whole.md".into(), "- embedded page text\n".into()));
    all.push((
        "Target.md".into(),
        "- the printed page\n- {{embed ((6553f1a0-0000-4000-8000-000000000001))}}\n- {{embed [[Whole]]}}\n".into(),
    ));
    let dir = graph(&all);
    let store = Store::open(&dir, Default::default()).unwrap().0;
    store.whole_graph().unwrap();
    store.refresh(tine_store::Depth::Stamps).unwrap();
    cost_counters::reset();
    let sheets = tine_graph_features::publish::sheet_export_inputs(
        &store,
        Some(&["Target".to_owned()]),
        None,
    )
    .unwrap();
    assert!(sheets.is_empty());
    let html = print::page_print_html_with_sheets(&store, "Target", Default::default(), Vec::new())
        .unwrap()
        .unwrap();
    let counts = cost_counters::snapshot();
    store.close();
    let _ = fs::remove_dir_all(&dir);
    (counts, html)
}

/// Cost-model audit 2026-11-02 row 5 (D-10): printing one page (its sheet
/// inputs and its document) costs the page and what it embeds, never a stat
/// of every graph file or a copy of every page.
#[test]
fn whole_print_flow_with_embeds_does_no_graph_wide_work() {
    let _integration_guard = crate::exclusive_integration_lock();
    let (few, few_html) = full_print_counts(20);
    let (many, many_html) = full_print_counts(200);
    eprintln!("full print cost: 20 pages {few:?}\n200 pages {many:?}");
    for html in [&few_html, &many_html] {
        assert!(html.contains("the printed page"));
        assert!(html.contains("embedded block text"), "block embed resolves");
        assert!(html.contains("embedded page text"), "page embed resolves");
    }
    assert_eq!(
        many.stamps_by_path, few.stamps_by_path,
        "a one-page print must not stat every graph file (sheet inputs refreshed the graph)"
    );
    assert_eq!(many.readdir, few.readdir, "no directory walk per print");
    assert_eq!(
        (few.corpus, many.corpus),
        (0, 0),
        "a one-page print must not copy every page into a corpus"
    );
}

fn namespace_print(unrelated: usize) -> (cost_counters::Counts, String) {
    let mut all: Vec<_> = (0..unrelated)
        .map(|i| (format!("Other{i:04}.md"), "- unrelated\n".to_owned()))
        .collect();
    all.extend([
        ("Target.md".into(), "- {{namespace Project}}\n".into()),
        ("Child.md".into(), "title:: Project/Child\n- child\n".into()),
        ("Lower.md".into(), "title:: project/y\n- lower\n".into()),
    ]);
    let dir = graph(&all);
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(60);
    for (file, _) in &all {
        fs::File::options()
            .write(true)
            .open(dir.join("pages").join(file))
            .unwrap()
            .set_modified(old)
            .unwrap();
    }
    let store = Store::open(&dir, Default::default()).unwrap().0;
    store.whole_graph().unwrap();
    store.refresh(tine_store::Depth::Stamps).unwrap();
    cost_counters::reset();
    let html = print::page_print_html(&store, "Target", Default::default())
        .unwrap()
        .unwrap();
    let counts = cost_counters::snapshot();
    // Time the waited print operation, excluding fixture construction and launch.
    let started = std::time::Instant::now();
    for _ in 0..30 {
        print::page_print_html(&store, "Target", Default::default())
            .unwrap()
            .unwrap();
    }
    let elapsed = started.elapsed();
    eprintln!("namespace print {unrelated}: {counts:?}; 30 prints {elapsed:?}");
    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "30 namespace prints exceeded the timing budget: {elapsed:?}"
    );
    store.close();
    let _ = fs::remove_dir_all(dir);
    (counts, html)
}

#[test]
fn namespace_print_cost_is_bounded() {
    let _integration_guard = crate::exclusive_integration_lock();
    let (few, few_html) = namespace_print(20);
    let (many, many_html) = namespace_print(200);
    assert_eq!(
        (few.corpus, many.corpus),
        (0, 0),
        "namespace printing must not copy the corpus"
    );
    assert_eq!(
        (few.namespace_name_visits, many.namespace_name_visits),
        (2, 2),
        "only matching stored names are visited"
    );
    assert_eq!(
        (few.full_reads, many.full_reads),
        (1, 1),
        "only the printed target is read"
    );
    assert_eq!(
        (
            few.files_written,
            many.files_written,
            few.fsyncs,
            many.fsyncs
        ),
        (0, 0, 0, 0)
    );
    assert_eq!(few.stamps_by_path, many.stamps_by_path);
    assert_eq!(few.readdir, many.readdir);
    for html in [few_html, many_html] {
        assert!(html.contains(">Project/Child</a>"));
        assert!(
            html.contains(">project/y</a>"),
            "identity membership is case insensitive"
        );
    }
}
