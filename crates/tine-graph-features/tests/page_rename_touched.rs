//! GH #535 (master 861cfb435d89): a rename reports exactly the page files it
//! moved, trashed or rewrote, and refuses to write a file whose in-memory
//! edits the caller could not save. Pages it does not touch need not be saved.
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use tine_graph_features::pages::{self, RenameOutcome, TouchedPage};
use tine_store::Store;

fn fixture(label: &str) -> (PathBuf, Store) {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "tine-rename-touched-{label}-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    for dir in ["pages", "journals", "assets", "logseq"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    for (rel, body) in [
        ("logseq/config.edn", "{:file/name-format :triple-lowbar}\n"),
        ("pages/Target.md", "- the target\n"),
        ("pages/Target___Child.md", "- child\n"),
        ("pages/Referrer.md", "- links [[Target]]\n"),
        ("journals/2026_09_21.md", "- met [[Target/Child]]\n"),
        ("pages/Unrelated.md", "- nothing to see\n"),
    ] {
        fs::write(root.join(rel), body).unwrap();
    }
    let store = Store::open(&root, Default::default()).unwrap().0;
    (root, store)
}

fn snapshot(root: &Path) -> Vec<(PathBuf, String)> {
    let mut files = Vec::new();
    for dir in ["pages", "journals"] {
        for entry in fs::read_dir(root.join(dir)).unwrap().flatten() {
            files.push((entry.path(), fs::read_to_string(entry.path()).unwrap()));
        }
    }
    files.sort();
    files
}

fn touched(path: &str, moved: bool) -> TouchedPage {
    TouchedPage {
        path: path.into(),
        moved,
    }
}

#[test]
fn a_rename_reports_every_page_it_moved_or_rewrote_and_nothing_else() {
    let (root, store) = fixture("report");
    let mut report =
        pages::rename_or_merge_page(&store, None, "Target", "Renamed", None, None, &[]).unwrap();
    assert_eq!(report.outcome, RenameOutcome::Renamed);
    report.touched.sort_by(|a, b| a.path.cmp(&b.path));
    assert_eq!(
        report.touched,
        vec![
            touched("journals/2026_09_21.md", false),
            touched("pages/Referrer.md", false),
            touched("pages/Target.md", true),
            touched("pages/Target___Child.md", true),
        ]
    );
    assert_eq!(
        fs::read_to_string(root.join("pages/Unrelated.md")).unwrap(),
        "- nothing to see\n"
    );
}

#[test]
fn a_rename_refuses_to_write_a_file_with_unsaved_edits_and_changes_nothing() {
    for blocked in ["pages/Referrer.md", "pages/Target___Child.md"] {
        let (root, store) = fixture("guarded");
        let before = snapshot(&root);
        let error = pages::rename_or_merge_page(
            &store,
            None,
            "Target",
            "Renamed",
            None,
            None,
            &[blocked.to_owned()],
        )
        .expect_err("writing a file under unsaved edits must refuse");
        let name = if blocked.contains("Child") {
            "“Target/Child”"
        } else {
            "“Referrer”"
        };
        assert!(error.to_string().contains(name), "{error}");
        assert_eq!(snapshot(&root), before, "{blocked}");
    }
}

#[test]
fn unsaved_edits_on_an_untouched_page_do_not_block_the_rename() {
    let (root, store) = fixture("untouched");
    let report = pages::rename_or_merge_page(
        &store,
        None,
        "Target",
        "Renamed",
        None,
        None,
        &["pages/Unrelated.md".to_owned()],
    )
    .unwrap();
    assert_eq!(report.outcome, RenameOutcome::Renamed);
    assert_eq!(
        fs::read_to_string(root.join("pages/Referrer.md")).unwrap(),
        "- links [[Renamed]]\n"
    );
}

#[test]
fn a_merge_reports_its_survivor_and_source_and_honours_unsaved_paths() {
    let (root, store) = fixture("merge");
    fs::write(root.join("pages/Survivor.md"), "- kept\n").unwrap();
    let store = {
        drop(store);
        Store::open(&root, Default::default()).unwrap().0
    };
    let before = snapshot(&root);
    let error = pages::rename_or_merge_page(
        &store,
        None,
        "Referrer",
        "Survivor",
        None,
        Some("pages/Survivor.md"),
        &["pages/Survivor.md".to_owned()],
    )
    .expect_err("the survivor holds unsaved edits");
    assert!(error.to_string().contains("“Survivor”"), "{error}");
    assert_eq!(snapshot(&root), before);

    let mut report = pages::rename_or_merge_page(
        &store,
        None,
        "Referrer",
        "Survivor",
        None,
        Some("pages/Survivor.md"),
        &[],
    )
    .unwrap();
    assert_eq!(report.outcome, RenameOutcome::Merged);
    report.touched.sort_by(|a, b| a.path.cmp(&b.path));
    assert_eq!(
        report.touched,
        vec![
            touched("pages/Referrer.md", true),
            touched("pages/Survivor.md", false),
        ]
    );
}
