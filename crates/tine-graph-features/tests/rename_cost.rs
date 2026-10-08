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
    let _case = CASE_LOCK.lock().unwrap_or_else(|error| error.into_inner());
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
    // Three guarded reads per rewritten referrer, each a base-revision or
    // publication check on the audited save path: preflight stage, the check
    // just before the rename, and the post-commit publication read that tells
    // own bytes from an external editor's (GH #623 QF3b removed a stage-2
    // verify that repeated the pre-rename check). None scales with the graph.
    assert!(
        many.full_reads - few.full_reads <= 3 * 28,
        "I-25: each extra referrer costs at most three whole-file reads (stage, \
         pre-rename check, publication)"
    );
}

#[test]
fn rename_publication_work_is_linear() {
    let _case = CASE_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    for referrers in [8, 64] {
        let counts = rename(80, referrers);
        assert!(counts.transaction_record_probes <= 6 * (referrers as u64 + 2),
            "GH #623/I-25: final publication must inspect transaction records O(touched files), not O(referrers²); exemplar transaction/publication.rs: {counts:?}");
    }
}

#[test]
fn rename_publication_parses_only_changed_documents() {
    let _case = CASE_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    for referrers in [8, 64] {
        let counts = rename(80, referrers);
        assert!(counts.parses <= referrers as u64 + 2,
            "GH #623/I-25: own reference rewrites must parse the new document once, without serializing/parsing the old one; exemplar model/transaction_publish.rs: {counts:?}");
    }
}

#[test]
fn rename_unit_cost_is_the_same_full_payload_on_one_and_sixty_blocks() {
    let _case = CASE_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    for (blocks, bytes) in [(1, 22), (60, 1320)] {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("pages")).unwrap();
        fs::write(root.path().join("pages/Target.md"), "- target\n").unwrap();
        fs::write(
            root.path().join("pages/Ref.md"),
            "- links [[Target]] 0\n".repeat(blocks),
        )
        .unwrap();
        let store = Store::open(root.path(), Default::default()).unwrap().0;
        store.whole_graph().unwrap();
        cost_counters::reset();
        pages::rename_page_expected(&store, "Target", "Renamed", None).unwrap();
        let cost = cost_counters::snapshot();
        assert_eq!(cost.files_written, 1);
        assert_eq!(cost.bytes_written, bytes);
        assert_eq!(
            cost.fsyncs, 3,
            "one temp sync + one referrer directory sync + source move directory sync"
        );
        assert_eq!(
            fs::read_to_string(root.path().join("pages/Ref.md")).unwrap(),
            "- links [[Renamed]] 0\n".repeat(blocks)
        );
        // The documented unit-cost values are pinned to this actual write path.
        let contract = include_str!("../../../docs/storage-contract.md");
        assert!(contract.contains("22/1,320 bytes"));
        store.close();
    }
}
