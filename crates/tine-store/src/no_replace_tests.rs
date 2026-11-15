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
    let root = crate::test_support::fresh_dir(&format!("tine-gh538-{label}-"));
    for dir in ["pages", "journals"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    root
}

/// The flag refusal is a Unix-target fallback (`move_at`): there the move must
/// have met it. Windows' `MoveFileExW` has no flag to refuse, so the leaf hook
/// never fires there and the move is the plain no-replace call.
fn met_the_refusal(before: usize, after: usize, step: &str) {
    #[cfg(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios"
    ))]
    assert!(after > before, "the {step} met the refusal");
    #[cfg(target_os = "windows")]
    assert_eq!(after, before, "the {step} has no flag refusal on Windows");
}

#[test]
fn gh538_host_creates_and_renames_pages_where_the_flag_is_refused() {
    use crate::no_replace::{REFUSED_BY_NAME, REFUSE_NOREPLACE_LEAF};
    use std::sync::atomic::Ordering::SeqCst;
    let root = fixture("rename");
    fs::write(root.join("pages/A.md"), "- original page\n").unwrap();
    let store = std::sync::Arc::new(Store::open(&root, Default::default()).unwrap().0);
    let host_writer = crate::test_host_writer::HostWriter::start(&store);
    let refused_before = REFUSED_BY_NAME.load(SeqCst);
    let moved = format!("{REFUSE_NOREPLACE_LEAF}-moved.md");
    let a = store.file_id(Area::Pages, "A.md").unwrap();
    let b = store.file_id(Area::Pages, &moved).unwrap();
    let rev = store.read(&a, None).unwrap().1;
    let mut tx = store.page_plan(Some(EditKind::RenamePage));
    tx.move_file(&a, rev, &b, None);
    host_writer
        .apply(tx.plan().unwrap())
        .expect("GH #538: a page rename must succeed on flag-refusing storage");
    assert_eq!(
        fs::read_to_string(root.join("pages").join(&moved)).unwrap(),
        "- original page\n"
    );
    assert!(!root.join("pages/A.md").exists());
    let after_rename = REFUSED_BY_NAME.load(SeqCst);
    met_the_refusal(refused_before, after_rename, "rename");

    let new_name = format!("{REFUSE_NOREPLACE_LEAF}-new");
    let created = store
        .as_page(
            &store
                .file_id(Area::Pages, &format!("{new_name}.md"))
                .unwrap(),
        )
        .unwrap();
    let doc = PageDto {
        name: new_name.clone(),
        kind: tine_core::model::PageKind::Page,
        title: new_name.clone(),
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
    host_writer
        .save(EditKind::CreatePage, &created, SaveBase::CreateNew, &doc)
        .expect("GH #538: page creation must succeed on flag-refusing storage");
    assert!(
        fs::read_to_string(root.join("pages").join(format!("{new_name}.md")))
            .unwrap()
            .contains("created")
    );
    met_the_refusal(after_rename, REFUSED_BY_NAME.load(SeqCst), "create");
    drop(host_writer);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn gh538_asset_create_and_move_work_where_the_flag_is_refused() {
    let root = fixture("asset");
    fs::create_dir_all(root.join("assets")).unwrap();
    fs::write(root.join("assets/A.txt"), "asset").unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    let _refused = Refused::on();
    let a = store.file_id(Area::Assets, "A.txt").unwrap();
    let b = store.file_id(Area::Assets, "B.txt").unwrap();
    let rev = store.read(&a, None).unwrap().1;
    let mut tx = store.transaction(None);
    tx.move_file(&a, rev, &b, None);
    assert!(
        matches!(tx.commit(), TxOutcome::Committed { .. }),
        "GH #538: a move must succeed on flag-refusing storage"
    );
    assert_eq!(
        fs::read_to_string(root.join("assets/B.txt")).unwrap(),
        "asset"
    );
    assert!(!root.join("assets/A.txt").exists());

    let created = store.file_id(Area::Assets, "New.txt").unwrap();
    let mut tx = store.transaction(None);
    tx.create(&created, crate::Content::Bytes(b"created".to_vec()));
    assert!(
        matches!(tx.commit(), TxOutcome::Committed { .. }),
        "GH #538: creation must succeed on flag-refusing storage"
    );
    assert_eq!(fs::read(root.join("assets/New.txt")).unwrap(), b"created");
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

/// Pages: a host operation's no-replace publication onto a page that appeared
/// after its guard is `production_tests::create_race_after_guard_is_no_replace_and_then_observes_conflict`.
#[test]
fn gh538_occupied_destination_is_still_refused_where_the_flag_is_refused() {
    let root = fixture("occupied");
    fs::create_dir_all(root.join("assets")).unwrap();
    fs::write(root.join("assets/source.txt"), "source").unwrap();
    fs::write(root.join("assets/dest.txt"), "destination").unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    let src = store.file_id(Area::Assets, "source.txt").unwrap();
    let dest = store.file_id(Area::Assets, "dest.txt").unwrap();
    let rev = store.read(&src, None).unwrap().1;
    let _refused = Refused::on();
    let mut tx = store.transaction(None);
    tx.move_file(&src, rev, &dest, None);
    assert!(matches!(tx.commit(), TxOutcome::NotCommitted { .. }));
    assert_eq!(fs::read(root.join("assets/source.txt")).unwrap(), b"source");
    assert_eq!(
        fs::read(root.join("assets/dest.txt")).unwrap(),
        b"destination"
    );
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

/// Master 678830a086af (GH #538, #590): a failed no-replace move names which
/// rename failed — the flagged call or the plain rename after the flag was
/// refused — so a field report tells the two apart.
#[test]
fn a_failed_move_names_the_rename_that_failed() {
    let root = fixture("step");
    let step = |error: std::io::Error| crate::platform_step::step_of(&error).map(|(op, _)| op);
    let missing = root.join("pages/missing.md");
    let error =
        crate::no_replace::move_file_noreplace(&missing, &root.join("pages/B.md")).unwrap_err();
    #[cfg(any(target_os = "linux", target_os = "android"))]
    assert_eq!(step(error), Some("renameat2(RENAME_NOREPLACE)"));
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    assert_eq!(step(error), Some("renameatx_np(RENAME_EXCL)"));
    // Windows has no flag-refused fallback: MoveFileExW without
    // MOVEFILE_REPLACE_EXISTING is the no-replace move itself.
    #[cfg(target_os = "windows")]
    assert_eq!(step(error), Some("MoveFileExW(MOVEFILE_WRITE_THROUGH)"));
    #[cfg(not(target_os = "windows"))]
    {
        let _refused = Refused::on();
        let error =
            crate::no_replace::move_file_noreplace(&missing, &root.join("pages/B.md")).unwrap_err();
        assert_eq!(
            error.kind(),
            std::io::ErrorKind::NotFound,
            "the kind survives the label"
        );
        assert_eq!(
            step(error),
            Some("renameat after the no-replace flag was refused")
        );
    }
    let _ = fs::remove_dir_all(root);
}
