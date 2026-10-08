//! GH #481 (master ee7730b48): a journal page's Linked References collect a link
//! written in any accepted spelling of its day, and the derived cache stays
//! consistent when such a link is added by a later edit.

use std::path::{Path, PathBuf};
use tine_store::{EditKind, OpenOptions, PageId, Resolved, SaveBase, SaveOutcome, Store};

fn scratch(tag: &str, config: &str, notes: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("tine-481-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for dir in ["journals", "pages", "logseq"] {
        std::fs::create_dir_all(root.join(dir)).unwrap();
    }
    std::fs::write(
        root.join("journals/2026_09_20.md"),
        "- journal day under test\n",
    )
    .unwrap();
    std::fs::write(root.join("pages/Notes.md"), notes).unwrap();
    std::fs::write(root.join("logseq/config.edn"), config).unwrap();
    root
}

fn open(root: &Path) -> Store {
    Store::open(root, OpenOptions::default()).unwrap().0
}

fn referrers(store: &Store, target: &str) -> Vec<String> {
    let mut pages: Vec<String> = store
        .whole_graph()
        .unwrap()
        .backlinks(target)
        .unwrap()
        .iter()
        .map(|group| group.page.clone())
        .collect();
    pages.sort();
    pages
}

fn rewrite_notes(store: &Store, raw: &str) {
    let id: PageId = match store.whole_graph().unwrap().resolve("Notes", false) {
        Resolved::Existing { id, .. } => id,
        _ => panic!("Notes must exist"),
    };
    let mut read = store.page(&id).unwrap();
    read.doc.blocks[0].raw = raw.into();
    assert!(matches!(
        store.save(
            EditKind::ReplacePage,
            &id,
            SaveBase::Existing(read.rev),
            &read.doc
        ),
        SaveOutcome::Saved(_)
    ));
}

#[test]
fn journal_target_collects_referrers_by_default_title() {
    let root = scratch("default", "{}\n", "- plan for [[Sep 20th, 2026]]\n");
    let store = open(&root);
    assert_eq!(referrers(&store, "Sep 20th, 2026"), vec!["Notes"]);
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn iso_title_format_graph_collects_iso_links() {
    let root = scratch(
        "iso",
        "{:journal/page-title-format \"yyyy-MM-dd\"}\n",
        "- plan for [[2026-09-20]]\n",
    );
    let store = open(&root);
    assert_eq!(referrers(&store, "2026-09-20"), vec!["Notes"]);
    std::fs::remove_dir_all(&root).ok();
}

/// The reproduced gap: under the default title format an ISO-spelled link is an
/// accepted spelling of the real journal day.
#[test]
fn iso_link_under_default_title_format_reaches_the_journal_page() {
    let root = scratch("iso-default", "{}\n", "- plan for [[2026-09-20]]\n");
    let store = open(&root);
    assert_eq!(referrers(&store, "Sep 20th, 2026"), vec!["Notes"]);
    assert_eq!(referrers(&store, "2026-09-20"), vec!["Notes"]);
    std::fs::remove_dir_all(&root).ok();
}

/// Scoped invalidation must know the same equivalence: a cached (empty) result
/// for the journal is stale once another page starts linking an accepted spelling.
#[test]
fn an_edit_adding_an_iso_link_evicts_the_cached_journal_backlinks() {
    let root = scratch("iso-evict", "{}\n", "- nothing linked yet\n");
    let store = open(&root);
    assert!(
        referrers(&store, "Sep 20th, 2026").is_empty(),
        "warm the derived cache"
    );
    rewrite_notes(&store, "plan for [[2026-09-20]]");
    assert_eq!(referrers(&store, "Sep 20th, 2026"), vec!["Notes"]);
    std::fs::remove_dir_all(&root).ok();
}
