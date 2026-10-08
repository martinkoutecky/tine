//! C3 L01 (og c3s F8): a sync-service conflict copy of a page whose own name
//! holds parentheses is reconciled against THAT page, never against a page
//! that merely shares a prefix. The provider suffix is parsed from the END of
//! the stem, and the resolver refuses to merge a copy into any page that is
//! not its own base (scenario: Dropbox/Syncthing/Seafile delivery).

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use tine_core::model::sync_conflict_base;
use tine_graph_features::conflicts;
use tine_store::Store;

fn scratch() -> PathBuf {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "tine-c3s-f8-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    for dir in ["pages", "journals", "assets", "logseq"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    root
}

#[test]
fn every_provider_suffix_is_parsed_from_the_end_of_the_stem() {
    for (stem, base) in [
        (
            "Meeting (draft) (Martin's conflicted copy 2026-09-29)",
            Some("Meeting (draft)"),
        ),
        (
            "Meeting (draft) (conflicted copy 2026-09-29)",
            Some("Meeting (draft)"),
        ),
        (
            "Meeting (draft).sync-conflict-20260929-101010-ABCDEFG",
            Some("Meeting (draft)"),
        ),
        (
            "Meeting (draft) (SFConflict me@example.com 2026-09-29-10-00-00)",
            Some("Meeting (draft)"),
        ),
        ("Report (conflicted copy 2026-08-01)", Some("Report")),
        (
            "Report (Alice's conflicted copy 2026-08-01)",
            Some("Report"),
        ),
        // Real page names that only resemble a Dropbox copy stay real pages.
        ("Meeting (conflicted copy notes) extra", None),
        ("My (draft) page", None),
        ("Notes on (conflicted copy) handling", None),
    ] {
        assert_eq!(sync_conflict_base(stem), base, "stem: {stem:?}");
    }
}

#[test]
fn a_copy_of_a_parenthesised_page_is_listed_against_it_and_never_merged_elsewhere() {
    let root = scratch();
    let copy = "pages/Meeting (draft) (Martin's conflicted copy 2026-09-29).md";
    fs::write(root.join("pages/Meeting.md"), "- unrelated meeting\n").unwrap();
    fs::write(root.join("pages/Meeting (draft).md"), "- draft\n").unwrap();
    fs::write(root.join(copy), "- draft\n- edit from the other device\n").unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;

    let listed = conflicts::list_sync_conflicts(&store).unwrap();
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert_eq!(
        listed[0].base_path.as_deref(),
        Some("pages/Meeting (draft).md"),
        "the copy shadows its own page"
    );

    // A resolve that names a page other than the copy's own base is refused
    // before anything is written or trashed.
    let diff = conflicts::sync_conflict_diff(&store, "pages/Meeting.md", copy, &[])
        .unwrap()
        .unwrap();
    let decisions: HashMap<String, String> = HashMap::new();
    let refused = conflicts::resolve_sync_conflict(
        &store,
        "pages/Meeting.md",
        copy,
        &decisions,
        &diff.base_rev,
        &diff.conflict_rev,
        None,
        &[],
        "union",
    );
    assert!(
        refused.is_err(),
        "merged a copy into a page it does not shadow"
    );
    assert_eq!(
        fs::read_to_string(root.join("pages/Meeting.md")).unwrap(),
        "- unrelated meeting\n"
    );
    assert_eq!(
        fs::read_to_string(root.join(copy)).unwrap(),
        "- draft\n- edit from the other device\n",
        "the copy stays in place for its real reconcile"
    );
    store.close();
    let _ = fs::remove_dir_all(&root);
}
