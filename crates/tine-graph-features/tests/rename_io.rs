//! GH #623 (QF3b): on Windows every file open is scanned, so a rename's cost
//! is its opens. QF3 measured eight successful read opens per rewritten
//! referrer and a planner that built the whole-graph name inventory. og's
//! contract: the planner's name work does not grow with the page count, each
//! referrer's references are rewritten once (not again under the writer
//! lock), and each rewritten referrer is opened a constant four times: the
//! planner's read, the preflight base-revision stage, the final pre-rename
//! guard, and the publication read (I-13, I-25).
use std::fs;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};
use tine_graph_features::pages;
use tine_store::cost_counters::{self, Counts};
use tine_store::Store;

static CASE_LOCK: Mutex<()> = Mutex::new(());

/// `pages_count` unrelated pages, `referrers` pages linking `[[Target]]`, and
/// the target. Every file is backdated past the watcher's racy window so a
/// fresh fixture's freshness re-hash is not counted as rename work.
fn rename(pages_count: usize, referrers: usize) -> Counts {
    let _case = CASE_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let temp = tempfile::Builder::new()
        .prefix("rename-io-")
        .tempdir()
        .unwrap();
    let root = temp.path().to_path_buf();
    fs::create_dir_all(root.join("pages")).unwrap();
    let mut files = Vec::new();
    for index in 0..pages_count {
        files.push((
            format!("pages/Other{index:05}.md"),
            format!("- unrelated [[Other{}]] {index}\n", index + 1),
        ));
    }
    for index in 0..referrers {
        files.push((
            format!("pages/Ref{index:05}.md"),
            format!("- links [[Target]] {index}\n"),
        ));
    }
    files.push(("pages/Target.md".into(), "- the target\n".into()));
    let old = SystemTime::now() - Duration::from_secs(3600);
    for (path, text) in &files {
        let path = root.join(path);
        fs::write(&path, text).unwrap();
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(old)
            .unwrap();
    }
    let store = Store::open(&root, Default::default()).unwrap().0;
    store.whole_graph().unwrap();
    cost_counters::reset();
    pages::rename_page_expected(&store, "Target", "Renamed", None).unwrap();
    let counts = cost_counters::snapshot();
    assert!(root.join("pages/Renamed.md").exists());
    assert_eq!(
        fs::read_to_string(root.join("pages/Ref00001.md")).unwrap(),
        "- links [[Renamed]] 1\n"
    );
    store.close();
    counts
}

#[test]
fn rename_planner_name_work_is_flat_in_graph_size() {
    let small = rename(60, 2);
    let large = rename(600, 2);
    assert_eq!(
        large.name_inventory_entries, small.name_inventory_entries,
        "GH #623/I-13: the rename planner needs only the renamed page's own files, alias \
         collisions and referrers; building the whole-graph name inventory made planning \
         grow with the graph (exemplar store/inventory.rs page_files_at_or_under): \
         {small:?} vs {large:?}"
    );
}

#[test]
fn rename_rewrites_each_referrer_once_outside_the_writer() {
    let few = rename(60, 2);
    let many = rename(60, 30);
    assert_eq!(
        many.transaction_rewrites, few.transaction_rewrites,
        "GH #623/I-25: a referrer's rewrite is prepared once by the planner and handed to \
         preflight, which reuses it only when the staged bytes equal the prepared old bytes \
         (exemplar transaction/prepared.rs): {few:?} vs {many:?}"
    );
}

#[test]
fn rename_reads_each_referrer_once_per_guard() {
    let few = rename(60, 2);
    let many = rename(60, 30);
    assert!(
        many.full_reads - few.full_reads <= 3 * 28,
        "GH #623/I-25: each extra referrer costs three whole-file reads under the writer: \
         the preflight base-revision stage, the final pre-rename guard and the publication \
         read; a separate stage-2 verify of a changing rewrite repeated the pre-rename \
         guard's comparison (exemplar transaction/read_checks.rs): {few:?} vs {many:?}"
    );
    assert!(
        many.store_reads - few.store_reads <= 28,
        "GH #623/I-25: the planner reads each referrer once: {few:?} vs {many:?}"
    );
}

#[test]
fn rename_publication_reopens_no_referrer() {
    let few = rename(60, 2);
    let many = rename(60, 30);
    assert_eq!(
        (many.preamble_reads + many.hash_reads) - (few.preamble_reads + few.hash_reads),
        0,
        "GH #623/I-25: publication derives the effective name and own stamp from the bytes \
         it already read; re-opening each written file for its preamble, the projection's \
         preamble and the own-write hash cost three opens per referrer \
         (exemplar transaction/publication.rs): {few:?} vs {many:?}"
    );
    assert!(
        many.stamps_by_path - few.stamps_by_path <= 28,
        "GH #623/I-25: one metadata stamp per written referrer (the own-write check); \
         the publication read's handle supplies the stamp it is compared with: \
         {few:?} vs {many:?}"
    );
}
