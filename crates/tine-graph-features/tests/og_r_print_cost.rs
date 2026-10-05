//! Checkpoint-5 packet R, I-13: a single-page print is single-page work.
//! Own test binary: the cost counters are process-global, so no other test may
//! run beside it.
use std::fs;
use std::path::PathBuf;
use tine_graph_features::print;
use tine_store::cost_counters;
use tine_store::Store;

fn graph(pages: &[(String, String)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "tine-ogr-cost-{}-{}",
        std::process::id(),
        pages.len()
    ));
    let _ = fs::remove_dir_all(&dir);
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
    store.scan_refresh().unwrap();
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
