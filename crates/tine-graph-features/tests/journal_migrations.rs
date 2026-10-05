//! The journal filename proposal list is exactly what apply performs, and apply
//! renames only the proposals the user confirmed (12a G3 finding 3, Rule 2 B5).
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use tine_graph_features::journals::{self, JournalFilenameMigration};
use tine_store::Store;

fn fixture(label: &str, files: &[&str]) -> (PathBuf, Store) {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "tine-journal-migrations-{label}-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    for dir in ["pages", "journals", "assets", "logseq"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    for name in files {
        fs::write(root.join("journals").join(name), format!("- {name}\n")).unwrap();
    }
    let store = Store::open(&root, Default::default()).unwrap().0;
    (root, store)
}

fn proposal(from: &str, to: &str) -> JournalFilenameMigration {
    JournalFilenameMigration {
        from: from.into(),
        to: to.into(),
    }
}

#[test]
fn apply_renames_only_the_confirmed_proposals() {
    let (root, store) = fixture("confirmed", &["Jun 19th, 2026.md", "Jun 21st, 2026.md"]);
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
    let result = journals::migrate_journal_filenames(&store, &listed).unwrap();
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
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn listed_proposals_are_exactly_what_apply_performs() {
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
    let listed = journals::journal_filename_migrations(&store).unwrap();
    assert_eq!(listed, [proposal("Jun 23rd, 2026.md", "2026_06_23.md")]);
    let result = journals::migrate_journal_filenames(&store, &listed).unwrap();
    assert_eq!((result.migrated, result.skipped.len()), (1, 0));
    let stale = [
        proposal("Jun 18th, 2026.md", "2026_06_18.md"),
        proposal("Jun 20th, 2026.org", "2026_06_20.org"),
    ];
    let result = journals::migrate_journal_filenames(&store, &stale).unwrap();
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
    drop(store);
    fs::remove_dir_all(root).unwrap();
}
