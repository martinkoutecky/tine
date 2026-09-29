//! GH #406 (master 05a4b0001c5b, 13fce7f25764): a page rename's disk bill is
//! linear in what it rewrites. Master listed every ancestor directory on each
//! referrer write, so the write phase cost referrers x pages. og's contract:
//! directory enumerations per rename do not grow with the referrer count or
//! the page count, whole-file reads do not grow with the page count, and each
//! referrer costs one write and a bounded number of guard reads (I-13, I-25).
use std::fs;
use std::sync::Mutex;
use tine_graph_features::pages;
use tine_store::cost_counters::{self, Counts};
use tine_store::Store;

static CASE_LOCK: Mutex<()> = Mutex::new(());

/// `pages` unrelated pages spread over three folders, `referrers` pages that
/// link `[[Target]]`, and the target itself.
fn rename(pages_count: usize, referrers: usize) -> Counts {
    // Self-deleting: dropped after `store` (declared later), and on a panic too.
    let temp = tempfile::Builder::new()
        .prefix("rename-cost-")
        .tempdir()
        .unwrap();
    let root = temp.path().to_path_buf();
    for dir in ["pages/a", "pages/b", "pages/c", "journals"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    for index in 0..pages_count {
        let folder = ["a", "b", "c"][index % 3];
        fs::write(
            root.join(format!("pages/{folder}/Other{index:05}.md")),
            format!("- unrelated {index}\n"),
        )
        .unwrap();
    }
    for index in 0..referrers {
        fs::write(
            root.join(format!("pages/Ref{index:05}.md")),
            format!("- links [[Target]] {index}\n"),
        )
        .unwrap();
    }
    fs::write(root.join("pages/Target.md"), "- the target\n").unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    store.whole_graph().unwrap();
    cost_counters::reset();
    pages::rename_page_expected(&store, "Target", "Renamed", None).unwrap();
    let counts = cost_counters::snapshot();
    assert!(root.join("pages/Renamed.md").exists());
    assert_eq!(
        fs::read_to_string(root.join("pages/Ref00000.md")).unwrap(),
        "- links [[Renamed]] 0\n"
    );
    store.close();
    counts
}

#[test]
fn rename_cost_is_linear_in_referrers_and_flat_in_graph_size() {
    let _case = CASE_LOCK.lock().unwrap();
    let few = rename(60, 2);
    let many = rename(60, 30);
    let large = rename(600, 2);
    eprintln!("GH #406 rename cost: 60p/2r={few:?}\n60p/30r={many:?}\n600p/2r={large:?}");
    assert_eq!(
        many.readdir, few.readdir,
        "GH #406/I-13: directory enumerations per rename must not grow with referrers \
         (master listed pages/ once per referrer write); exemplar transaction.rs Transaction::apply"
    );
    assert_eq!(
        large.readdir, few.readdir,
        "GH #406/I-13: directory enumerations per rename must not grow with the page count"
    );
    assert_eq!(
        large.full_reads, few.full_reads,
        "I-13: a rename must not re-read unrelated pages"
    );
    assert_eq!(
        many.files_written - few.files_written,
        28,
        "I-25: each extra referrer costs exactly one file write"
    );
    // Four guarded reads per rewritten referrer, each a base-revision or
    // publication check on the audited save path: preflight stage,
    // pre-write verify, the check just before the rename, and the
    // post-commit publication read that tells own bytes from an external
    // editor's. None scales with the graph.
    assert!(
        many.full_reads - few.full_reads <= 4 * 28,
        "I-25: each extra referrer costs at most four whole-file reads (stage, verify, \
         pre-rename check, publication)"
    );
}
