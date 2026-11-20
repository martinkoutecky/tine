//! The journal filename proposal list is exactly what apply performs, and apply
//! renames only the proposals the user confirmed (12a G3 finding 3, Rule 2 B5).
use std::fs;
use std::path::PathBuf;
use tine_graph_features::journals::{self, JournalFilenameMigration};
use tine_store::Store;

fn fixture(label: &str, files: &[&str]) -> (PathBuf, std::sync::Arc<Store>) {
    let root = crate::test_support::fresh_dir(&format!("tine-journal-migrations-{label}-"));
    for dir in ["pages", "journals", "assets", "logseq"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    for name in files {
        fs::write(root.join("journals").join(name), format!("- {name}\n")).unwrap();
    }
    let store = std::sync::Arc::new(Store::open(&root, Default::default()).unwrap().0);
    (root, store)
}

fn proposal(from: &str, to: &str) -> JournalFilenameMigration {
    JournalFilenameMigration {
        from: from.into(),
        to: to.into(),
    }
}

#[test]
fn folded_stem_day_claims_block_duplicate_migrations() {
    let _integration_guard = crate::integration_lock();
    for (format, folded, twin) in [
        ("/yyyyMMdd/", "20261102.md", "2026_11_02.md"),
        ("É yyyy_MM_dd", "é 2026_11_02.md", "É 2026_11_02.org"),
        ("É yyyy_MM_dd", "E\u{301} 2026_11_02.md", "É 2026_11_02.org"),
        ("É yyyy_MM_dd", "e\u{301} 2026_11_02.md", "É 2026_11_02.org"),
    ] {
        let root = tempfile::tempdir().unwrap();
        for dir in ["journals", "logseq"] {
            fs::create_dir(root.path().join(dir)).unwrap();
        }
        fs::write(
            root.path().join("logseq/config.edn"),
            format!("{{:journal/page-title-format \"{format}\"}}\n"),
        )
        .unwrap();
        // In the composed/lowercase case the twins use different extensions
        // so they can coexist on case-insensitive and normalizing volumes.
        fs::write(root.path().join("journals").join(folded), "- folded\n").unwrap();
        fs::write(root.path().join("journals").join(twin), "- twin\n").unwrap();
        let store = Store::open(root.path(), Default::default()).unwrap().0;
        assert_eq!(
            journals::journal_conflicts(&store)
                .unwrap()
                .iter()
                .map(|day| day.files.len())
                .collect::<Vec<_>>(),
            [2],
            "{format}: {folded}"
        );
        assert!(
            journals::journal_filename_migrations(&store)
                .unwrap()
                .is_empty(),
            "{format}: {folded}"
        );
        store.close();
    }
}

#[test]
fn migration_omits_a_free_destination_that_reads_as_another_century() {
    let _integration_guard = crate::integration_lock();
    let root = tempfile::tempdir().unwrap();
    for dir in ["journals", "logseq"] {
        fs::create_dir(root.path().join(dir)).unwrap();
    }
    fs::write(
        root.path().join("logseq/config.edn"),
        "{:journal/file-name-format \"d.M.yy\"}\n",
    )
    .unwrap();
    for name in ["Nov 2nd, 1926.md", "Nov 3rd, 2026.md"] {
        fs::write(root.path().join("journals").join(name), "- entry\n").unwrap();
    }
    let store = std::sync::Arc::new(Store::open(root.path(), Default::default()).unwrap().0);
    assert_eq!(
        journals::journal_filename_migrations(&store).unwrap(),
        [proposal("Nov 3rd, 2026.md", "3.11.26.md")]
    );
    let host = crate::test_host(&store);
    let stale = [proposal("Nov 2nd, 1926.md", "2.11.26.md")];
    let applied = journals::migrate_journal_filenames(&store, &host, &stale).unwrap();
    assert_eq!(applied.migrated, 0);
    assert_eq!(applied.skipped[0].file, "Nov 2nd, 1926.md");
    assert!(!root.path().join("journals/2.11.26.md").exists());
    assert!(root.path().join("journals/Nov 2nd, 1926.md").is_file());
    drop(host);
    store.close();
}

#[test]
fn apply_renames_only_the_confirmed_proposals() {
    let _integration_guard = crate::integration_lock();
    let (root, store) = fixture("confirmed", &["Jun 19th, 2026.md", "Jun 21st, 2026.md"]);
    let host = crate::test_host(&store);
    let listed = journals::journal_filename_migrations(&store).unwrap();
    assert_eq!(
        listed,
        [
            proposal("Jun 19th, 2026.md", "2026_06_19.md"),
            proposal("Jun 21st, 2026.md", "2026_06_21.md"),
        ]
    );
    // The user confirms the list; before apply, sync delivers another
    // title-named journal and removes one listed file.
    fs::write(root.join("journals/Jun 22nd, 2026.md"), "- synced\n").unwrap();
    fs::remove_file(root.join("journals/Jun 21st, 2026.md")).unwrap();
    store.refresh(tine_store::Depth::Stamps).unwrap();
    let result = journals::migrate_journal_filenames(&store, &host, &listed).unwrap();
    assert_eq!(result.migrated, 1);
    assert_eq!(result.skipped.len(), 1, "{:?}", result.skipped);
    assert_eq!(result.skipped[0].file, "Jun 21st, 2026.md");
    assert_eq!(result.skipped[0].reason, "changed since it was listed");
    assert!(root.join("journals/2026_06_19.md").exists());
    assert!(
        root.join("journals/Jun 22nd, 2026.md").exists(),
        "a file the user never confirmed is never renamed"
    );
    assert!(!root.join("journals/2026_06_22.md").exists());
    drop(host);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn listed_proposals_are_exactly_what_apply_performs() {
    let _integration_guard = crate::integration_lock();
    let (root, store) = fixture(
        "exact",
        &[
            // Two titles for one day: neither is proposed (a duplicate day).
            "Jun 18th, 2026.md",
            "Jun 18th, 2026.org",
            // A title whose .md date twin exists.
            "Jun 20th, 2026.org",
            "2026_06_20.md",
            // The one clean proposal.
            "Jun 23rd, 2026.md",
        ],
    );
    let host = crate::test_host(&store);
    let listed = journals::journal_filename_migrations(&store).unwrap();
    assert_eq!(listed, [proposal("Jun 23rd, 2026.md", "2026_06_23.md")]);
    let result = journals::migrate_journal_filenames(&store, &host, &listed).unwrap();
    assert_eq!((result.migrated, result.skipped.len()), (1, 0));
    let stale = [
        proposal("Jun 18th, 2026.md", "2026_06_18.md"),
        proposal("Jun 20th, 2026.org", "2026_06_20.org"),
    ];
    let result = journals::migrate_journal_filenames(&store, &host, &stale).unwrap();
    assert_eq!(result.migrated, 0);
    assert!(
        result
            .skipped
            .iter()
            .all(|skip| skip.reason.contains("same-day")),
        "{:?}",
        result.skipped
    );
    for name in [
        "Jun 18th, 2026.md",
        "Jun 18th, 2026.org",
        "Jun 20th, 2026.org",
    ] {
        assert!(root.join("journals").join(name).exists(), "{name}");
    }
    drop(host);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}
