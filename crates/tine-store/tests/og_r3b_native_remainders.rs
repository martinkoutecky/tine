use std::fs;
use tine_graph_features::pages;
use tine_store::{EditKind, PageId, SaveBase, SaveOutcome, Store};

#[test]
fn hard_linked_pages_save_and_rename_with_master_bytes() {
    for placement in ["annex", "external"] {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("pages")).unwrap();
        let target = root.path().join("pages/Old.md");
        fs::write(&target, "- before\n").unwrap();
        fs::write(root.path().join("pages/Referrer.md"), "- see [[Old]]\n").unwrap();
        let alias = if placement == "annex" {
            let alias = root.path().join(".git/annex/objects/f4/Old.md");
            fs::create_dir_all(alias.parent().unwrap()).unwrap();
            alias
        } else {
            outside.path().join("Old.md")
        };
        fs::hard_link(&target, &alias).unwrap();
        let referrer_alias = outside.path().join("Referrer.md");
        fs::hard_link(root.path().join("pages/Referrer.md"), &referrer_alias).unwrap();
        let store = Store::open(root.path(), Default::default()).unwrap().0;
        let id = PageId::from("pages/Old.md");
        let mut read = store.page(&id).unwrap();
        read.doc.blocks[0].raw = "after".into();
        assert!(
            matches!(
                store.save(
                    EditKind::SaveBlock,
                    &id,
                    SaveBase::Existing(read.rev),
                    &read.doc
                ),
                SaveOutcome::Saved(_)
            ),
            "{placement}"
        );
        // Master fb69f9fcc + 35d51742e: atomic replacement leaves the other
        // link's original bytes; moving the page and rewriting refs both work.
        assert_eq!(fs::read(&target).unwrap(), b"- after\n");
        assert_eq!(fs::read(&alias).unwrap(), b"- before\n");
        pages::rename_page_expected(&store, None, "Old", "New", None).unwrap();
        assert!(!target.exists());
        assert_eq!(
            fs::read(root.path().join("pages/New.md")).unwrap(),
            b"- after\n"
        );
        assert_eq!(
            fs::read(root.path().join("pages/Referrer.md")).unwrap(),
            b"- see [[New]]\n"
        );
        assert_eq!(fs::read(&alias).unwrap(), b"- before\n");
        assert_eq!(fs::read(&referrer_alias).unwrap(), b"- see [[Old]]\n");
        drop(store);
        let reopened = Store::open(root.path(), Default::default()).unwrap().0;
        assert_eq!(
            reopened
                .page(&PageId::from("pages/New.md"))
                .unwrap()
                .doc
                .blocks[0]
                .raw,
            "after"
        );
    }
}

#[test]
fn a_missing_journal_baseline_cannot_overwrite_sync_delivered_bytes() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("journals")).unwrap();
    fs::create_dir(root.path().join("logseq")).unwrap();
    fs::write(
        root.path().join("logseq/config.edn"),
        "{:journal/page-title-format \"yyyy-MM-dd\"}\n",
    )
    .unwrap();
    let target = root.path().join("journals/2026_07_10.md");
    let delivered = b"- Synced custom-format journal\n";
    fs::write(&target, delivered).unwrap();
    let store = Store::open(root.path(), Default::default()).unwrap().0;
    let id = PageId::from("journals/2026_07_10.md");
    let mut template = store.page(&id).unwrap().doc;
    template.name = "Jul 10th, 2026".into();
    template.title = template.name.clone();
    template.blocks[0].raw = "Template body".into();
    let outcome =
        pages::save_page(&store, EditKind::CreatePage, &id, &template, None, false).unwrap();
    assert!(
        !matches!(outcome, SaveOutcome::Saved(_) | SaveOutcome::Unchanged(_)),
        "the claimed overwrite must be refused: {outcome:?}"
    );
    assert_eq!(
        fs::read(target).unwrap(),
        delivered,
        "I-4: no-baseline journal save must retain delivered bytes"
    );
}
