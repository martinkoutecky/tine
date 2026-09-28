//! GH #538 (master 1739c5109a72, decision B1): storage that refuses the
//! no-replace flag (Android 11-14 shared storage, NFS) still creates, saves and
//! renames through the store, and an occupied destination is still refused
//! with nothing moved. Uses this crate's own `Store` so the `cfg(test)` flag
//! reaches the one no-replace owner.
use crate::no_replace::REFUSE_NOREPLACE_FLAG;
use crate::{Area, EditKind, SaveBase, Store, TxOutcome};
use std::fs;
use tine_core::model::PageDto;

struct Refused;
impl Refused {
    fn on() -> Self {
        REFUSE_NOREPLACE_FLAG.with(|flag| flag.set(true));
        Refused
    }
}
impl Drop for Refused {
    fn drop(&mut self) {
        REFUSE_NOREPLACE_FLAG.with(|flag| flag.set(false));
    }
}

fn fixture(label: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("tine-gh538-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    for dir in ["pages", "journals"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    root
}

#[test]
fn gh538_create_save_and_rename_work_where_the_flag_is_refused() {
    let root = fixture("rename");
    fs::write(root.join("pages/A.md"), "- original page\n").unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    let _refused = Refused::on();
    let a = store.file_id(Area::Pages, "A.md").unwrap();
    let b = store.file_id(Area::Pages, "B.md").unwrap();
    let rev = store.read(&a, None).unwrap().1;
    let mut tx = store.transaction(Some(EditKind::RenamePage));
    tx.move_file(&a, rev, &b, None);
    assert!(
        matches!(tx.commit(), TxOutcome::Committed { .. }),
        "GH #538: a page rename must succeed on flag-refusing storage"
    );
    assert_eq!(
        fs::read_to_string(root.join("pages/B.md")).unwrap(),
        "- original page\n"
    );
    assert!(!root.join("pages/A.md").exists());

    let created = store
        .as_page(&store.file_id(Area::Pages, "New.md").unwrap())
        .unwrap();
    let doc = PageDto {
        name: "New".into(),
        kind: tine_core::model::PageKind::Page,
        title: "New".into(),
        pre_block: None,
        format: tine_core::model::Format::Md,
        blocks: vec![tine_core::model::BlockDto {
            id: "gh538-block".into(),
            raw: "created".into(),
            ..Default::default()
        }],
        rev: None,
        read_only: false,
        guide: false,
    };
    let mut tx = store.transaction(Some(EditKind::CreatePage));
    tx.save_page(&[EditKind::CreatePage], &created, SaveBase::CreateNew, &doc);
    assert!(
        matches!(tx.commit(), TxOutcome::Committed { .. }),
        "GH #538: page creation must succeed on flag-refusing storage"
    );
    assert!(fs::read_to_string(root.join("pages/New.md"))
        .unwrap()
        .contains("created"));
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn gh538_occupied_destination_is_still_refused_where_the_flag_is_refused() {
    let root = fixture("occupied");
    fs::write(root.join("pages/source.md"), "source").unwrap();
    fs::write(root.join("pages/dest.md"), "destination").unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    let src = store.file_id(Area::Pages, "source.md").unwrap();
    let dest = store.file_id(Area::Pages, "dest.md").unwrap();
    let rev = store.read(&src, None).unwrap().1;
    let _refused = Refused::on();
    let mut tx = store.transaction(Some(EditKind::ReplacePage));
    tx.move_file(&src, rev, &dest, None);
    assert!(matches!(tx.commit(), TxOutcome::NotCommitted { .. }));
    assert_eq!(fs::read(root.join("pages/source.md")).unwrap(), b"source");
    assert_eq!(
        fs::read(root.join("pages/dest.md")).unwrap(),
        b"destination"
    );
    drop(store);
    fs::remove_dir_all(root).unwrap();
}
